# Build the official riscv-tests (BSD-3-Clause) "p" environment ELFs for the
# RV32 core's compliance test. Output: target\riscv-tests (never committed).
#
# Needs git and one RISC-V capable compiler, tried in this order:
#   $env:RISCV_CC (full command), riscv64-unknown-elf-gcc, clang, zig cc
#   (e.g. `pip install ziglang` gives `python -m ziglang cc`).
# Usage: scripts\build-riscv-tests.ps1 [-Out <dir>]
param([string]$Out = "")
$ErrorActionPreference = "Stop"

$Rev = "3df703cd52d5f0d8224e3c50bf03c6cb70efff98"   # pinned riscv-tests commit
$Root = Split-Path -Parent $PSScriptRoot
if (-not $Out) { $Out = Join-Path $Root "target\riscv-tests" }
$Src = Join-Path $Root "target\riscv-tests-src"
$Suites = "rv32ui", "rv32um", "rv32ua", "rv32uc", "rv32mi"

if (-not (Test-Path (Join-Path $Src ".git"))) {
    git clone --quiet https://github.com/riscv-software-src/riscv-tests.git $Src
}
git -C $Src fetch --quiet origin $Rev 2>$null
git -C $Src checkout --quiet $Rev
git -C $Src submodule update --init --quiet env

$Gnu = @("-march=rv32imac_zicsr_zifencei", "-mabi=ilp32")
$Zig = @("-target", "riscv32-freestanding-none", "-mcpu=generic_rv32+m+a+c")
if ($env:RISCV_CC) {
    $Cc = $env:RISCV_CC -split " "; $Arch = if ($env:RISCV_ARCH) { $env:RISCV_ARCH -split " " } else { $Gnu }
} elseif (Get-Command riscv64-unknown-elf-gcc -ErrorAction SilentlyContinue) {
    $Cc = @("riscv64-unknown-elf-gcc"); $Arch = $Gnu
} elseif (Get-Command zig -ErrorAction SilentlyContinue) {
    $Cc = @("zig", "cc"); $Arch = $Zig
} elseif ((Get-Command python -ErrorAction SilentlyContinue) -and (python -m ziglang version 2>$null)) {
    $Cc = @("python", "-m", "ziglang", "cc"); $Arch = $Zig
} elseif (Get-Command clang -ErrorAction SilentlyContinue) {
    $Cc = @("clang", "--target=riscv32-unknown-elf", "-fuse-ld=lld"); $Arch = $Gnu
} else {
    throw "no RISC-V compiler found (set RISCV_CC)"
}
$Exe = $Cc[0]; $Pre = @($Cc | Select-Object -Skip 1)

New-Item -ItemType Directory -Force $Out | Out-Null
Push-Location $Src
try {
    $Flags = $Arch + @("-nostdlib", "-static", "-Ienv/p", "-Iisa/macros/scalar", "-Wl,-T,env/p/link.ld")
    $built = 0; $failed = 0
    foreach ($suite in $Suites) {
        foreach ($file in Get-ChildItem "isa/$suite/*.S") {
            $t = $file.BaseName
            $dst = Join-Path $Out "$suite-p-$t"
            & $Exe @Pre @Flags $file.FullName -o $dst 2>$null
            if ($LASTEXITCODE -eq 0) { $built++; continue }
            # Some assemblers reject the env's `.weak mtvec_handler` when the
            # test defines it globally; preprocess and drop the directive (the
            # preprocessor joins macro lines with ';', so edit, don't delete).
            $tmp = [System.IO.Path]::GetTempFileName() + ".s"
            $text = & $Exe @Pre @Arch -E -Ienv/p -Iisa/macros/scalar $file.FullName
            ($text | ForEach-Object { $_ -replace "\.weak mtvec_handler;?", "" }) | Set-Content -Encoding ascii $tmp
            & $Exe @Pre @Flags -x assembler $tmp -o $dst
            if ($LASTEXITCODE -eq 0) { $built++ } else { Write-Warning "could not build $suite/$t"; $failed++ }
            Remove-Item -Force $tmp -ErrorAction SilentlyContinue
        }
    }
    Write-Host "built $built tests into $Out ($failed failed)"
    Write-Host "run: cargo test -p of-cpu-riscv --test riscv_tests -- --nocapture"
} finally { Pop-Location }
