# Ohmforge project rules (Grok Build)

Grok Build also reads the root `AGENTS.md`; that file is the full guide and wins on any conflict. Short version:

1. Source of truth: `docs/DESIGN_AND_PLAN.md` (never edit it unless Stefan asks). Current tasks: `docs/ROADMAP.md`.
2. Current phase: **P1 Analog core** (`crates/of-analog`, `of-netlist::spice`, `ohmforge run`; design notes in `AGENTS.md` section 3b), now including `.ac`, nested `.dc`, `.param` expressions, temperature, MOS level 3/Meyer caps, full Gummel-Poon, K/S/W, LED and ESR/ESL. Phase one, the RISC-V core (`crates/of-cpu-riscv`, section 3a), is done. New analog features need an analytic test and an ngspice golden circuit (`* ngspice-replace:` for Ohmforge-only syntax), all within 1 %.
3. Rust edition 2024, MSRV 1.95. On Windows, load the Build Tools `vcvars64.bat` environment for that machine first (`AGENTS.md` section 9). Then run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, the GUI check and the wasm32 check from `AGENTS.md`.
4. No GPL/LGPL/AGPL dependencies; parts data is CC-BY-4.0; never copy code from QEMU, CircuitJS, SimulIDE or Wokwi. riscv-tests (BSD-3) are built from source, never committed. ngspice is only ever run as an external tool to make golden CSVs (`scripts/gen-ngspice-golden.sh`); never link, vendor or ship it.
5. Git: **no commits without Stefan's approval**; leave changes uncommitted and propose a breakdown. Approved commits are local, signed off (`git commit -s`) Conventional Commits. Never add a remote or push.
6. "Ohmforge" and `of-` are placeholder names (`scripts/rename.ps1`).
7. Simulation crates stay deterministic (virtual `SimTime`, no wall-clock) and wasm32-compatible.
