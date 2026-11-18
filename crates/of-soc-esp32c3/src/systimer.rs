//! SYSTIMER (TRM chapter 10): two 52-bit counters at 16 MHz and three
//! comparators that raise interrupts.
//!
//! - Counters (units 0 and 1) run while `CONF.TIMER_UNITn_WORK_EN` is set
//!   (unit 0 runs from reset). Firmware reads one by writing
//!   `UNITn_OP.UPDATE`, polling `UNITn_OP.VALUE_VALID`, then reading
//!   `UNITn_VALUE_HI/LO`; `UNITn_LOAD_HI/LO` plus `UNITn_LOAD` set it.
//! - Comparators 0 to 2 compare against the unit picked by
//!   `TARGETn_CONF.TIMER_UNIT_SEL`. In target mode an alarm fires once when
//!   the count is greater than or equal to `TARGETn_HI/LO` (a target in the
//!   past fires at once). In period mode (`TARGETn_CONF.PERIOD_MODE`) the
//!   first alarm comes `PERIOD` ticks after the comparator was armed and
//!   then every `PERIOD` ticks, without drift. Target and period values take
//!   effect when `COMPn_LOAD` is written; the mode and unit bits act at
//!   once. A comparator is armed by `COMPn_LOAD`, by `CONF.TARGETn_WORK_EN`
//!   going high, or by a change of its mode or unit while working.
//! - Each alarm sets `INT_RAW` bit n; `INT_ST = INT_RAW & INT_ENA` drives the
//!   level-type interrupt sources `SYSTIMER_TARGET0..2` (37 to 39);
//!   `INT_CLR` (or writing 1 to `INT_RAW`) clears.
//!
//! All counting is aligned to the global 16 MHz grid (tick `n` starts at
//! `n * 62.5 ns`), so unit 0 reads `t / 62.5 ns` when it was never loaded.
//! Writes are applied with the time of the instruction that made them
//! ([`Systimer::write`]); the SoC calls [`Systimer::advance`] before that and
//! uses [`Systimer::next_event`] to stop the hart at the next alarm. The
//! core-stall bits and 52-bit wrap-around are not modelled.

use of_core::SimTime;

/// Counter rate: XTAL 40 MHz divided by 2.5.
pub const TICK_HZ: u64 = 16_000_000;
/// One counter tick in picoseconds.
pub const TICK_PS: u64 = 1_000_000_000_000 / TICK_HZ;
const MASK52: u64 = (1 << 52) - 1;

/// Register offsets from [`crate::map::SYSTIMER_BASE`].
pub mod reg {
    /// Clock and enable bits (see [`super::conf`]).
    pub const CONF: u32 = 0x00;
    /// Unit 0 operation: bit 30 `UPDATE` (write), bit 29 `VALUE_VALID`.
    pub const UNIT0_OP: u32 = 0x04;
    /// Unit 1 operation.
    pub const UNIT1_OP: u32 = 0x08;
    /// Unit 0 load value, bits 51:32 (unit 1 at +8).
    pub const UNIT0_LOAD_HI: u32 = 0x0c;
    /// Unit 0 load value, bits 31:0 (unit 1 at +8).
    pub const UNIT0_LOAD_LO: u32 = 0x10;
    /// Comparator 0 target, bits 51:32 (comparator n at +8n).
    pub const TARGET0_HI: u32 = 0x1c;
    /// Comparator 0 target, bits 31:0 (comparator n at +8n).
    pub const TARGET0_LO: u32 = 0x20;
    /// Comparator 0 configuration (comparator n at +4n): bits 25:0
    /// `PERIOD`, bit 30 `PERIOD_MODE`, bit 31 `TIMER_UNIT_SEL`.
    pub const TARGET0_CONF: u32 = 0x34;
    /// Latched unit 0 value, bits 51:32.
    pub const UNIT0_VALUE_HI: u32 = 0x40;
    /// Latched unit 0 value, bits 31:0.
    pub const UNIT0_VALUE_LO: u32 = 0x44;
    /// Latched unit 1 value, bits 51:32.
    pub const UNIT1_VALUE_HI: u32 = 0x48;
    /// Latched unit 1 value, bits 31:0.
    pub const UNIT1_VALUE_LO: u32 = 0x4c;
    /// Write bit 0 to load comparator 0's target and period (n at +4n).
    pub const COMP0_LOAD: u32 = 0x50;
    /// Write bit 0 to load unit 0 from `UNIT0_LOAD_HI/LO` (unit 1 at +4).
    pub const UNIT0_LOAD: u32 = 0x5c;
    /// Interrupt enable, bit n = comparator n.
    pub const INT_ENA: u32 = 0x64;
    /// Raw interrupt status (write 1 to clear).
    pub const INT_RAW: u32 = 0x68;
    /// Interrupt clear (write 1).
    pub const INT_CLR: u32 = 0x6c;
    /// Masked interrupt status.
    pub const INT_ST: u32 = 0x70;
}

