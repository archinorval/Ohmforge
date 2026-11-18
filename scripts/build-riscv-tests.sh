#!/usr/bin/env bash
# Build the official riscv-tests (BSD-3-Clause) "p" environment ELFs for the
# RV32 core's compliance test. Output: target/riscv-tests (never committed).
#
# Needs git and one RISC-V capable compiler, tried in this order:
#   $RISCV_CC (full command), riscv64-unknown-elf-gcc, riscv32-unknown-elf-gcc,
#   clang, zig cc (e.g. `pip install ziglang` gives `python -m ziglang cc`).
# Usage: scripts/build-riscv-tests.sh [output-dir]
set -euo pipefail

REV=3df703cd52d5f0d8224e3c50bf03c6cb70efff98   # pinned riscv-tests commit
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/target/riscv-tests}"
SRC="$ROOT/target/riscv-tests-src"
SUITES="rv32ui rv32um rv32ua rv32uc rv32mi"

if [ ! -d "$SRC/.git" ]; then
  git clone --quiet https://github.com/riscv-software-src/riscv-tests.git "$SRC"
fi
git -C "$SRC" fetch --quiet origin "$REV" 2>/dev/null || true
git -C "$SRC" checkout --quiet "$REV"
git -C "$SRC" submodule update --init --quiet env

if [ -n "${RISCV_CC:-}" ]; then
  read -r -a CC <<<"$RISCV_CC"
  ARCH=(-march=rv32imac_zicsr_zifencei -mabi=ilp32)
elif command -v riscv64-unknown-elf-gcc >/dev/null; then
  CC=(riscv64-unknown-elf-gcc); ARCH=(-march=rv32imac_zicsr_zifencei -mabi=ilp32)
elif command -v riscv32-unknown-elf-gcc >/dev/null; then
  CC=(riscv32-unknown-elf-gcc); ARCH=(-march=rv32imac_zicsr_zifencei -mabi=ilp32)
elif command -v clang >/dev/null && clang --print-targets 2>/dev/null | grep -q riscv32; then
  CC=(clang --target=riscv32-unknown-elf -fuse-ld=lld); ARCH=(-march=rv32imac_zicsr_zifencei -mabi=ilp32)
elif command -v zig >/dev/null; then
  CC=(zig cc); ARCH=(-target riscv32-freestanding-none -mcpu=generic_rv32+m+a+c)
elif python3 -m ziglang version >/dev/null 2>&1; then
  CC=(python3 -m ziglang cc); ARCH=(-target riscv32-freestanding-none -mcpu=generic_rv32+m+a+c)
else
  echo "error: no RISC-V compiler found (set RISCV_CC)" >&2; exit 1
fi
[[ -n "${RISCV_CC:-}" && -n "${RISCV_ARCH:-}" ]] && read -r -a ARCH <<<"$RISCV_ARCH"

mkdir -p "$OUT"
cd "$SRC"
FLAGS=("${ARCH[@]}" -nostdlib -static -Ienv/p -Iisa/macros/scalar -Wl,-T,env/p/link.ld)
built=0; failed=0
for suite in $SUITES; do
  for src in isa/"$suite"/*.S; do
    t="$(basename "$src" .S)"
    out="$OUT/$suite-p-$t"
    if "${CC[@]}" "${FLAGS[@]}" "$src" -o "$out" 2>/dev/null; then
      built=$((built + 1)); continue
    fi
    # Some assemblers reject the env's `.weak mtvec_handler` when the test
    # defines that symbol globally; preprocess and drop the weak directive
    # (the preprocessor joins macro lines with ';', so edit, don't delete).
    tmp="$(mktemp --suffix=.s)"
    if "${CC[@]}" "${ARCH[@]}" -E -Ienv/p -Iisa/macros/scalar "$src" | sed 's/\.weak mtvec_handler;\{0,1\}//' >"$tmp" &&
       "${CC[@]}" "${FLAGS[@]}" -x assembler "$tmp" -o "$out"; then
      built=$((built + 1))
    else
      echo "warning: could not build $suite/$t" >&2; failed=$((failed + 1))
    fi
    rm -f "$tmp"
  done
done
echo "built $built tests into $OUT ($failed failed)"
echo "run: cargo test -p of-cpu-riscv --test riscv_tests -- --nocapture"
