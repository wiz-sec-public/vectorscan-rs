#!/usr/bin/env pwsh
#
# SDE fat-runtime variant verification for vectorscan-rs (x86_64-pc-windows-msvc).
#
# The fat runtime compiles the scanner once per micro-architecture and selects
# the best variant at load time via CPUID. On any single host we only ever run
# the one variant the host CPU supports, so a normal test exercises just one of
# them. Intel SDE emulates *both* CPUID and the instruction set, so running the
# example under `sde -<cpu>` forces a specific variant AND faults (#UD /
# "not valid for specified chip") on any instruction outside that CPU's ISA.
#
# Identical, correct output under each emulated CPU therefore proves:
#   (a) every variant is ISA-clean (no instruction above its declared tier),
#   (b) the CPUID dispatcher routes correctly for each CPU tier,
#   (c) the COFF symbol rename (fat_rename.py) didn't break any variant's
#       internal cross-references.
#
# Requirements:
#   - Intel SDE: either on PATH, or $env:SDE_HOME pointing at the extracted SDE
#     directory (the one containing sde.exe / sde64.exe). Known-good build:
#     https://downloadmirror.intel.com/831748/sde-external-9.44.0-2024-08-22-win.tar.xz
#   - The MSVC build toolchain (clang-cl, ninja, llvm-nm, llvm-objcopy, python)
#     and $env:VECTORSCAN_BOOST_INCLUDE, as documented in
#     vectorscan-rs-sys/README.md.
#
# Usage:
#   pwsh scripts/sde-verify.ps1            # debug build (default)
#   pwsh scripts/sde-verify.ps1 -Release   # release build

[CmdletBinding()]
param(
    [switch]$Release
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$target   = "x86_64-pc-windows-msvc"
$repoRoot = Split-Path -Parent $PSScriptRoot

# variant -> minimal SDE CPU that routes the dispatcher to that variant.
#
# core2 is intentionally NOT tested: emulating a pre-AVX CPU faults inside the
# host's own VCRUNTIME140.dll, which itself uses AVX -- a host-CRT limitation,
# not a vectorscan issue. -snb has AVX (so the CRT is happy) but no AVX2, so the
# dispatcher still selects the corei7 variant, which is what we want to exercise.
$cases = [ordered]@{
    corei7     = "-snb"   # Sandy Bridge  : SSE4.2 + POPCNT + AVX, no AVX2
    avx2       = "-hsw"   # Haswell       : AVX2
    avx512     = "-skx"   # Skylake-X     : AVX-512 F/BW/CD/DQ/VL
    avx512vbmi = "-icx"   # Ice Lake-SP   : AVX-512 VBMI
}

function Find-Sde {
    foreach ($name in @("sde64.exe", "sde.exe")) {
        $cmd = Get-Command $name -ErrorAction SilentlyContinue
        if ($cmd) { return $cmd.Source }
        if ($env:SDE_HOME) {
            $candidate = Join-Path $env:SDE_HOME $name
            if (Test-Path $candidate) { return $candidate }
        }
    }
    return $null
}

$sde = Find-Sde
if (-not $sde) {
    throw "Intel SDE not found. Put sde64.exe/sde.exe on PATH or set `$env:SDE_HOME " +
          "to the extracted SDE directory. See the header of this script for the download URL."
}
Write-Host "Using SDE: $sde"

$profileArg = if ($Release) { @("--release") } else { @() }
$profileDir = if ($Release) { "release" } else { "debug" }

Push-Location $repoRoot
try {
    Write-Host "==> Building fat-runtime example ($profileDir)..."
    & cargo build -q -p vectorscan-rs-example --target $target `
        --features vectorscan-rs/fat_runtime @profileArg
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $exe = Join-Path $repoRoot "target/$target/$profileDir/vectorscan-rs-example.exe"
    if (-not (Test-Path $exe)) { throw "example exe not found at $exe" }

    # Native baseline: the matches the dispatcher's host variant produces.
    Write-Host "==> Native baseline run..."
    $baselineOut = & $exe 2>&1
    if ($LASTEXITCODE -ne 0) { throw "native run failed:`n$($baselineOut -join "`n")" }
    $baseline = ($baselineOut | Where-Object { $_ -match "match id=" }) -join "`n"
    Write-Host ($baselineOut -join "`n")
    if (-not $baseline) { throw "native run produced no 'match id=' lines" }

    $failures = 0
    foreach ($variant in $cases.Keys) {
        $flag = $cases[$variant]
        Write-Host ""
        Write-Host "==> $variant  (sde $flag)"
        $out = & $sde $flag -- $exe 2>&1
        $rc  = $LASTEXITCODE
        $text = $out -join "`n"

        if ($rc -ne 0) {
            Write-Host "    FAIL: exited $rc"
            Write-Host $text
            $failures++; continue
        }
        if ($text -match "not valid for specified chip|SDE-ERROR|TID .* ICOUNT") {
            Write-Host "    FAIL: out-of-ISA instruction for this chip"
            Write-Host $text
            $failures++; continue
        }
        $matchesOut = ($out | Where-Object { $_ -match "match id=" }) -join "`n"
        if ($matchesOut -ne $baseline) {
            Write-Host "    FAIL: matches differ from native baseline"
            Write-Host "    expected:`n$baseline"
            Write-Host "    got:`n$matchesOut"
            $failures++; continue
        }
        Write-Host "    OK: ISA-clean, matches baseline"
    }

    Write-Host ""
    if ($failures -ne 0) {
        throw "$failures of $($cases.Count) variant(s) FAILED"
    }
    Write-Host "PASS: all $($cases.Count) fat-runtime variants verified under SDE."
} finally {
    Pop-Location
}
