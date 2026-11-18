# Test firmware

Small firmware images that the simulator's tests run. Everything here is our
own code (AGPL-3.0-or-later); third-party binaries are never committed.

| Folder | What it does | Used by |
|---|---|---|
| `esp32c3-blinky` | Bare-metal RV32IMC assembly for the ESP32-C3: prints `ohmforge blinky` on UART0, routes GPIO8 through the IO MUX and GPIO matrix, then toggles it every 250 µs using SYSTIMER deadlines | `crates/of-soc-esp32c3/tests/blinky.rs`, `tests/blinky_led.rs` |
| `esp32c3-timer-irq` | Prints `ohmforge timer irq`, switches off the RTC WDT, both MWDTs and the super watchdog (auto-feed), routes SYSTIMER target 0 (period 250 µs) to CPU interrupt 1 and the TIMG0 timer 0 alarm (1 ms, auto-reload) to CPU interrupt 2 through the interrupt matrix, then sleeps in `wfi`; the vectored handlers toggle GPIO8 and count ticks | `crates/of-soc-esp32c3/tests/timer_irq.rs` |

Each folder holds the source, a linker script, a `build.sh` and the built ELF.
The ELF is committed (a few KB) so the tests need no RISC-V toolchain.

## Rebuilding

The build needs zig, which bundles clang and lld for riscv32:

```sh
pip install ziglang==0.17.1
bash firmware/esp32c3-blinky/build.sh
bash firmware/esp32c3-timer-irq/build.sh
```

The build is reproducible: it embeds no paths or build IDs, and `build.sh`
resets the ELF to mode 644. CI rebuilds every image with the pinned zig and
fails if the result differs from the committed file, so commit the source and
the rebuilt ELF together.
