//! Information about the SIMD implementation Vectorscan uses on this host, and
//! about which compiled databases it can load.

use std::ffi::CStr;
use std::fmt;
use vectorscan_rs_sys as ffi;

/// Returns the name of the SIMD implementation that runtime calls (scanning,
/// stream operations, deserialization) dispatch to on this host, e.g.
/// `"avx512vbmi"`, `"avx2"`, `"corei7"`, `"sve2"` or `"neon"`.
///
/// With the `fat_runtime` feature this runs the same CPU feature checks as the
/// runtime dispatcher, so it is accurate before anything is compiled or
/// scanned.
pub fn runtime_variant() -> &'static str {
    // SAFETY: hs_runtime_variant returns a pointer to a static NUL-terminated string.
    unsafe { CStr::from_ptr(ffi::hs_runtime_variant()) }
        .to_str()
        .unwrap_or("unknown")
}

/// The optional CPU features a compiled database requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DbFeatures {
    Generic,
    Avx2,
    Avx512,
    Avx512Vbmi,
    Sve,
    Sve2,
}

impl DbFeatures {
    pub fn as_str(self) -> &'static str {
        match self {
            DbFeatures::Generic => "generic",
            DbFeatures::Avx2 => "avx2",
            DbFeatures::Avx512 => "avx512",
            DbFeatures::Avx512Vbmi => "avx512vbmi",
            DbFeatures::Sve => "sve",
            DbFeatures::Sve2 => "sve2",
        }
    }

    /// Parses the feature names used by `hs_db_target` and `hs_database_info`.
    fn from_hs_name(name: &str) -> Option<Self> {
        Some(match name {
            "" => DbFeatures::Generic,
            "AVX2" => DbFeatures::Avx2,
            "AVX512" => DbFeatures::Avx512,
            "AVX512VBMI" => DbFeatures::Avx512Vbmi,
            "SVE" => DbFeatures::Sve,
            "SVE2" => DbFeatures::Sve2,
            _ => return None,
        })
    }
}

/// Identifies the kind of database that compiling on this host produces.
///
/// A serialized database only loads on a host with the same architecture whose
/// runtime has every feature the database requires; otherwise deserialization
/// fails with [`HyperscanErrorCode::DbPlatformError`](crate::HyperscanErrorCode::DbPlatformError).
/// When used as a key for precompiled databases, it must be combined with the
/// Vectorscan version and the exact build of this crate: the header's version
/// check does not change when only the patches applied here do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DbTarget {
    /// Target architecture, as in [`std::env::consts::ARCH`].
    pub arch: &'static str,
    pub features: DbFeatures,
}

/// Formats as `<arch>-<features>`, e.g. `x86_64-avx512vbmi` or `aarch64-generic`.
impl fmt::Display for DbTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.arch, self.features.as_str())
    }
}

