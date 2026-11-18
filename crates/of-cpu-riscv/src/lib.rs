//! RV32 CPU core for the ESP32-C3 (and later ESP32-C6).
//!
//! Implements RV32I with the M, A (optional, off on the C3) and C
//! extensions, Zicsr and Zifencei, machine-mode CSRs, exceptions and
//! interrupts (direct and vectored `mtvec`), cycle counting tied to the
//! `of-core` picosecond clock, and a stepping/breakpoint API for the
//! debugger and GDB stub.
//!
//! The core is a plain state machine: [`Hart::step`] executes one
//! instruction against any [`Bus`], so the SoC crate decides the memory map,
//! wait states, custom CSRs and when to hand control back to the scheduler.
//!
//! ```
//! use of_cpu_riscv::{Hart, HartConfig, Ram, RunExit, asm};
//!
//! let mut cfg = HartConfig::generic_imac();
//! cfg.ebreak = of_cpu_riscv::EbreakAction::Halt;
//! let mut ram = Ram::new(0x8000_0000, 4096);
//! ram.load_words(0x8000_0000, &[asm::addi(10, 0, 40), asm::addi(10, 10, 2), asm::EBREAK]).unwrap();
//! let mut hart = Hart::new(cfg);
//! assert!(matches!(hart.run(&mut ram, 100), RunExit::Halted(_)));
//! assert_eq!(hart.reg(10), 42);
//! ```

pub mod asm;
pub mod bus;
pub mod compressed;
pub mod csr;
pub mod decode;
pub mod hart;
pub mod timing;
pub mod trap;

#[cfg(test)]
mod tests;

pub use bus::{AccessKind, Bus, BusError, Ram, Size};
pub use decode::{Instr, InstrClass, decode};
pub use hart::{
    EbreakAction, Extensions, HaltReason, Hart, HartConfig, HartState, MisalignedPolicy, RunExit, StepOutcome,
};
pub use timing::TimingModel;
pub use trap::{Exception, TrapCause};
