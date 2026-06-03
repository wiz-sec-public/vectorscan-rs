# `vectorscan-rs-sys`


## Overview
This crate implements minimal Rust bindings to the [Vectorscan](https://github.com/Vectorcamp/vectorscan) fork of [Hyperscan](https://github.com/intel/hyperscan), the high-performance regular expression engine.
This crate builds a vendored copy of Vectorscan from source.


## Dependencies
- [Boost](https://boost.org) >= 1.57
- [CMake](https://cmake.org)
- [Ragel](https://github.com/adrian-thurston/ragel) (state machine compiler, used by Vectorscan's parser)
- `patch`
- Optional: [Clang](https://clang.llvm.org), when building with the `bindgen` feature

### Linux
```
apt install build-essential cmake libboost-all-dev ragel patch pkg-config libsqlite3-dev
```

This has been tested on x86_64 Linux.


## Vectorscan Source

Vectorscan is included as a git submodule at [`vectorscan/`](vectorscan/) pinned to tag `vectorscan/5.4.11`.
The build script copies this source into `OUT_DIR`, applies the [patches](#patches) below, then builds via CMake.

To initialize after cloning:
```
git submodule update --init
```


## Patches

A set of patches in [`patches/`](patches/) are applied on top of the upstream Vectorscan 5.4.11 source at build time.
They are applied in alphabetical order (hence the numeric prefix).
These patches relax some platform assumptions that don't hold outside Linux.

### `01-static-dispatch.patch`
Replaces the `__attribute__((ifunc(...)))` dispatch mechanism in `src/dispatcher.c` with a static function pointer pattern.
The ifunc attribute is a glibc/ELF feature that does not exist on Windows (PE/COFF).
The replacement works identically: the first call resolves the best SIMD implementation for the current CPU and caches the function pointer; subsequent calls go directly through the pointer.

Based on upstream commit [`c743bb3`](https://github.com/VectorCamp/vectorscan/commit/c743bb320a0727033eb267ade256cb7a2d0eb239), adapted for 5.4.11.

### `02-remove-ifunc-and-linux-only.patch`
Modifies `cmake/osdetection.cmake` to:
- Remove the `FAT_RUNTIME AND NOT LINUX` fatal error, allowing fat runtime builds on Windows.
- Remove the ifunc compiler attribute check, since `01-static-dispatch.patch` eliminates the need for ifunc.
- Relax the cmake generator check from `"Unix Makefiles"` to `"Makefiles"` to allow MSYS Makefiles.

Based on upstream commit [`0ba7222`](https://github.com/VectorCamp/vectorscan/commit/0ba7222ca832b4e56ba457daf59f226050fcd1c4) by voidbar, extended for 5.4.11.

### `03-build-wrapper-mingw.patch`
Modifies `cmake/build_wrapper.sh` (used by the fat runtime to rename symbols per microarchitecture) to work on MinGW:
- Uses configurable `$NM`, `$OBJCOPY`, `$OBJDUMP` instead of hardcoded tool names.
- Reads the C runtime library path from `$VECTORSCAN_LIBC_SO` (set by `build.rs`) instead of hardcoding `libc.so.6`.
- Detects shared vs static libraries for correct `nm` flags (`-D` for `.so`, omitted for `.a`).
- Renames `.refptr` COMDAT sections to avoid link-time conflicts on MinGW.

This patch has no upstream equivalent; it is custom for Windows MinGW support.

### `04-pkgconfig-extra-libs.patch`
Adds a `PKGCONFIG_EXTRA_LIBS` cmake cache variable and substitutes it into `libhs.pc.in` and `chimera/libch.pc.in`.
This allows downstream consumers using pkg-config from a plain C compiler (e.g. CGO with `gcc` instead of `g++`) to link the C++ standard library by passing `-DPKGCONFIG_EXTRA_LIBS="-lstdc++ -lm"`.

Based on upstream commit [`f379157`](https://github.com/VectorCamp/vectorscan/commit/f3791575bbddd887046aa510193bea2cfacbb2fa) by voidbar, adapted for 5.4.11.

### `05-pkgconfig-quiet.patch`
Changes `find_package(PkgConfig REQUIRED)` to `find_package(PkgConfig QUIET)` in `CMakeLists.txt`.
On Windows, pkg-config is often unavailable; making it optional allows cmake to proceed since Vectorscan does not strictly require it to build.

This change is present in upstream master but not in 5.4.11.


## Implementation Notes
This crate was originally written as part of [Nosey Parker](https://github.com/praetorian-inc/noseyparker).
It was adapted from the [pyperscan](https://github.com/vlaci/pyperscan) project, which uses Rust to expose Hyperscan to Python.
(That project is released under either the Apache 2.0 or MIT license.)

The only bindings exposed at present are for Vectorscan's block-based matching APIs.
The various other APIs such as stream- and vector-based matching are not exposed.
Other features, such as the Chimera PCRE library, test code, benchmark code, and supporting utilities are disabled.


## License
This project is licensed under either of

- [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0)
  ([LICENSE-APACHE](../LICENSE-APACHE))

- [MIT License](https://opensource.org/licenses/MIT)
  ([LICENSE-MIT](../LICENSE-MIT))

at your option.

This project contains a vendored copy of [Vectorscan](https://github.com/Vectorcamp/vectorscan), which is released under a 3-clause BSD license.
See the [NOTICE](../NOTICE) file for details.


## Contributing
Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in `vectorscan-rs-sys` by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
