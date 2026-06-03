use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("`{}` should be set in the environment", name))
}

fn rename_library(dst: &Path) {
    for lib_folder in &[dst.join("lib"), dst.join("lib64")] {
        let src = lib_folder.join("libhs.a");
        let dest = lib_folder.join("libvs.a");
        if src.exists() {
            fs::rename(&src, &dest).unwrap_or_else(|e| {
                panic!("Failed to rename {:?} to {:?}: {}", src, dest, e)
            });
        }
    }
}

fn copy_dir_all(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap_or_else(|e| {
        panic!("Failed to create directory {}: {e}", dst.display())
    });
    for entry in fs::read_dir(src).unwrap_or_else(|e| {
        panic!("Failed to read directory {}: {e}", src.display())
    }) {
        let entry = entry.expect("Failed to read directory entry");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry.file_type().expect("Failed to get file type").is_dir() {
            copy_dir_all(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).unwrap_or_else(|e| {
                panic!("Failed to copy {} -> {}: {e}", src_path.display(), dst_path.display());
            });
        }
    }
}

/// Try to resolve the submodule's git HEAD file for Cargo change detection.
/// For a git submodule, `<submodule>/.git` is a file containing `gitdir: <path>`.
fn resolve_submodule_head(submodule_dir: &Path) -> Option<PathBuf> {
    let dot_git = submodule_dir.join(".git");
    let content = fs::read_to_string(&dot_git).ok()?;
    let gitdir_ref = content.strip_prefix("gitdir: ")?.trim();
    let resolved = submodule_dir.join(gitdir_ref);
    let head = resolved.join("HEAD");
    head.exists().then_some(head)
}

fn build_vectorscan(manifest_dir: &Path, out_dir: &Path) {
    let include_dir = out_dir
        .join("include")
        .into_os_string()
        .into_string()
        .unwrap();

    let submodule_dir = manifest_dir.join("vectorscan");
    let vectorscan_src_dir = out_dir.join("vectorscan-src");

    assert!(
        submodule_dir.join("CMakeLists.txt").exists(),
        "Vectorscan submodule not found at {}. Run: git submodule update --init",
        submodule_dir.display()
    );

    match fs::remove_dir_all(&vectorscan_src_dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("Failed to clean vectorscan source directory: {e}"),
    }
    copy_dir_all(&submodule_dir, &vectorscan_src_dir);

    let patches_dir = manifest_dir.join("patches");
    if patches_dir.is_dir() {
        let mut patches: Vec<_> = fs::read_dir(&patches_dir)
            .expect("Failed to read patches directory")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "patch"))
            .map(|e| e.path())
            .collect();
        patches.sort();
        for patch_path in &patches {
            let patchfile = File::open(patch_path)
                .unwrap_or_else(|e| panic!("Failed to open {}: {e}", patch_path.display()));
            let output = Command::new("patch")
                .args(["-p1", "--forward"])
                .current_dir(&vectorscan_src_dir)
                .stdin(patchfile)
                .output()
                .unwrap_or_else(|e| panic!("Failed to run patch for {}: {e}", patch_path.display()));
            assert!(
                output.status.success(),
                "Failed to apply {}:\nstdout: {}\nstderr: {}",
                patch_path.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            eprintln!("Applied {}", patch_path.file_name().unwrap().to_string_lossy());
        }
    }
    eprintln!("Vectorscan source prepared at {}", vectorscan_src_dir.display());

    let mut cfg = cmake::Config::new(&vectorscan_src_dir);
    cfg.out_dir(out_dir);

    macro_rules! cfg_define_feature {
        ($cmake_feature: tt, $cargo_feature: tt) => {
            cfg.define(
                $cmake_feature,
                if cfg!(feature = $cargo_feature) {
                    "ON"
                } else {
                    "OFF"
                },
            )
        };
    }

    cfg.profile("Release")
        .define("CMAKE_INSTALL_INCLUDEDIR", &include_dir)
        .define("CMAKE_VERBOSE_MAKEFILE", "ON")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("BUILD_STATIC_LIBS", "ON")
        .define("BUILD_EXAMPLES", "OFF")
        .define("BUILD_BENCHMARKS", "OFF")
        .define("BUILD_DOC", "OFF");

    cfg_define_feature!("BUILD_UNIT", "unit_hyperscan");
    cfg_define_feature!("USE_CPU_NAIVE", "cpu_native");

    if cfg!(feature = "asan") {
        cfg.define("SANITIZE", "address");
    }

    if cfg!(feature = "fat_runtime") {
        cfg.define("FAT_RUNTIME", "ON");
    } else {
        cfg.define("FAT_RUNTIME", "OFF");
    }

    if cfg!(feature = "simd_specialization") {
        macro_rules! x86_64_feature {
            () => {{
                #[cfg(target_arch = "x86_64")]
                {
                    "ON"
                }
                #[cfg(not(target_arch = "x86_64"))]
                {
                    "OFF"
                }
            }};
        }

        macro_rules! aarch64_feature {
            () => {{
                #[cfg(target_arch = "aarch64")]
                {
                    "ON"
                }
                #[cfg(not(target_arch = "aarch64"))]
                {
                    "OFF"
                }
            }};
        }

        cfg.define("BUILD_AVX2", x86_64_feature!());
        cfg.define("BUILD_AVX512", x86_64_feature!());
        cfg.define("BUILD_AVX512VBMI", x86_64_feature!());

        cfg.define("BUILD_SVE", aarch64_feature!());
        cfg.define("BUILD_SVE2", aarch64_feature!());
        cfg.define("BUILD_SVE2_BITPERM", aarch64_feature!());
    } else {
        cfg.define("BUILD_AVX2", "OFF")
            .define("BUILD_AVX512", "OFF")
            .define("BUILD_AVX512VBMI", "OFF")
            .define("BUILD_SVE", "OFF")
            .define("BUILD_SVE2", "OFF")
            .define("BUILD_SVE2_BITPERM", "OFF");
    }

    // Vectorscan 5.4.11's dispatcher.c unconditionally references avx2_/avx512_/avx512vbmi_
    // symbols on x86_64 (only AVX512 has a DISABLE_*_DISPATCH guard, AVX2 has none),
    // so a fat_runtime build needs all three ISA variants compiled or the link fails.
    // 5.4.12's cflags-x86.cmake forced these on automatically; 5.4.11 doesn't, so do it here.
    #[cfg(target_arch = "x86_64")]
    if cfg!(feature = "fat_runtime") {
        cfg.define("BUILD_AVX2", "ON");
        cfg.define("BUILD_AVX512", "ON");
        cfg.define("BUILD_AVX512VBMI", "ON");
    }

    if cfg!(feature = "fat_runtime") {
        let libc_path = String::from_utf8(
            Command::new("cc")
                .args(["--print-file-name=libc.so.6"])
                .output()
                .expect("Failed to get libc.so.6 path from cc")
                .stdout,
        )
        .expect("Invalid UTF-8 in cc output")
        .trim()
        .to_string();
        std::env::set_var("VECTORSCAN_LIBC_SO", &libc_path);
        eprintln!("VECTORSCAN_LIBC_SO={}", std::env::var("VECTORSCAN_LIBC_SO").unwrap());
    }

    cfg.build();

    rename_library(out_dir);

    println!("cargo:rustc-link-search={}", out_dir.join("lib").display());
    println!(
        "cargo:rustc-link-search={}",
        out_dir.join("lib64").display()
    );
}

