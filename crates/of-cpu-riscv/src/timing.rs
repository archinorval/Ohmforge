//! Cycle cost model.
//!
//! The ESP32-C3 core is a 4-stage in-order pipeline. Espressif does not
//! publish per-instruction timings, so these defaults are estimates to be
//! calibrated against the hardware-in-the-loop rig (plan section 12). Bus
//! wait states (flash cache misses, peripheral latency) are added on top via
//! [`crate::Bus::wait_states`].

use crate::decode::InstrClass;

/// Cycles per instruction class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimingModel {
    /// ALU, LUI, AUIPC.
    pub alu: u32,
    /// Loads (before wait states).
    pub load: u32,
    /// Stores (before wait states).
    pub store: u32,
    /// Branch taken.
    pub branch_taken: u32,
    /// Branch not taken.
    pub branch_not_taken: u32,
    /// JAL, JALR.
    pub jump: u32,
    /// Multiplies.
    pub mul: u32,
    /// Divides and remainders.
    pub div: u32,
    /// Atomics (read-modify-write).
    pub atomic: u32,
    /// CSR accesses.
    pub csr: u32,
    /// Fences.
    pub fence: u32,
    /// ECALL, EBREAK, MRET, WFI.
    pub system: u32,
    /// Extra cycles to enter a trap handler.
    pub trap_entry: u32,
}

impl TimingModel {
    /// One cycle for everything: handy for tests and functional runs.
    pub const UNIT: Self = Self {
        alu: 1,
        load: 1,
        store: 1,
        branch_taken: 1,
        branch_not_taken: 1,
        jump: 1,
        mul: 1,
        div: 1,
        atomic: 1,
        csr: 1,
        fence: 1,
        system: 1,
        trap_entry: 1,
    };

    /// Estimated ESP32-C3 timings (uncalibrated).
    pub const ESP32C3_ESTIMATE: Self = Self {
        alu: 1,
        load: 2,
        store: 1,
        branch_taken: 3,
        branch_not_taken: 1,
        jump: 2,
        mul: 1,
        div: 17,
        atomic: 3,
        csr: 1,
        fence: 1,
        system: 3,
        trap_entry: 4,
    };

    /// Base cost of an instruction class (`taken` matters only for branches).
    pub fn cycles(&self, class: InstrClass, taken: bool) -> u32 {
        match class {
            InstrClass::Alu => self.alu,
            InstrClass::Load => self.load,
            InstrClass::Store => self.store,
            InstrClass::Branch if taken => self.branch_taken,
            InstrClass::Branch => self.branch_not_taken,
            InstrClass::Jump => self.jump,
            InstrClass::Mul => self.mul,
            InstrClass::Div => self.div,
            InstrClass::Atomic => self.atomic,
            InstrClass::Csr => self.csr,
            InstrClass::Fence => self.fence,
            InstrClass::System => self.system,
        }
    }
}

impl Default for TimingModel {
    fn default() -> Self {
        Self::ESP32C3_ESTIMATE
    }
}
