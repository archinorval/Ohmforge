# Roadmap and task list

Derived from `docs/DESIGN_AND_PLAN.md` section 14. Tick a box only when the work is verified and all checks in `AGENTS.md` pass; `[~]` means done in the working tree but not yet committed (awaiting Stefan's approval). Status as of 18 November 2026.

**Current phase: P1 Analog core** (`crates/of-analog`, `of-netlist::spice`, `ohmforge run`). Phase one (the RISC-V CPU core for the ESP32-C3, pulled forward from P2) is done in the working tree. P0 leftovers continue alongside.

## Phase one: RISC-V CPU core (`of-cpu-riscv`)

Status: the core is implemented and tested (62 unit tests, 1 doc test, 75 of 77 riscv-tests passing, 2 expected-unsupported). Design notes: `AGENTS.md` section 3a.

- [~] `Bus` trait (sizes, access kinds, wait states, custom CSR hooks, sync requests) and a `Ram` device
- [~] RV32I decoder and disassembler; RVC expander (all RV32C forms, reserved encodings rejected)
- [~] Execution of RV32I, M, A (optional, off on the C3) and C; Zicsr; Zifencei (`fence.i` as a no-op)
- [~] Machine-mode CSRs: `mstatus`, `misa`, `mie`/`mip`, `mtvec` (direct and vectored), `mscratch`, `mepc`, `mcause`, `mtval`, `mcycle[h]`, `minstret[h]`, `mcountinhibit`, ID CSRs, user counter shadows
- [~] Exceptions (misaligned, access fault, illegal, breakpoint, ecall) and interrupts with ESP32-C3 line mask; `wfi`
- [~] Cycle model (`TimingModel`, uncalibrated C3 estimates) and `SimTime` mapping (`run_until`, `idle_until`, `local_time`)
- [~] Debugger API: `step`, `run`, breakpoints, halt/resume/reset, register/CSR access, `disassemble_at`
- [~] riscv-tests harness (`tests/riscv_tests.rs`) and build scripts (`scripts/build-riscv-tests.sh` / `.ps1`), CI job
- [ ] Engine adapter: implement the `of-core` engine trait so the scheduler drives the hart in quanta (needs the P0 engine trait)
- [ ] `of-soc-esp32c3`: memory map bus (IROM/DROM via flash cache, IRAM/DRAM, ROM, peripherals) on top of `Bus`
- [ ] ESP32-C3 custom CSRs (e.g. performance counters `mpcer`/`mpcmr`/`mpccr`, GPIO CSRs) through `Bus::csr_read`/`csr_write`
- [ ] Interrupt matrix model feeding `set_interrupt_lines` (priority, threshold, edge/level)
- [ ] Calibrate `TimingModel` against a real ESP32-C3 (cycle counts of benchmark loops, flash cache misses)
- [ ] Decode cache (invalidated by `fence.i` and stores to code) for speed; benchmark MIPS
- [ ] Debug triggers (`tselect`/`tdata1`/`tdata2`) for hardware breakpoints and watchpoints; then pass `rv32mi-p-breakpoint`
- [ ] PMP (the C3 has 16 entries) and U-mode if firmware needs them; then pass `rv32mi-p-pmpaddr`
- [ ] `of-gdb`: remote serial protocol stub on top of the debugger API
- [ ] riscv-arch-test (the newer compliance suite) once a signature-dump harness exists

## P0 Foundations (weeks 0 to 6)

- [x] Cargo workspace with stub crates matching the planned layout
- [x] Licences (MIT OR Apache-2.0, CC-BY-4.0 parts), NOTICE, README, CONTRIBUTING, Code of Conduct
- [x] rustfmt, clippy, cargo-deny and toolchain configuration
- [x] CI workflow file (inert until pushed)
- [x] Example part (`parts/sensors/bosch-bme280/part.toml`) parsed by `of-parts`
- [~] Agent guides (`AGENTS.md`, `.grok/rules/`, this roadmap)
- [~] GUI scaffold ported to eframe 0.37 (`App::ui`) with X11/Wayland backends so the Linux CI GUI check builds
- [ ] `of-core`: scheduler snapshot/restore and replay log
- [ ] `of-core`: engine trait (`advance_to(t) -> next_event`) used by all engines
- [ ] Project file format v0 (`project.toml` + circuit file) with serde round-trip tests
- [ ] `of-parts`: JSON Schema generation (schemars) and a validator command in `of-cli`
- [ ] `of-cli`: `ohmforge validate-parts parts/`
- [ ] RFC template in `rfcs/0000-template.md`
- [ ] mdBook docs skeleton
- [ ] Fill in the Code of Conduct contact and choose the final project name

## P1 Analog core (weeks 4 to 16)

Status: the SPICE-grade solver is committed (`of-analog`, `of-netlist::spice`, `ohmforge run`); the deferred P1 models and analyses below are done in the working tree. `of-analog` has 21 unit tests, 29 analytic tests, 1 doc test and an ngspice reference set of 32 circuits (64 signals: OP, DC, nested DC, AC magnitude/phase and transient) all within 1 %; `of-netlist` has 7 tests. Design notes: `AGENTS.md` section 3b.

- [~] SPICE netlist subset parser (`of-netlist::spice`): R C L V I E G F H D Q M, `.model`, `.subckt`/`X` flattening, `.param` constants, `.include`, `.options`, `.ic`, `.op`, `.dc`, `.tran`
- [~] Netlist builder (`of_analog::netlist::build`): deck to MNA unknowns, ground handling, voltage-source and inductor branches, clear errors and unsupported-parameter warnings
- [~] Sparse MNA with faer sparse LU: pattern built once, symbolic factorisation reused, refactorisation skipped when the matrix is unchanged (linear fast path)
- [~] R, C, L (with `ic=`), independent V/I sources (DC, PULSE, SIN, EXP, PWL) and linear controlled sources E/G/F/H
- [~] DC operating point: Newton-Raphson with pnjlim/fetlim/limvds, then damped Newton, gmin stepping and source stepping; DC sweeps with warm starts
- [~] Transient: trapezoidal (default) and variable-step Gear-2, charge-based LTE step control (SPICE `CKTterr`), exact breakpoints at source corners, `uic`
- [~] Diode: Shockley with emission coefficient, series R, depletion and diffusion charge, breakdown (`BV`/`IBV`)
- [~] BJT: Gummel-Poon subset (IS BF BR NF NR VAF VAR IKF IKR ISE NE ISC NC RB RC RE CJE VJE MJE CJC VJC MJC TF TR FC)
- [~] MOSFET level 1 (Shichman-Hodges) with body effect, channel-length modulation, RD/RS, bulk diodes and overlap capacitances
- [~] `ohmforge run netlist.cir [--out x.csv] [--vcd x.vcd]`
- [~] ngspice comparison suite (`tests/ngspice_compare.rs`, golden CSVs generated by `scripts/gen-ngspice-golden.sh`, ngspice used only as an external program): all within 1 % of span
- [~] MOSFET Meyer gate capacitances (TOX) and bulk junction capacitances (CBD/CBS, CJ/CJSW with AD/AS/PD/PS, MJ/MJSW/PB/FC, JS)
- [~] MOSFET level 3 (SPICE semi-empirical: ETA, THETA, KAPPA, VMAX, XJ, DELTA, NFS, NSUB-derived parameters), exact derivatives via dual numbers; EKV-lite not planned
- [~] BJT extras: XTF/VTF/ITF transit-time modulation, IRB/RBM, CJS substrate cap (4th node), XCJC split, excess phase (PTF, transient), temperature (TEMP/TNOM, XTI/EG/XTB); gmin on the leakage diodes only, as SPICE
- [~] Temperature for diodes (IS, junction potential and CJO, BV with TCV, NBV) and MOSFETs (KP/U0, PHI, VTO, IS/JS, bulk caps); `.temp`, `.options temp/tnom`, instance `temp`/`dtemp`
- [~] Coupled inductors (`K`), switches (`S`/`W`, SW/CSW models with hysteresis), LED model (`.model x led(vf if n rs wavelength eqe)` with `popt()` and `bright()` probes), capacitor ESR/ESL, inductor `rser`, `tc1`/`tc2` tempcos and stored `tol=`
- [~] `.ac` small-signal analysis (dec/oct/lin, complex MNA on the same sparse pattern, magnitude/phase output); nested `.dc`; `.param` expressions with functions; subcircuit parameters and subcircuit-local `.model`/`.param`
- [ ] Switch timestep control near thresholds (ngspice `SWtrunc`), BJT excess phase in `.ac`, `.noise`/`.tf`/`.sens`, `.step`/Monte Carlo using `tol=`
- [ ] Bridge from `of-components` parts to `of-analog` devices (schematic to MNA without a SPICE deck)
- [ ] Engine adapter for the `of-core` scheduler (`advance_to(t) -> next_event`) and latent-subcircuit bypass
- [ ] MVP analog checks: LED with series resistor, transistor switch and AMS1117 macromodel within 1 % of ngspice

## P2 ESP32-C3 MVP (weeks 12 to 28)

- [~] RV32IMC interpreter passing riscv-tests (see phase one; riscv-arch-test still to do)
- [ ] ELF loading (`object`, `gimli`), flash image layout, ROM from esp-rom-elfs
- [ ] C3 memory map, interrupt matrix, systimer, timer groups, UART
- [ ] GPIO/IOMUX with electrical pin model (drive strength, thresholds, pulls)
- [ ] ADC with attenuation, nonlinearity and noise; LEDC PWM
- [ ] Mixed-signal sync: CPU stops on MMIO, analog advances to the exact time
- [ ] egui schematic editor (place, wire, probe), oscilloscope, logic analyser with UART decode, serial monitor
- [ ] BOM v1: CSV with MPN, supplier links, ZAR cost; generated build checklist
- [ ] MVP demo: blink, button interrupt, potentiometer to ADC, LED dimming, all from unmodified Arduino/ESP-IDF/esp-hal ELFs

### MVP definition (exit criteria for 0.1)

- [ ] Unmodified ESP32-C3 ELF (Arduino-ESP32, ESP-IDF or esp-hal) boots from the real ROM
- [ ] Blink, button with interrupt, potentiometer into ADC, LED dimming via LEDC behave as on hardware
- [ ] RC filter, LED with series resistor, transistor switch, AMS1117 regulator within 1 % of ngspice
- [ ] Oscilloscope, logic analyser with UART decode, multimeter probe, serial monitor
- [ ] BOM CSV export with MPNs, supplier links and ZAR cost, plus build checklist
- [ ] Desktop builds on Windows, macOS, Linux; deterministic headless runs in CI

## P3 Breadth (weeks 28 to 42), exit 0.2: common hobby projects run

- [ ] Classic ESP32 (Xtensa LX6) through a QEMU sidecar process (`of-cpu-qemu-ipc`), kept out of process for licence and stability reasons
- [ ] Peripherals: I2C, SPI, RMT, DAC (C3 and classic ESP32)
- [ ] Breadboard view alongside the schematic editor
- [ ] Parts library to about 50 parts (sensors, displays, drivers, regulators), each with a `part.toml`, behaviour and tests
- [ ] Op-amp, comparator and regulator macromodels and `.subckt` import (AMS1117, LM358, TL431)
- [ ] Electromechanical parts: DC motor with back-EMF and inertia, relay coil and contacts, servo
- [ ] KiCad schematic/netlist export (`of-kicad`) and XLSX BOM export
- [ ] Release 0.2

## P4 Own Xtensa core and web (weeks 40 to 60), exit 0.3: classic ESP32 in the browser

- [ ] Own Rust Xtensa LX6/LX7 core (`of-cpu-xtensa`) replacing the QEMU sidecar where possible
- [ ] Web app (`of-web`, wasm32) running the simulation in the browser
- [ ] Wi-Fi virtual radio (simulated network between boards and a host bridge)
- [ ] Sleep modes and a power profiler (current over time, battery life estimate)
- [ ] Energy parts: Li-ion/LiPo with state of charge and internal resistance, coin cell, USB 5 V source
- [ ] Release 0.3

## P5 Realism (weeks 56 to 72), exit 0.4: validation report published

- [ ] Monte Carlo runs over part tolerances
- [ ] Thermal model (self-heating, ambient, tempco; MOSFET Rds(on) against temperature)
- [ ] Fault injection (opens, shorts, brown-outs, noisy supplies)
- [ ] Hardware-in-the-loop lab: compare simulation with real boards
- [ ] Supplier price plugins for the BOM
- [ ] Publish the validation report (simulation against hardware and ngspice); release 0.4

## P6 1.0 (weeks 70 to 84)

- [ ] Stable component API and WASM SDK for third-party parts (`of-component-api`, `of-plugin-wasm`)
- [ ] ESP32-S3 and ESP32-C6 support
- [ ] Complete documentation (mdBook user guide, part-authoring guide, API docs)
- [ ] BLE (stretch goal)
- [ ] Release 1.0
