//! Machine-mode control and status registers.

/// CSR addresses implemented by the core.
pub mod addr {
    #![allow(missing_docs)]
    pub const MSTATUS: u16 = 0x300;
    pub const MISA: u16 = 0x301;
    pub const MIE: u16 = 0x304;
    pub const MTVEC: u16 = 0x305;
    pub const MSTATUSH: u16 = 0x310;
    pub const MCOUNTINHIBIT: u16 = 0x320;
    pub const MSCRATCH: u16 = 0x340;
    pub const MEPC: u16 = 0x341;
    pub const MCAUSE: u16 = 0x342;
    pub const MTVAL: u16 = 0x343;
    pub const MIP: u16 = 0x344;
    pub const MCYCLE: u16 = 0xB00;
    pub const MINSTRET: u16 = 0xB02;
    pub const MCYCLEH: u16 = 0xB80;
    pub const MINSTRETH: u16 = 0xB82;
    pub const CYCLE: u16 = 0xC00;
    pub const INSTRET: u16 = 0xC02;
    pub const CYCLEH: u16 = 0xC80;
    pub const INSTRETH: u16 = 0xC82;
    pub const MVENDORID: u16 = 0xF11;
    pub const MARCHID: u16 = 0xF12;
    pub const MIMPID: u16 = 0xF13;
    pub const MHARTID: u16 = 0xF14;
    pub const MCONFIGPTR: u16 = 0xF15;
}

/// `mstatus.MIE`
pub const MSTATUS_MIE: u32 = 1 << 3;
/// `mstatus.MPIE`
pub const MSTATUS_MPIE: u32 = 1 << 7;
/// `mstatus.MPP` (hard-wired to machine mode: no U-mode yet).
pub const MSTATUS_MPP: u32 = 3 << 11;

/// The machine-mode CSR file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsrFile {
    /// `mstatus` (only MIE and MPIE are writable).
    pub mstatus: u32,
    /// `misa` (read-only, derived from the configured extensions).
    pub misa: u32,
    /// `mie`
    pub mie: u32,
    /// `mip` (driven by the SoC through the hart's interrupt API).
    pub mip: u32,
    /// `mtvec` (base plus mode 0 = direct, 1 = vectored).
    pub mtvec: u32,
    /// `mscratch`
    pub mscratch: u32,
    /// `mepc`
    pub mepc: u32,
    /// `mcause`
    pub mcause: u32,
    /// `mtval`
    pub mtval: u32,
    /// `mcycle` (64-bit).
    pub mcycle: u64,
    /// `minstret` (64-bit).
    pub minstret: u64,
    /// `mcountinhibit` (bit 0 cycle, bit 2 instret).
    pub mcountinhibit: u32,
    /// `mhartid`
    pub mhartid: u32,
    /// `mvendorid`
    pub mvendorid: u32,
    /// `marchid`
    pub marchid: u32,
    /// `mimpid`
    pub mimpid: u32,
    /// Writable `mie`/`mip` bits (which interrupt lines exist).
    pub irq_mask: u32,
    /// Whether `mie` and `mip` are implemented (see
    /// [`crate::HartConfig::mie_mip`]); when not, accessing them goes to the
    /// bus and is normally an illegal instruction.
    pub mie_mip: bool,
    /// Set when an instruction wrote `mcycle[h]`/`minstret[h]` (suppresses
    /// that instruction's own increment, as the spec requires).
    pub(crate) counter_written: u8,
}

/// Result of a CSR access attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsrAccess {
    /// Handled with this (old) value.
    Ok(u32),
    /// Not implemented by the core; ask the bus.
    Unknown,
    /// Exists but the access is illegal (write to a read-only CSR).
    Illegal,
}

impl CsrFile {
    /// Reset state.
    pub fn new(misa: u32, hart_id: u32, irq_mask: u32) -> Self {
        Self {
            mstatus: MSTATUS_MPP,
            misa,
            mie: 0,
            mip: 0,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mcycle: 0,
            minstret: 0,
            mcountinhibit: 0,
            mhartid: hart_id,
            mvendorid: 0,
            marchid: 0,
            mimpid: 0,
            irq_mask,
            mie_mip: true,
            counter_written: 0,
        }
    }

    fn c_enabled(&self) -> bool {
        self.misa & (1 << 2) != 0
    }