/// `CONF` bits.
pub mod conf {
    /// Comparator n works when bit `TARGET0_WORK_EN - n` is set.
    pub const TARGET0_WORK_EN: u32 = 1 << 24;
    /// Unit 1 counts.
    pub const UNIT1_WORK_EN: u32 = 1 << 29;
    /// Unit 0 counts.
    pub const UNIT0_WORK_EN: u32 = 1 << 30;
    /// Reset value: unit 0 working, unit 1 core-stall bits set.
    pub const RESET: u32 = UNIT0_WORK_EN | (1 << 26) | (1 << 25);
}

const PERIOD_MODE: u32 = 1 << 30;
const UNIT_SEL: u32 = 1 << 31;
const PERIOD_MASK: u32 = (1 << 26) - 1;

/// A counter: `value` at global tick `tick`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Unit {
    value: u64,
    tick: u64,
}

/// An active comparator (the values loaded by `COMPn_LOAD`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Comp {
    target: u64,
    period: u32,
    armed: bool,
    /// Unit count at which the next alarm fires.
    fire_at: u64,
}

/// SYSTIMER state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Systimer {
    conf: u32,
    units: [Unit; 2],
    load: [u64; 2],
    latched: [u64; 2],
    valid: [bool; 2],
    target: [u64; 3],
    tconf: [u32; 3],
    comps: [Comp; 3],
    int_ena: u32,
    int_raw: u32,
}

impl Default for Systimer {
    fn default() -> Self {
        Self::new()
    }
}

/// Global 16 MHz tick at time `t`.
fn tick_at(t: SimTime) -> u64 {
    t.0 / TICK_PS
}

impl Systimer {
    /// Reset state.
    pub fn new() -> Self {
        Self {
            conf: conf::RESET,
            units: [Unit::default(); 2],
            load: [0; 2],
            latched: [0; 2],
            valid: [false; 2],
            target: [0; 3],
            tconf: [0; 3],
            comps: [Comp::default(); 3],
            int_ena: 0,
            int_raw: 0,
        }
    }

    /// Unit 0 value at `t` if it was never stopped or loaded.
    pub fn ticks_at(t: SimTime) -> u64 {
        tick_at(t) & MASK52
    }

    fn running(&self, unit: usize) -> bool {
        let bit = if unit == 0 {
            conf::UNIT0_WORK_EN
        } else {
            conf::UNIT1_WORK_EN
        };
        self.conf & bit != 0
    }

    /// Count of `unit` at global tick `k` (not before its anchor).
    fn count(&self, unit: usize, k: u64) -> u64 {
        let u = self.units[unit];
        if self.running(unit) {
            (u.value + k.saturating_sub(u.tick)) & MASK52
        } else {
            u.value
        }
    }

    /// Current count of `unit` at `t`.
    pub fn value(&self, unit: usize, t: SimTime) -> u64 {
        self.count(unit, tick_at(t))
    }

    fn comp_unit(&self, n: usize) -> usize {
        usize::from(self.tconf[n] & UNIT_SEL != 0)
    }

    fn working(&self, n: usize) -> bool {
        self.conf & (conf::TARGET0_WORK_EN >> n) != 0
    }

    /// Global tick at which comparator `n` fires next, if ever.
    fn fire_tick(&self, n: usize) -> Option<u64> {
        let c = self.comps[n];
        let unit = self.comp_unit(n);
        if !c.armed || !self.working(n) || !self.running(unit) {
            return None;
        }
        let u = self.units[unit];
        Some(if c.fire_at <= u.value {
            u.tick
        } else {
            u.tick + (c.fire_at - u.value)
        })
    }

    fn arm(&mut self, n: usize, k: u64) {
        let unit = self.comp_unit(n);
        let now = self.count(unit, k);
        let c = &mut self.comps[n];
        if self.tconf[n] & PERIOD_MODE != 0 {
            c.armed = c.period != 0;
            c.fire_at = now + u64::from(c.period);
        } else {
            c.armed = true;
            c.fire_at = c.target;
        }
    }

