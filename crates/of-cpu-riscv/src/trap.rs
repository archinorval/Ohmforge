//! Exceptions, interrupts and their `mcause` codes.

/// Synchronous exceptions (machine mode only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exception {
    /// Jump or branch target not aligned to the instruction width.
    InstructionAddressMisaligned,
    /// Fetch from an address with no device.
    InstructionAccessFault,
    /// Illegal or unsupported instruction.
    IllegalInstruction,
    /// `ebreak` (when configured to trap rather than halt).
    Breakpoint,
    /// Misaligned load.
    LoadAddressMisaligned,
    /// Load from an address with no device.
    LoadAccessFault,
    /// Misaligned store or atomic.
    StoreAddressMisaligned,
    /// Store or atomic to an address with no device.
    StoreAccessFault,
    /// `ecall` from machine mode.
    EcallFromM,
}

impl Exception {
    /// The `mcause` exception code.
    pub const fn code(self) -> u32 {
        match self {
            Exception::InstructionAddressMisaligned => 0,
            Exception::InstructionAccessFault => 1,
            Exception::IllegalInstruction => 2,
            Exception::Breakpoint => 3,
            Exception::LoadAddressMisaligned => 4,
            Exception::LoadAccessFault => 5,
            Exception::StoreAddressMisaligned => 6,
            Exception::StoreAccessFault => 7,
            Exception::EcallFromM => 11,
        }
    }
}

/// What caused a trap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapCause {
    /// A synchronous exception, with the value written to `mtval`.
    Exception(Exception, u32),
    /// An interrupt on line 0 to 31 (`mip`/`mie` bit number).
    Interrupt(u8),
}

impl TrapCause {
    /// The value written to `mcause`.
    pub const fn mcause(self) -> u32 {
        match self {
            TrapCause::Exception(e, _) => e.code(),
            TrapCause::Interrupt(line) => 0x8000_0000 | line as u32,
        }
    }
}

/// Standard machine-level interrupt bit numbers.
pub mod irq {
    /// Machine software interrupt.
    pub const MSI: u8 = 3;
    /// Machine timer interrupt.
    pub const MTI: u8 = 7;
    /// Machine external interrupt.
    pub const MEI: u8 = 11;
}

/// Pick the interrupt to take from the pending-and-enabled set: the standard
/// order MEI, MSI, MTI first, then the lowest-numbered platform line. On the
/// ESP32-C3 the interrupt matrix applies priorities and thresholds before a
/// line reaches the core, so the SoC model only asserts lines that qualify.
pub fn select_interrupt(pending_enabled: u32) -> Option<u8> {
    if pending_enabled == 0 {
        return None;
    }
    for line in [irq::MEI, irq::MSI, irq::MTI] {
        if pending_enabled & (1 << line) != 0 {
            return Some(line);
        }
    }
    Some(pending_enabled.trailing_zeros() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcause_values_and_priority() {
        assert_eq!(TrapCause::Exception(Exception::EcallFromM, 0).mcause(), 11);
        assert_eq!(TrapCause::Interrupt(7).mcause(), 0x8000_0007);
        assert_eq!(select_interrupt(0), None);
        assert_eq!(select_interrupt((1 << 7) | (1 << 11)), Some(11));
        assert_eq!(select_interrupt((1 << 20) | (1 << 5)), Some(5));
        assert_eq!(select_interrupt((1 << 20) | (1 << 7)), Some(7));
    }
}
