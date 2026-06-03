# scripts

## `sde-verify.ps1`

Verifies the **fat runtime** on `x86_64-pc-windows-msvc` by running the example
under [Intel SDE](https://www.intel.com/content/www/us/en/developer/articles/tool/software-development-emulator.html)
for each micro-architecture tier.

The fat runtime compiles the scanner once per micro-arch and dispatches on CPUID
at load time, so on a single host only one variant ever runs. SDE emulates both
CPUID and the instruction set, letting us force each variant and fault on any
out-of-ISA instruction. Identical, correct output under every emulated CPU proves
each variant is ISA-clean, the dispatcher routes correctly, and the COFF symbol
rename (`fat_rename.py`) didn't break any variant.

| Variant      | SDE CPU | Emulated     | Routes because                     |
|--------------|---------|--------------|------------------------------------|
| `corei7`     | `-snb`  | Sandy Bridge | SSE4.2+POPCNT+AVX, **no** AVX2     |
| `avx2`       | `-hsw`  | Haswell      | AVX2                               |
| `avx512`     | `-skx`  | Skylake-X    | AVX-512 F/BW/CD/DQ/VL              |
| `avx512vbmi` | `-icx`  | Ice Lake-SP  | AVX-512 VBMI                       |

`core2` is not SDE-tested: a pre-AVX emulated CPU faults inside the host's own
`VCRUNTIME140.dll` (which uses AVX) — a host-CRT limitation, not a vectorscan bug.

### Usage
```powershell
$env:SDE_HOME = "C:\path\to\sde-external-...-win"   # or put sde.exe on PATH
$env:VECTORSCAN_BOOST_INCLUDE = "C:\path\to\boost-root"
pwsh scripts/sde-verify.ps1            # debug build
pwsh scripts/sde-verify.ps1 -Release   # release build
```
Exits non-zero if any variant fails (bad exit code, out-of-ISA fault, or output
that differs from the native baseline).
