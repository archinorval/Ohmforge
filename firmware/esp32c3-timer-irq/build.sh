#!/usr/bin/env bash
# Rebuild timer_irq.elf. Needs zig (`pip install ziglang`), which bundles
# clang and lld for riscv32. The ELF is committed so tests need no
# toolchain; CI rebuilds it and checks it is byte-identical.
set -euo pipefail
cd "$(dirname "$0")"
ZIGCC=${ZIGCC:-python3 -m ziglang cc}
$ZIGCC -target riscv32-freestanding-none -mcpu=generic_rv32+m+c \
    -nostdlib -static -g0 -Wl,-T,link.ld -Wl,--build-id=none -Wl,-z,norelro -Wl,-z,max-page-size=16 \
    -o timer_irq.elf timer_irq.S
# Linkers mark their output executable; the committed ELF is a data file (100644).
chmod 644 timer_irq.elf
echo "built $(wc -c < timer_irq.elf) bytes"
