#!/usr/bin/env bash
# Rebuild blinky.elf. Needs zig (`pip install ziglang`), which bundles
# clang and lld for riscv32. The ELF is committed so tests need no
# toolchain; CI rebuilds it and checks it is byte-identical.
set -euo pipefail
cd "$(dirname "$0")"
ZIGCC=${ZIGCC:-python3 -m ziglang cc}
$ZIGCC -target riscv32-freestanding-none -mcpu=generic_rv32+m+c \
    -nostdlib -static -g0 -Wl,-T,link.ld -Wl,--build-id=none -Wl,-z,norelro -Wl,-z,max-page-size=16 \
    -o blinky.elf blinky.S
echo "built $(wc -c < blinky.elf) bytes"