    /// Read without side effects. `None` if the core does not implement it.
    pub fn peek(&self, csr: u16) -> Option<u32> {
        use addr::*;
        if !self.mie_mip && matches!(csr, MIE | MIP) {
            return None;
        }
        Some(match csr {
            MSTATUS => self.mstatus,
            MISA => self.misa,
            MIE => self.mie,
            MTVEC => self.mtvec,
            MSTATUSH | MCONFIGPTR => 0,
            MCOUNTINHIBIT => self.mcountinhibit,
            MSCRATCH => self.mscratch,
            MEPC => {
                if self.c_enabled() {
                    self.mepc
                } else {
                    self.mepc & !3
                }
            }
            MCAUSE => self.mcause,
            MTVAL => self.mtval,
            MIP => self.mip,
            MCYCLE | CYCLE => self.mcycle as u32,
            MCYCLEH | CYCLEH => (self.mcycle >> 32) as u32,
            MINSTRET | INSTRET => self.minstret as u32,
            MINSTRETH | INSTRETH => (self.minstret >> 32) as u32,
            MVENDORID => self.mvendorid,
            MARCHID => self.marchid,
            MIMPID => self.mimpid,
            MHARTID => self.mhartid,
            _ => return None,
        })
    }

    /// Write with WARL masking. Returns `false` if the CSR is not
    /// implemented by the core.
    pub fn poke(&mut self, csr: u16, v: u32) -> bool {
        use addr::*;
        if !self.mie_mip && matches!(csr, MIE | MIP) {
            return false;
        }
        match csr {
            MSTATUS => self.mstatus = (v & (MSTATUS_MIE | MSTATUS_MPIE)) | MSTATUS_MPP,
            MISA | MSTATUSH => {} // WARL: read-only in this implementation
            MIE => self.mie = v & self.irq_mask,
            MIP => {}                     // lines are driven by the SoC, not by software
            MTVEC => self.mtvec = v & !2, // modes 0 (direct) and 1 (vectored)
            MCOUNTINHIBIT => self.mcountinhibit = v & 0b101,
            MSCRATCH => self.mscratch = v,
            MEPC => self.mepc = v & !1,
            MCAUSE => self.mcause = v,
            MTVAL => self.mtval = v,
            MCYCLE => {
                self.mcycle = (self.mcycle & !0xffff_ffff) | u64::from(v);
                self.counter_written |= 1;
            }
            MCYCLEH => {
                self.mcycle = (self.mcycle & 0xffff_ffff) | (u64::from(v) << 32);
                self.counter_written |= 1;
            }
            MINSTRET => {
                self.minstret = (self.minstret & !0xffff_ffff) | u64::from(v);
                self.counter_written |= 4;
            }
            MINSTRETH => {
                self.minstret = (self.minstret & 0xffff_ffff) | (u64::from(v) << 32);
                self.counter_written |= 4;
            }
            _ => return false,
        }
        true
    }

    /// Architectural access check for an instruction that may write.
    pub fn access(&self, csr: u16, writes: bool) -> CsrAccess {
        match self.peek(csr) {
            None => CsrAccess::Unknown,
            Some(_) if writes && (csr >> 10) == 3 => CsrAccess::Illegal,
            Some(v) => CsrAccess::Ok(v),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warl_fields() {
        let mut c = CsrFile::new(0x4000_1104, 0, 0x888);
        c.poke(addr::MSTATUS, 0xffff_ffff);
        assert_eq!(c.mstatus, MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP);
        c.poke(addr::MIE, 0xffff_ffff);
        assert_eq!(c.mie, 0x888);
        c.poke(addr::MTVEC, 0x4000_0003);
        assert_eq!(c.mtvec, 0x4000_0001);
        c.poke(addr::MEPC, 0x4000_0003);
        assert_eq!(c.peek(addr::MEPC), Some(0x4000_0002));
        assert_eq!(c.access(addr::MHARTID, true), CsrAccess::Illegal);
        assert_eq!(c.access(0x7c0, false), CsrAccess::Unknown);
    }

    #[test]
    fn mie_and_mip_can_be_absent() {
        let mut c = CsrFile::new(0, 0, 0xffff_fffe);
        c.mie_mip = false;
        assert_eq!(c.peek(addr::MIE), None);
        assert_eq!(c.peek(addr::MIP), None);
        assert!(!c.poke(addr::MIE, 1 << 5));
        assert_eq!(c.access(addr::MIE, false), CsrAccess::Unknown);
        assert_eq!(c.access(addr::MSTATUS, false), CsrAccess::Ok(MSTATUS_MPP));
    }

    #[test]
    fn counters_are_64_bit() {
        let mut c = CsrFile::new(0, 0, 0);
        c.poke(addr::MCYCLEH, 1);
        c.poke(addr::MCYCLE, 2);
        assert_eq!(c.mcycle, (1 << 32) | 2);
        assert_eq!(c.peek(addr::CYCLEH), Some(1));
    }
}