/// Returns the [`DbTarget`] of databases compiled on this host (i.e. without an
/// explicit platform), or `None` if the library reports an unrecognized
/// feature set.
pub fn db_target() -> Option<DbTarget> {
    // SAFETY: a null platform selects the current host; the result is a
    // pointer to a static NUL-terminated string.
    let name = unsafe { CStr::from_ptr(ffi::hs_db_target(std::ptr::null())) };
    let features = DbFeatures::from_hs_name(name.to_str().ok()?)?;
    Some(DbTarget {
        arch: std::env::consts::ARCH,
        features,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrapper::{Database, ScanMode};
    use crate::{Error, Flag, HyperscanErrorCode, Pattern};

    // Offset and bits of the `platform` field in the serialized database header
    // (see `struct hs_database` and `HS_PLATFORM_*` in database.h).
    const PLATFORM_OFFSET: usize = 12;
    const CPU_MASK: u64 = 0x3f;
    const NOAVX2: u64 = 4 << 13;
    const NOSVE2: u64 = 0x40 << 13;

    fn serialized_hello() -> Vec<u8> {
        let patterns = vec![Pattern::new(b"hello".to_vec(), Flag::default(), None)];
        let db = Database::new(patterns, ScanMode::BLOCK).unwrap();
        db.serialize().unwrap().as_bytes().to_vec()
    }

    fn platform(bytes: &[u8]) -> u64 {
        u64::from_le_bytes(bytes[PLATFORM_OFFSET..PLATFORM_OFFSET + 8].try_into().unwrap())
    }

    fn set_platform(bytes: &mut [u8], p: u64) {
        bytes[PLATFORM_OFFSET..PLATFORM_OFFSET + 8].copy_from_slice(&p.to_le_bytes());
    }

    fn assert_platform_error(bytes: &[u8]) {
        match Database::try_from(bytes) {
            Err(Error::Hyperscan(HyperscanErrorCode::DbPlatformError, _)) => {}
            Err(e) => panic!("expected DbPlatformError, got {e:?}"),
            Ok(_) => panic!("expected DbPlatformError, got a database"),
        }
    }

    #[test]
    fn runtime_variant_matches_db_target() {
        let variant = runtime_variant();
        let target = db_target().expect("known db target");
        assert_eq!(target.arch, std::env::consts::ARCH);

        #[cfg(target_arch = "x86_64")]
        {
            let expected = match variant {
                "avx512vbmi" => DbFeatures::Avx512Vbmi,
                "avx512" => DbFeatures::Avx512,
                "avx2" => DbFeatures::Avx2,
                "corei7" | "simde" => DbFeatures::Generic,
                other => panic!("unexpected x86_64 runtime variant {other}"),
            };
            assert_eq!(target.features, expected);
        }

        #[cfg(target_arch = "aarch64")]
        {
            assert!(matches!(variant, "sve2" | "sve" | "neon"), "unexpected variant {variant}");
            // Only a compiler built for SVE emits SVE bytecode; the runtime
            // variant must then be able to run it.
            match target.features {
                DbFeatures::Generic => {}
                DbFeatures::Sve => assert!(matches!(variant, "sve" | "sve2")),
                DbFeatures::Sve2 => assert_eq!(variant, "sve2"),
                other => panic!("unexpected aarch64 db features {other:?}"),
            }
        }
    }

    #[test]
    fn database_info_reports_arch() {
        let bytes = serialized_hello();
        let mut info = std::ptr::null_mut();
        unsafe {
            assert_eq!(
                ffi::hs_serialized_database_info(bytes.as_ptr().cast(), bytes.len(), &mut info),
                ffi::HS_SUCCESS as ffi::hs_error_t
            );
            let s = CStr::from_ptr(info).to_str().unwrap().to_owned();
            libc::free(info.cast());

            let expected_arch = if cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
                "Arch: x86"
            } else if cfg!(any(target_arch = "arm", target_arch = "aarch64")) {
                "Arch: arm"
            } else {
                "Arch: ppc64el"
            };
            assert!(s.contains(expected_arch), "{s}");
        }
    }

    #[test]
    fn serialized_database_round_trips() {
        let bytes = serialized_hello();
        Database::try_from(&bytes[..]).expect("own database must load");
    }

    #[test]
    fn rejects_database_for_other_arch() {
        let mut bytes = serialized_hello();
        let p = platform(&bytes);
        let arch = p & CPU_MASK;
        assert_ne!(arch, 0, "database header must record its arch");

        // Databases from before the arch was recorded have arch 0.
        for other in [0, if arch == 1 { 2 } else { 1 }] {
            set_platform(&mut bytes, (p & !CPU_MASK) | other);
            assert_platform_error(&bytes);
        }
    }

    #[test]
    fn rejects_database_requiring_missing_feature() {
        let mut bytes = serialized_hello();
        let p = platform(&bytes);
        // A feature of the other architecture is never available.
        let missing = if cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
            NOSVE2
        } else {
            NOAVX2
        };
        assert_ne!(p & missing, 0);
        set_platform(&mut bytes, p & !missing);
        assert_platform_error(&bytes);
    }

    #[test]
    fn rejects_unknown_platform_bits() {
        let mut bytes = serialized_hello();
        let p = platform(&bytes);
        set_platform(&mut bytes, p | (1 << 40));
        assert_platform_error(&bytes);
    }

    #[test]
    fn accepts_database_requiring_fewer_features() {
        let mut bytes = serialized_hello();
        let p = platform(&bytes);
        // Marking more features as not required keeps the database loadable.
        set_platform(&mut bytes, p | NOAVX2 | NOSVE2 | (0x18 << 13));
        Database::try_from(&bytes[..]).expect("database requiring fewer features must load");
    }
}