fn main() {
    let target_os = env("CARGO_CFG_TARGET_OS");
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let is_windows_msvc = target_os == "windows" && target_env == "msvc";

    println!("cargo:rerun-if-env-changed=VECTORSCAN_LIB_DIR");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=patches");

    // CARGO_FEATURE_* env vars are set by cargo when features are enabled.
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_FAT_RUNTIME");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_SIMD_SPECIALIZATION");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_CPU_NATIVE");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_UNIT_HYPERSCAN");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_ASAN");

    let manifest_dir = PathBuf::from(env("CARGO_MANIFEST_DIR"));

    // Fingerprint the submodule's git HEAD so that `git submodule update`
    // automatically triggers a rebuild without scanning thousands of files.
    if let Some(head_path) = resolve_submodule_head(&manifest_dir.join("vectorscan")) {
        println!("cargo:rerun-if-changed={}", head_path.display());
    }

    let out_dir = PathBuf::from(env("OUT_DIR"));

    if is_windows_msvc {
        let lib_dir = std::env::var("VECTORSCAN_LIB_DIR").expect(
            "VECTORSCAN_LIB_DIR must be set for MSVC targets, \
             pointing to a directory containing vs.lib (import library for vs.dll)",
        );
        println!("cargo:rustc-link-search={lib_dir}");
        println!("cargo:rustc-link-lib=dylib=vs");
    } else {
        let compiler_version_out = String::from_utf8(
            Command::new("c++")
                .args(["-v"])
                .output()
                .expect("Failed to get C++ compiler version")
                .stderr,
        )
        .unwrap();

        if compiler_version_out.contains("gcc") {
            println!("cargo:rustc-link-lib=stdc++");
        } else if compiler_version_out.contains("clang") {
            println!("cargo:rustc-link-lib=c++");
        } else {
            panic!("No compatible compiler found: either clang or gcc is needed");
        }

        if let Some(lib_dir) = std::env::var_os("VECTORSCAN_LIB_DIR") {
            println!("cargo:rustc-link-search={}", lib_dir.display());
        } else {
            build_vectorscan(&manifest_dir, &out_dir);
        }

        println!("cargo:rustc-link-lib=static=vs");

        #[cfg(feature = "unit_hyperscan")]
        {
            let unittests = out_dir.join("build").join("bin").join("unit-hyperscan");
            match Command::new(unittests).status() {
                Ok(rc) if rc.success() => {}
                Ok(rc) => panic!("Failed to run unit tests: exit with code {rc}"),
                Err(e) => panic!("Failed to run unit tests: {e}"),
            }
        }
    }

    #[cfg(feature = "bindgen")]
    {
        let config = bindgen::Builder::default()
            .allowlist_function("hs_.*")
            .allowlist_type("hs_.*")
            .allowlist_var("HS_.*")
            .header("wrapper.h")
            .clang_arg(format!("-I{}", &include_dir));
        config
            .generate()
            .expect("Unable to generate bindings")
            .write_to_file(out_dir.join("bindings.rs"))
            .expect("Failed to write Rust bindings to Vectorscan");
    }
    #[cfg(not(feature = "bindgen"))]
    {
        fs::copy("src/bindings.rs", out_dir.join("bindings.rs"))
            .expect("Failed to write Rust bindings to Vectorscan");
    }
}
