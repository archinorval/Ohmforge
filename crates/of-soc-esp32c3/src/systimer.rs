//! SYSTIMER unit 0 (TRM chapter 10): a 52-bit counter at 16 MHz.
//!
//! Firmware reads it by writing `UNIT0_OP.UPDATE`, polling
//! `UNIT0_OP.VALUE_VALID`, then reading `UNIT0_VALUE_LO/HI`. The bus
//! records the update request and asks for a sync; [`Systimer::latch`] is
//! then called with the hart's exact time. Alarms and interrupts come later.

use of_core::SimTime;

/// Counter rate: XTAL 40 MHz divided by 2.5.
pub const TICK_HZ: u64 = 16_000_000;
const TICK_PS: u64 = 1_000_000_000_000 / TICK_HZ;

/// Register offsets from [`crate::map::SYSTIMER_BASE`].
pub mod reg {
    /// Unit 0 operation: bit 30 `UPDATE` (write), bit 29 `VALUE_VALID`.
    pub const UNIT0_OP: u32 = 0x04;
    /// Latched value, bits 51:32.
    pub const UNIT0_VALUE_HI: u32 = 0x40;
    /// Latched value, bits 31:0.
    pub const UNIT0_VALUE_LO: u32 = 0x44;
}

/// SYSTIMER state.
#[derive(Debug, Clone, Default)]
pub struct Systimer {
    latched: u64,
    valid: bool,
    pending: bool,
}

impl Systimer {
    /// Counter value at `t` (counting from time zero).
    pub fn ticks_at(t: SimTime) -> u64 {
        (t.0 / TICK_PS) & ((1 << 52) - 1)
    }

    /// Read a register.
    pub fn read(&self, off: u32) -> Option<u32> {
        Some(match off {
            reg::UNIT0_OP => u32::from(self.valid) << 29,
            reg::UNIT0_VALUE_HI => (self.latched >> 32) as u32,
            reg::UNIT0_VALUE_LO => self.latched as u32,
            _ => return None,
        })
    }

    /// Write a register; returns whether a latch (sync) is needed.
    pub fn write(&mut self, off: u32, v: u32) -> Option<bool> {
        match off {
            reg::UNIT0_OP if v & (1 << 30) != 0 => {
                self.valid = false;
                self.pending = true;
                Some(true)
            }
            reg::UNIT0_OP => Some(false),
            _ => None,
        }
    }

    /// Complete a pending update at time `t`.
    pub fn latch(&mut self, t: SimTime) {
        if self.pending {
            self.pending = false;
            self.latched = Self::ticks_at(t);
            self.valid = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_latches_at_the_given_time() {
        let mut s = Systimer::default();
        assert_eq!(s.write(reg::UNIT0_OP, 1 << 30), Some(true));
        assert_eq!(s.read(reg::UNIT0_OP), Some(0));
        s.latch(SimTime::from_us(1000));
        assert_eq!(s.read(reg::UNIT0_OP), Some(1 << 29));
        assert_eq!(s.read(reg::UNIT0_VALUE_LO), Some(16_000));
        assert_eq!(s.read(reg::UNIT0_VALUE_HI), Some(0));
        s.latch(SimTime::from_us(2000));
        assert_eq!(s.read(reg::UNIT0_VALUE_LO), Some(16_000), "no new update requested");
    }
}