    /// Time of the next alarm, if any comparator is armed.
    pub fn next_event(&self) -> Option<SimTime> {
        (0..3)
            .filter_map(|n| self.fire_tick(n))
            .min()
            .map(|k| SimTime(k * TICK_PS))
    }

    /// Fire every alarm due at or before `t`.
    pub fn advance(&mut self, t: SimTime) {
        let k = tick_at(t);
        while let Some((_, n)) = (0..3)
            .filter_map(|n| self.fire_tick(n).map(|f| (f, n)))
            .filter(|&(f, _)| f <= k)
            .min()
        {
            self.int_raw |= 1 << n;
            let c = &mut self.comps[n];
            if self.tconf[n] & PERIOD_MODE != 0 {
                c.fire_at += u64::from(c.period);
            } else {
                c.armed = false;
            }
        }
    }

    /// `INT_ST`: which comparators assert their interrupt source.
    pub fn int_status(&self) -> u32 {
        self.int_raw & self.int_ena
    }

    /// Whether `off` is a SYSTIMER register.
    pub fn is_reg(off: u32) -> bool {
        off <= reg::INT_ST && off.is_multiple_of(4)
    }

    /// Read a register.
    pub fn read(&self, off: u32) -> Option<u32> {
        let unit = |o: u32, base: u32, stride: u32| ((o - base) / stride) as usize;
        Some(match off {
            reg::CONF => self.conf,
            reg::UNIT0_OP | reg::UNIT1_OP => u32::from(self.valid[unit(off, reg::UNIT0_OP, 4)]) << 29,
            0x0c | 0x14 => (self.load[unit(off, 0x0c, 8)] >> 32) as u32,
            0x10 | 0x18 => self.load[unit(off, 0x10, 8)] as u32,
            0x1c | 0x24 | 0x2c => (self.target[unit(off, 0x1c, 8)] >> 32) as u32,
            0x20 | 0x28 | 0x30 => self.target[unit(off, 0x20, 8)] as u32,
            0x34 | 0x38 | 0x3c => self.tconf[unit(off, 0x34, 4)],
            reg::UNIT0_VALUE_HI | reg::UNIT1_VALUE_HI => (self.latched[unit(off, 0x40, 8)] >> 32) as u32,
            reg::UNIT0_VALUE_LO | reg::UNIT1_VALUE_LO => self.latched[unit(off, 0x44, 8)] as u32,
            0x50..=0x60 => 0,
            reg::INT_ENA => self.int_ena,
            reg::INT_RAW => self.int_raw,
            reg::INT_CLR => 0,
            reg::INT_ST => self.int_status(),
            _ => return None,
        })
    }

