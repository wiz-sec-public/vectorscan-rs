//! Information about the SIMD implementation Vectorscan uses on this host, and
//! about which compiled databases it can produce and load.
//!
//! Two independent things are involved:
//!
//! - The *runtime variant* ([`runtime_variant`]) is the SIMD code that scanning
//!   and deserialization dispatch to. The fat runtime picks it per process from
//!   the host CPU. It decides which databases load ([`loadable_targets`]).
//! - The *database target* ([`DbTarget`]) is the architecture plus the CPU
//!   features compiled bytecode requires, recorded in the database header. The
//!   compiler decides it: [`db_target`] is what it produces by default here,
//!   [`compilable_targets`] everything it can be asked for.
//!
//! A database loads where the architecture matches and the runtime variant has
//! every feature the database requires. On x86 the default target follows the
//! runtime variant; on AArch64 fat runtime builds always compile generic
//! databases, even where the runtime variant is SVE2.

use std::ffi::CStr;
use std::fmt;
use vectorscan_rs_sys as ffi;

/// Returns the name of the SIMD implementation that runtime calls (scanning,
/// stream operations, deserialization) dispatch to on this host, e.g.
/// `"avx512vbmi"`, `"avx2"`, `"corei7"`, `"sve2"`, `"neon"` or `"vsx"`.
/// Reports `"none"` if the fat runtime has no implementation for this CPU, and
/// `"generic"` for single-implementation builds on other architectures.
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

    /// Feature levels a database may require on `arch` (as in
    /// [`std::env::consts::ARCH`]), best first. A runtime that has a level
    /// also has every level after it.
    pub fn levels(arch: &str) -> &'static [DbFeatures] {
        match arch {
            "x86_64" => &[
                DbFeatures::Avx512Vbmi,
                DbFeatures::Avx512,
                DbFeatures::Avx2,
                DbFeatures::Generic,
            ],
            "aarch64" => &[DbFeatures::Sve2, DbFeatures::Sve, DbFeatures::Generic],
            _ => &[DbFeatures::Generic],
        }
    }

    /// `HS_CPU_FEATURES_*` bits that request this level when compiling. Each
    /// level includes the levels below it, so the header stays consistent.
    /// SVE levels can't be requested and map to 0 (see [`compilable_targets`]).
    pub fn cpu_features(self) -> u64 {
        let bits = match self {
            DbFeatures::Generic | DbFeatures::Sve | DbFeatures::Sve2 => 0,
            DbFeatures::Avx2 => ffi::HS_CPU_FEATURES_AVX2,
            DbFeatures::Avx512 => ffi::HS_CPU_FEATURES_AVX2 | ffi::HS_CPU_FEATURES_AVX512,
            DbFeatures::Avx512Vbmi => {
                ffi::HS_CPU_FEATURES_AVX2
                    | ffi::HS_CPU_FEATURES_AVX512
                    | ffi::HS_CPU_FEATURES_AVX512VBMI
            }
        };
        bits.into()
    }

    /// Name used by `hs_db_target` and `hs_database_info`.
    fn hs_name(self) -> &'static str {
        match self {
            DbFeatures::Generic => "",
            DbFeatures::Avx2 => "AVX2",
            DbFeatures::Avx512 => "AVX512",
            DbFeatures::Avx512Vbmi => "AVX512VBMI",
            DbFeatures::Sve => "SVE",
            DbFeatures::Sve2 => "SVE2",
        }
    }

    /// Features a runtime variant has, as named by [`runtime_variant`], or
    /// `None` for `"none"` (no usable implementation).
    fn from_runtime_variant(variant: &str) -> Option<Self> {
        Some(match variant {
            "avx512vbmi" => DbFeatures::Avx512Vbmi,
            "avx512" => DbFeatures::Avx512,
            "avx2" => DbFeatures::Avx2,
            "sve2" => DbFeatures::Sve2,
            "sve" => DbFeatures::Sve,
            "corei7" | "simde" | "neon" | "vsx" | "generic" => DbFeatures::Generic,
            _ => return None,
        })
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
///
/// On x86 the features follow the host CPU, limited to what this build
/// supports. On AArch64 they depend on how this crate was built rather than on
/// the CPU: SVE/SVE2 are only required when the compiler itself was built with
/// them (`simd_specialization` or `cpu_native` without `fat_runtime`), so fat
/// runtime builds always report [`DbFeatures::Generic`] there, even though SVE
/// hosts still scan with SVE code. The SVE vector length is not part of the
/// target, although an SVE-enabled compiler emits some engines only on hosts
/// with vectors of at least 256 bits.
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

impl DbTarget {
    /// Platform to pass to the compiler to produce databases of this target.
    pub(crate) fn platform_info(self) -> ffi::hs_platform_info_t {
        ffi::hs_platform_info_t {
            tune: ffi::HS_TUNE_FAMILY_GENERIC,
            cpu_features: self.features.cpu_features(),
            reserved1: 0,
            reserved2: 0,
        }
    }

    /// Whether compiling with [`Self::platform_info`] produces databases of
    /// exactly this target on this host.
    pub(crate) fn is_compilable(self) -> bool {
        if self.arch != std::env::consts::ARCH {
            return false;
        }
        let platform = self.platform_info();
        // SAFETY: hs_db_target returns a pointer to a static NUL-terminated string.
        let name = unsafe { CStr::from_ptr(ffi::hs_db_target(&platform)) };
        name.to_bytes() == self.features.hs_name().as_bytes()
    }
}

/// Returns the [`DbTarget`] of databases compiled on this host by default
/// (i.e. without an explicit platform), or `None` if the library reports an
/// unrecognized feature set.
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

/// Targets of databases the runtime variant on this host can deserialize and
/// scan with, best first: the variant's own level, then every level below it.
/// Empty if there is no usable runtime variant.
pub fn loadable_targets() -> Vec<DbTarget> {
    let arch = std::env::consts::ARCH;
    let Some(features) = DbFeatures::from_runtime_variant(runtime_variant()) else {
        return Vec::new();
    };
    let levels = DbFeatures::levels(arch);
    let levels = match levels.iter().position(|f| *f == features) {
        Some(i) => &levels[i..],
        None => std::slice::from_ref(&DbFeatures::Generic),
    };
    levels
        .iter()
        .map(|&features| DbTarget { arch, features })
        .collect()
}

/// Targets this build can compile databases for on this host, best first
/// (see [`BlockDatabase::new_for_target`](crate::BlockDatabase::new_for_target)).
/// The architecture is always the host's. On x86 every AVX level the build
/// supports is compilable regardless of the host CPU; SVE levels only with a
/// compiler built for SVE (never in fat runtime builds). Compilable databases
/// may not be loadable here.
pub fn compilable_targets() -> Vec<DbTarget> {
    let arch = std::env::consts::ARCH;
    DbFeatures::levels(arch)
        .iter()
        .map(|&features| DbTarget { arch, features })
        .filter(|t| t.is_compilable())
        .collect()
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
        u64::from_le_bytes(
            bytes[PLATFORM_OFFSET..PLATFORM_OFFSET + 8]
                .try_into()
                .unwrap(),
        )
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
            assert!(
                matches!(variant, "sve2" | "sve" | "neon"),
                "unexpected variant {variant}"
            );
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

    fn serialize_block(db: &crate::BlockDatabase) -> Vec<u8> {
        let mut bytes = std::ptr::null_mut();
        let mut len = 0;
        unsafe {
            assert_eq!(
                ffi::hs_serialize_database(db.as_ptr(), &mut bytes, &mut len),
                ffi::HS_SUCCESS as ffi::hs_error_t
            );
            let v = std::slice::from_raw_parts(bytes.cast::<u8>(), len).to_vec();
            libc::free(bytes.cast());
            v
        }
    }

    fn database_info(bytes: &[u8]) -> String {
        let mut info = std::ptr::null_mut();
        unsafe {
            assert_eq!(
                ffi::hs_serialized_database_info(bytes.as_ptr().cast(), bytes.len(), &mut info),
                ffi::HS_SUCCESS as ffi::hs_error_t
            );
            let s = CStr::from_ptr(info).to_str().unwrap().to_owned();
            libc::free(info.cast());
            s
        }
    }

    fn hello() -> Vec<Pattern> {
        vec![Pattern::new(b"hello".to_vec(), Flag::default(), None)]
    }

    #[test]
    fn loadable_targets_follow_runtime_variant() {
        let loadable = loadable_targets();
        let own = DbFeatures::from_runtime_variant(runtime_variant()).unwrap();
        assert_eq!(loadable.first().unwrap().features, own);
        assert_eq!(loadable.last().unwrap().features, DbFeatures::Generic);
        assert!(loadable.iter().all(|t| t.arch == std::env::consts::ARCH));
        // Databases compiled here by default must load here.
        assert!(loadable.contains(&db_target().unwrap()));
    }

    #[test]
    fn compilable_targets_include_default() {
        let compilable = compilable_targets();
        assert!(compilable.contains(&db_target().unwrap()));
        assert_eq!(compilable.last().unwrap().features, DbFeatures::Generic);
        #[cfg(all(target_arch = "aarch64", feature = "fat_runtime"))]
        assert_eq!(compilable.len(), 1);
    }

    #[test]
    fn new_for_target_records_target_in_header() {
        let loadable = loadable_targets();
        for target in compilable_targets() {
            let db = crate::BlockDatabase::new_for_target(hello(), target).unwrap();
            let bytes = serialize_block(&db);
            let info = database_info(&bytes);
            let features = format!("Features: {} Mode", target.features.hs_name());
            assert!(info.contains(&features), "{target}: {info}");
            if loadable.contains(&target) {
                Database::try_from(&bytes[..]).expect("loadable target must load");
            } else {
                assert_platform_error(&bytes);
            }
        }
    }

    #[test]
    fn new_for_target_rejects_uncompilable() {
        let other_arch = DbTarget {
            arch: if cfg!(target_arch = "x86_64") {
                "aarch64"
            } else {
                "x86_64"
            },
            features: DbFeatures::Generic,
        };
        let other_features = DbTarget {
            arch: std::env::consts::ARCH,
            features: if cfg!(target_arch = "x86_64") {
                DbFeatures::Sve2
            } else {
                DbFeatures::Avx2
            },
        };
        for target in [other_arch, other_features] {
            assert!(!compilable_targets().contains(&target));
            match crate::BlockDatabase::new_for_target(hello(), target) {
                Err(Error::Hyperscan(HyperscanErrorCode::Invalid, _)) => {}
                Err(e) => panic!("{target}: expected Invalid, got {e:?}"),
                Ok(_) => panic!("{target}: expected Invalid, got a database"),
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
