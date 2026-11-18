//! Runs the official riscv-tests (BSD-3-Clause, built from source; the
//! binaries are never committed) against the core.
//!
//! Build them with `scripts/build-riscv-tests.sh` (or `.ps1`), which puts
//! ELF files in `target/riscv-tests`. Point `OF_RISCV_TESTS_DIR` elsewhere
//! to override. If the directory is missing the test is skipped with a note,
//! so a plain `cargo test` never needs a RISC-V toolchain.

use std::path::{Path, PathBuf};

use of_cpu_riscv::{EbreakAction, Hart, HartConfig, MisalignedPolicy, Ram, RunExit, Size};

const RAM_BASE: u32 = 0x8000_0000;
const RAM_SIZE: usize = 1 << 20;
const MAX_CYCLES: u64 = 5_000_000;

/// Tests that need features the core deliberately does not implement yet.
const EXPECTED_UNSUPPORTED: &[(&str, &str)] = &[
    ("rv32mi-p-breakpoint", "debug triggers (tselect/tdata) not implemented"),
    ("rv32mi-p-pmpaddr", "PMP not implemented"),
];

/// Minimal ELF32 little-endian reader: loadable segments and one symbol.
struct Elf {
    entry: u32,
    segments: Vec<(u32, Vec<u8>)>,
    tohost: u32,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn parse_elf(b: &[u8]) -> Result<Elf, String> {
    if b.len() < 52 || &b[..4] != b"\x7fELF" || b[4] != 1 || b[5] != 1 {
        return Err("not a little-endian ELF32 file".into());
    }
    if u16_at(b, 18) != 243 {
        return Err("not a RISC-V ELF".into());
    }
    let entry = u32_at(b, 24);
    if entry == 0 {
        return Err("ELF entry point is 0 (missing _start?)".into());
    }
    let (phoff, shoff) = (u32_at(b, 28) as usize, u32_at(b, 32) as usize);
    let (phentsize, phnum) = (u16_at(b, 42) as usize, u16_at(b, 44) as usize);
    let (shentsize, shnum) = (u16_at(b, 46) as usize, u16_at(b, 48) as usize);

    let mut segments = Vec::new();
    for i in 0..phnum {
        let p = phoff + i * phentsize;
        if u32_at(b, p) != 1 {
            continue; // PT_LOAD only
        }
        let (off, paddr) = (u32_at(b, p + 4) as usize, u32_at(b, p + 12));
        let (filesz, memsz) = (u32_at(b, p + 16) as usize, u32_at(b, p + 20) as usize);
        let mut data = b.get(off..off + filesz).ok_or("segment out of range")?.to_vec();
        data.resize(memsz, 0);
        segments.push((paddr, data));
    }

    let mut tohost = None;
    for i in 0..shnum {
        let s = shoff + i * shentsize;
        if u32_at(b, s + 4) != 2 {
            continue; // SHT_SYMTAB
        }
        let (off, size, link) = (
            u32_at(b, s + 16) as usize,
            u32_at(b, s + 20) as usize,
            u32_at(b, s + 24) as usize,
        );
        let strtab = u32_at(b, shoff + link * shentsize + 16) as usize;
        for sym in (off..off + size).step_by(16) {
            let name = strtab + u32_at(b, sym) as usize;
            let end = b[name..].iter().position(|&c| c == 0).map(|n| name + n).unwrap_or(name);
            if &b[name..end] == b"tohost" {
                tohost = Some(u32_at(b, sym + 4));
            }
        }
    }
    Ok(Elf {
        entry,
        segments,
        tohost: tohost.ok_or("no tohost symbol")?,
    })
}

enum Verdict {
    Pass,
    Fail(u32),
    Timeout(RunExit, u32),
}

fn run_one(path: &Path, misaligned: MisalignedPolicy) -> Result<Verdict, String> {
    let elf = parse_elf(&std::fs::read(path).map_err(|e| e.to_string())?)?;
    let mut ram = Ram::new(RAM_BASE, RAM_SIZE);
    for (addr, data) in &elf.segments {
        ram.load(*addr, data)
            .map_err(|e| format!("segment at {:#x}: {e:?}", e.addr))?;
    }
    let config = HartConfig {
        reset_pc: elf.entry,
        ebreak: EbreakAction::Trap,
        misaligned,
        ..HartConfig::generic_imac()
    };
    let mut hart = Hart::new(config);
    let mut exit = RunExit::BudgetExhausted;
    while hart.elapsed_cycles() < MAX_CYCLES {
        exit = hart.run(&mut ram, 10_000);
        let v = ram.peek(elf.tohost, Size::Word).unwrap_or(0);
        if v != 0 {
            return Ok(if v == 1 { Verdict::Pass } else { Verdict::Fail(v >> 1) });
        }
        if !matches!(exit, RunExit::BudgetExhausted) {
            break;
        }
    }
    Ok(Verdict::Timeout(exit, hart.pc()))
}

fn tests_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OF_RISCV_TESTS_DIR") {
        return dir.into();
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/riscv-tests")
}

#[test]
fn riscv_tests_compliance() {
    let dir = tests_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!(
            "skipping: no riscv-tests at {} (run scripts/build-riscv-tests)",
            dir.display()
        );
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("rv32") && n.contains("-p-") && !n.contains('.'))
        .collect();
    names.sort();
    let (mut passed, mut skipped, mut failures) = (0, 0, Vec::new());
    for name in &names {
        let unsupported = EXPECTED_UNSUPPORTED.iter().find(|(n, _)| n == name);
        // ma_data has no trap handler: it checks hardware misaligned support.
        let policy = if name.ends_with("ma_data") {
            MisalignedPolicy::Allow
        } else {
            MisalignedPolicy::Trap
        };
        match run_one(&dir.join(name), policy) {
            Ok(Verdict::Pass) => {
                passed += 1;
                if unsupported.is_some() {
                    eprintln!("note: {name} now passes; remove it from EXPECTED_UNSUPPORTED");
                }
            }
            _ if unsupported.is_some() => skipped += 1,
            Ok(Verdict::Fail(t)) => failures.push(format!("{name}: failed test case {t}")),
            Ok(Verdict::Timeout(exit, pc)) => failures.push(format!("{name}: no result ({exit:?}, pc {pc:#010x})")),
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    eprintln!(
        "riscv-tests: {passed} passed, {skipped} expected-unsupported, {} failed of {}",
        failures.len(),
        names.len()
    );
    assert!(failures.is_empty(), "riscv-tests failures:\n{}", failures.join("\n"));
    assert!(!names.is_empty(), "no rv32*-p-* tests found in {}", dir.display());
}