    /// Write a register at time `t` (call [`Self::advance`] to `t` first).
    pub fn write(&mut self, off: u32, v: u32, t: SimTime) -> Option<()> {
        let k = tick_at(t);
        match off {
            reg::CONF => {
                let old = self.conf;
                // Re-anchor units whose run state changes.
                for unit in 0..2 {
                    let value = self.count(unit, k);
                    self.units[unit] = Unit { value, tick: k };
                }
                self.conf = v;
                for n in 0..3 {
                    let bit = conf::TARGET0_WORK_EN >> n;
                    if v & bit != 0 && old & bit == 0 {
                        self.arm(n, k);
                    }
                }
            }
            reg::UNIT0_OP | reg::UNIT1_OP if v & (1 << 30) != 0 => {
                let unit = ((off - reg::UNIT0_OP) / 4) as usize;
                self.latched[unit] = self.count(unit, k);
                self.valid[unit] = true;
            }
            reg::UNIT0_OP | reg::UNIT1_OP => {}
            0x0c | 0x14 => {
                let u = ((off - 0x0c) / 8) as usize;
                self.load[u] = (self.load[u] & 0xffff_ffff) | (u64::from(v & 0xf_ffff) << 32);
            }
            0x10 | 0x18 => {
                let u = ((off - 0x10) / 8) as usize;
                self.load[u] = (self.load[u] & !0xffff_ffff) | u64::from(v);
            }
            0x1c | 0x24 | 0x2c => {
                let n = ((off - 0x1c) / 8) as usize;
                self.target[n] = (self.target[n] & 0xffff_ffff) | (u64::from(v & 0xf_ffff) << 32);
            }
            0x20 | 0x28 | 0x30 => {
                let n = ((off - 0x20) / 8) as usize;
                self.target[n] = (self.target[n] & !0xffff_ffff) | u64::from(v);
            }
            0x34 | 0x38 | 0x3c => {
                let n = ((off - 0x34) / 4) as usize;
                let old = self.tconf[n];
                self.tconf[n] = v & (PERIOD_MODE | UNIT_SEL | PERIOD_MASK);
                if (old ^ v) & (PERIOD_MODE | UNIT_SEL) != 0 && self.working(n) {
                    self.arm(n, k);
                }
            }
            reg::UNIT0_VALUE_HI..=reg::UNIT1_VALUE_LO => {}
            0x50 | 0x54 | 0x58 => {
                let n = ((off - 0x50) / 4) as usize;
                if v & 1 != 0 {
                    self.comps[n].target = self.target[n];
                    self.comps[n].period = self.tconf[n] & PERIOD_MASK;
                    if self.working(n) {
                        self.arm(n, k);
                    }
                }
            }
            0x5c | 0x60 => {
                let u = ((off - 0x5c) / 4) as usize;
                if v & 1 != 0 {
                    self.units[u] = Unit {
                        value: self.load[u],
                        tick: k,
                    };
                }
            }
            reg::INT_ENA => self.int_ena = v & 7,
            reg::INT_RAW | reg::INT_CLR => self.int_raw &= !v,
            reg::INT_ST => {}
            _ => return None,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const US: u64 = 1_000_000;

    fn at_us(us: u64) -> SimTime {
        SimTime(us * US)
    }

    /// Write at `t` the way the SoC does: advance first.
    fn w(s: &mut Systimer, off: u32, v: u32, t: SimTime) {
        s.advance(t);
        s.write(off, v, t).unwrap();
    }

    #[test]
    fn update_latches_at_the_given_time() {
        let mut s = Systimer::new();
        assert_eq!(s.read(reg::CONF), Some(0x4600_0000), "reset value");
        w(&mut s, reg::UNIT0_OP, 1 << 30, SimTime::from_us(1000));
        assert_eq!(s.read(reg::UNIT0_OP), Some(1 << 29));
        assert_eq!(s.read(reg::UNIT0_VALUE_LO), Some(16_000));
        assert_eq!(s.read(reg::UNIT0_VALUE_HI), Some(0));
        s.advance(SimTime::from_us(2000));
        assert_eq!(s.read(reg::UNIT0_VALUE_LO), Some(16_000), "no new update requested");
        // Unit 1 is stopped at reset.
        w(&mut s, reg::UNIT1_OP, 1 << 30, SimTime::from_us(2000));
        assert_eq!(s.read(reg::UNIT1_VALUE_LO), Some(0));
    }

    #[test]
    fn units_load_stop_and_resume() {
        let mut s = Systimer::new();
        w(&mut s, reg::UNIT0_LOAD_HI, 0x1, at_us(10));
        w(&mut s, reg::UNIT0_LOAD_LO, 0x10, at_us(10));
        w(&mut s, reg::UNIT0_LOAD, 1, at_us(10));
        assert_eq!(s.value(0, at_us(20)), (1 << 32) + 0x10 + 160);
        // Stop at 20 µs, restart at 30 µs: 10 µs not counted.
        w(&mut s, reg::CONF, 0, at_us(20));
        assert_eq!(s.value(0, at_us(25)), (1 << 32) + 0x10 + 160);
        w(&mut s, reg::CONF, conf::UNIT0_WORK_EN | conf::UNIT1_WORK_EN, at_us(30));
        assert_eq!(s.value(0, at_us(31)), (1 << 32) + 0x10 + 176);
        assert_eq!(s.value(1, at_us(31)), 16, "unit 1 counts once enabled");
        assert_eq!(s.read(reg::UNIT0_LOAD_HI), Some(1));
    }

    #[test]
    fn target_mode_fires_once_at_the_target() {
        let mut s = Systimer::new();
        let t0 = at_us(100);
        w(&mut s, reg::TARGET0_LO, 16_000 + 3200, t0); // 1000 µs + 200 µs
        w(&mut s, reg::COMP0_LOAD, 1, t0);
        assert_eq!(s.next_event(), None, "comparator not working yet");
        w(&mut s, reg::INT_ENA, 1, t0);
        w(&mut s, reg::CONF, conf::RESET | conf::TARGET0_WORK_EN, t0);
        assert_eq!(s.next_event(), Some(at_us(1200)));
        s.advance(at_us(1199));
        assert_eq!(s.int_status(), 0);
        s.advance(at_us(1200));
        assert_eq!(s.int_status(), 1);
        assert_eq!(s.read(reg::INT_RAW), Some(1));
        assert_eq!(s.next_event(), None, "one-shot");
        w(&mut s, reg::INT_CLR, 1, at_us(1300));
        assert_eq!(s.int_status(), 0);
        s.advance(at_us(5000));
        assert_eq!(s.int_status(), 0, "does not fire again");
    }

    #[test]
    fn target_in_the_past_fires_immediately() {
        let mut s = Systimer::new();
        let t = at_us(500);
        w(&mut s, reg::CONF, conf::RESET | conf::TARGET0_WORK_EN >> 2, t);
        w(&mut s, 0x30, 100, t); // TARGET2_LO
        w(&mut s, 0x58, 1, t); // COMP2_LOAD
        assert_eq!(s.next_event(), Some(t));
        s.advance(t);
        assert_eq!(s.read(reg::INT_RAW), Some(1 << 2));
        assert_eq!(s.int_status(), 0, "not enabled");
    }

    #[test]
    fn period_mode_repeats_without_drift() {
        let mut s = Systimer::new();
        let t0 = SimTime(1_234_567); // mid-tick: 19 ticks + a bit
        w(&mut s, reg::TARGET0_CONF + 4, PERIOD_MODE | 4000, t0); // comparator 1, 250 µs
        w(&mut s, reg::COMP0_LOAD + 4, 1, t0);
        w(&mut s, reg::INT_ENA, 2, t0);
        w(&mut s, reg::CONF, conf::RESET | (conf::TARGET0_WORK_EN >> 1), t0);
        let first = (19 + 4000) * TICK_PS;
        assert_eq!(s.next_event(), Some(SimTime(first)));
        for n in 0..5 {
            let due = SimTime(first + n * 4000 * TICK_PS);
            assert_eq!(s.next_event(), Some(due));
            // The hart may overshoot by an instruction: the next alarm
            // still keeps the original grid.
            s.advance(SimTime(due.0 + 6250));
            assert_eq!(s.int_status(), 2);
            w(&mut s, reg::INT_CLR, 2, SimTime(due.0 + 12_500));
        }
        // Disabling the comparator stops it.
        w(&mut s, reg::CONF, conf::RESET, SimTime(first * 3));
        assert_eq!(s.next_event(), None);
    }

    #[test]
    fn mode_toggle_rearms_and_unit_select_follows_unit1() {
        let mut s = Systimer::new();
        let t = at_us(10);
        w(
            &mut s,
            reg::CONF,
            conf::RESET | conf::UNIT1_WORK_EN | conf::TARGET0_WORK_EN,
            t,
        );
        w(&mut s, reg::TARGET0_CONF, UNIT_SEL | 160, t);
        w(&mut s, reg::COMP0_LOAD, 1, t);
        // Target mode, target 0 on unit 1: already reached.
        assert_eq!(s.next_event(), Some(t));
        s.advance(t);
        w(&mut s, reg::INT_CLR, 1, t);
        // Switch to period mode at 20 µs: first alarm 10 µs later.
        w(&mut s, reg::TARGET0_CONF, UNIT_SEL | PERIOD_MODE | 160, at_us(20));
        assert_eq!(s.next_event(), Some(at_us(30)));
        // Stopping unit 1 stops its comparator.
        w(&mut s, reg::CONF, conf::RESET | conf::TARGET0_WORK_EN, at_us(25));
        assert_eq!(s.next_event(), None);
        // Period 0 never fires.
        w(&mut s, reg::TARGET0_CONF, PERIOD_MODE, at_us(26));
        w(&mut s, reg::COMP0_LOAD, 1, at_us(26));
        assert_eq!(s.next_event(), None);
    }

    #[test]
    fn three_comparators_fire_in_time_order() {
        let mut s = Systimer::new();
        let t = SimTime::ZERO;
        for (n, target) in [(0u32, 300u32), (1, 100), (2, 200)] {
            w(&mut s, reg::TARGET0_LO + 8 * n, target, t);
            w(&mut s, reg::COMP0_LOAD + 4 * n, 1, t);
        }
        w(&mut s, reg::INT_ENA, 7, t);
        w(&mut s, reg::CONF, conf::RESET | (7 << 22), t);
        assert_eq!(s.next_event(), Some(SimTime(100 * TICK_PS)));
        s.advance(SimTime(250 * TICK_PS));
        assert_eq!(s.int_status(), 0b110);
        assert_eq!(s.next_event(), Some(SimTime(300 * TICK_PS)));
        assert!(Systimer::is_reg(reg::INT_ST) && !Systimer::is_reg(0xfc));
    }
}
