//! 32-bit instruction decoder and disassembler (RV32I, M, A, Zicsr, Zifencei).
//!
//! Decoding is pure ISA: whether an extension is enabled is checked by the
//! hart at execution time, so the debugger can disassemble anything.

use core::fmt;

/// Integer register index (0 to 31).
pub type Reg = u8;

/// Register-register and register-immediate ALU operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AluOp {
    /// Addition.
    Add,
    /// Subtraction (register form only).
    Sub,
    /// Shift left logical.
    Sll,
    /// Set if less than (signed).
    Slt,
    /// Set if less than (unsigned).
    Sltu,
    /// Exclusive or.
    Xor,
    /// Shift right logical.
    Srl,
    /// Shift right arithmetic.
    Sra,
    /// Or.
    Or,
    /// And.
    And,
}

/// Multiply and divide operations (M extension).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MulOp {
    /// Low 32 bits of the product.
    Mul,
    /// High 32 bits, signed × signed.
    Mulh,
    /// High 32 bits, signed × unsigned.
    Mulhsu,
    /// High 32 bits, unsigned × unsigned.
    Mulhu,
    /// Signed division.
    Div,
    /// Unsigned division.
    Divu,
    /// Signed remainder.
    Rem,
    /// Unsigned remainder.
    Remu,
}

/// Conditional branch comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchOp {
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
    /// Less than (signed).
    Lt,
    /// Greater or equal (signed).
    Ge,
    /// Less than (unsigned).
    Ltu,
    /// Greater or equal (unsigned).
    Geu,
}

/// Load widths and extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOp {
    /// Signed byte.
    Lb,
    /// Signed half-word.
    Lh,
    /// Word.
    Lw,
    /// Unsigned byte.
    Lbu,
    /// Unsigned half-word.
    Lhu,
}

/// Store widths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOp {
    /// Byte.
    Sb,
    /// Half-word.
    Sh,
    /// Word.
    Sw,
}

/// Atomic operations (A extension, word size only on RV32).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmoOp {
    /// Load-reserved.
    Lr,
    /// Store-conditional.
    Sc,
    /// Swap.
    Swap,
    /// Add.
    Add,
    /// Xor.
    Xor,
    /// And.
    And,
    /// Or.
    Or,
    /// Signed minimum.
    Min,
    /// Signed maximum.
    Max,
    /// Unsigned minimum.
    Minu,
    /// Unsigned maximum.
    Maxu,
}

/// CSR access operations (Zicsr).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsrOp {
    /// Read and write.
    Rw,
    /// Read and set bits.
    Rs,
    /// Read and clear bits.
    Rc,
}

/// Source operand of a CSR instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsrSrc {
    /// A register (`csrrw` and friends).
    Reg(Reg),
    /// A 5-bit zero-extended immediate (`csrrwi` and friends).
    Imm(u8),
}

/// A decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)] // field names follow the ISA manual
pub enum Instr {
    Lui {
        rd: Reg,
        imm: u32,
    },
    Auipc {
        rd: Reg,
        imm: u32,
    },
    Jal {
        rd: Reg,
        offset: i32,
    },
    Jalr {
        rd: Reg,
        rs1: Reg,
        offset: i32,
    },
    Branch {
        op: BranchOp,
        rs1: Reg,
        rs2: Reg,
        offset: i32,
    },
    Load {
        op: LoadOp,
        rd: Reg,
        rs1: Reg,
        offset: i32,
    },
    Store {
        op: StoreOp,
        rs1: Reg,
        rs2: Reg,
        offset: i32,
    },
    OpImm {
        op: AluOp,
        rd: Reg,
        rs1: Reg,
        imm: i32,
    },
    Op {
        op: AluOp,
        rd: Reg,
        rs1: Reg,
        rs2: Reg,
    },
    MulDiv {
        op: MulOp,
        rd: Reg,
        rs1: Reg,
        rs2: Reg,
    },
    Amo {
        op: AmoOp,
        rd: Reg,
        rs1: Reg,
        rs2: Reg,
        aq: bool,
        rl: bool,
    },
    Csr {
        op: CsrOp,
        rd: Reg,
        csr: u16,
        src: CsrSrc,
    },
    Fence {
        pred: u8,
        succ: u8,
    },
    FenceI,
    Ecall,
    Ebreak,
    Mret,
    Wfi,
}

/// Broad instruction class, used by the timing model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrClass {
    /// ALU, LUI, AUIPC.
    Alu,
    /// Loads.
    Load,
    /// Stores.
    Store,
    /// Conditional branches.
    Branch,
    /// JAL and JALR.
    Jump,
    /// Multiplies.
    Mul,
    /// Divides and remainders.
    Div,
    /// Atomics.
    Atomic,
    /// CSR accesses.
    Csr,
    /// Fences.
    Fence,
    /// ECALL, EBREAK, MRET, WFI.
    System,
}

impl Instr {
    /// The timing class of this instruction.
    pub fn class(&self) -> InstrClass {
        match self {
            Instr::Lui { .. } | Instr::Auipc { .. } | Instr::OpImm { .. } | Instr::Op { .. } => InstrClass::Alu,
            Instr::Jal { .. } | Instr::Jalr { .. } => InstrClass::Jump,
            Instr::Branch { .. } => InstrClass::Branch,
            Instr::Load { .. } => InstrClass::Load,
            Instr::Store { .. } => InstrClass::Store,
            Instr::MulDiv { op, .. } => match op {
                MulOp::Mul | MulOp::Mulh | MulOp::Mulhsu | MulOp::Mulhu => InstrClass::Mul,
                _ => InstrClass::Div,
            },
            Instr::Amo { .. } => InstrClass::Atomic,
            Instr::Csr { .. } => InstrClass::Csr,
            Instr::Fence { .. } | Instr::FenceI => InstrClass::Fence,
            Instr::Ecall | Instr::Ebreak | Instr::Mret | Instr::Wfi => InstrClass::System,
        }
    }
}

fn sext(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

/// Decode a 32-bit instruction. Returns `None` for illegal or unsupported
/// encodings (including all F/D/V instructions).
pub fn decode(raw: u32) -> Option<Instr> {
    let opcode = raw & 0x7f;
    let rd = ((raw >> 7) & 31) as Reg;
    let f3 = (raw >> 12) & 7;
    let rs1 = ((raw >> 15) & 31) as Reg;
    let rs2 = ((raw >> 20) & 31) as Reg;
    let f7 = raw >> 25;
    let imm_i = (raw as i32) >> 20;
    let imm_s = ((raw as i32) >> 25 << 5) | ((raw >> 7) & 0x1f) as i32;
    let imm_b = sext(
        ((raw >> 31) << 12) | (((raw >> 7) & 1) << 11) | (((raw >> 25) & 0x3f) << 5) | (((raw >> 8) & 0xf) << 1),
        13,
    );
    let imm_j = sext(
        ((raw >> 31) << 20) | (((raw >> 12) & 0xff) << 12) | (((raw >> 20) & 1) << 11) | (((raw >> 21) & 0x3ff) << 1),
        21,
    );
    Some(match opcode {
        0x37 => Instr::Lui {
            rd,
            imm: raw & 0xffff_f000,
        },
        0x17 => Instr::Auipc {
            rd,
            imm: raw & 0xffff_f000,
        },
        0x6f => Instr::Jal { rd, offset: imm_j },
        0x67 if f3 == 0 => Instr::Jalr { rd, rs1, offset: imm_i },
        0x63 => {
            let op = match f3 {
                0 => BranchOp::Eq,
                1 => BranchOp::Ne,
                4 => BranchOp::Lt,
                5 => BranchOp::Ge,
                6 => BranchOp::Ltu,
                7 => BranchOp::Geu,
                _ => return None,
            };
            Instr::Branch {
                op,
                rs1,
                rs2,
                offset: imm_b,
            }
        }
        0x03 => {
            let op = match f3 {
                0 => LoadOp::Lb,
                1 => LoadOp::Lh,
                2 => LoadOp::Lw,
                4 => LoadOp::Lbu,
                5 => LoadOp::Lhu,
                _ => return None,
            };
            Instr::Load {
                op,
                rd,
                rs1,
                offset: imm_i,
            }
        }
        0x23 => {
            let op = match f3 {
                0 => StoreOp::Sb,
                1 => StoreOp::Sh,
                2 => StoreOp::Sw,
                _ => return None,
            };
            Instr::Store {
                op,
                rs1,
                rs2,
                offset: imm_s,
            }
        }
        0x13 => {
            let (op, imm) = match f3 {
                0 => (AluOp::Add, imm_i),
                2 => (AluOp::Slt, imm_i),
                3 => (AluOp::Sltu, imm_i),
                4 => (AluOp::Xor, imm_i),
                6 => (AluOp::Or, imm_i),
                7 => (AluOp::And, imm_i),
                1 if f7 == 0 => (AluOp::Sll, i32::from(rs2)),
                5 if f7 == 0 => (AluOp::Srl, i32::from(rs2)),
                5 if f7 == 0x20 => (AluOp::Sra, i32::from(rs2)),
                _ => return None,
            };
            Instr::OpImm { op, rd, rs1, imm }
        }
        0x33 => match f7 {
            0 | 0x20 => {
                let op = match (f7, f3) {
                    (0, 0) => AluOp::Add,
                    (0x20, 0) => AluOp::Sub,
                    (0, 1) => AluOp::Sll,
                    (0, 2) => AluOp::Slt,
                    (0, 3) => AluOp::Sltu,
                    (0, 4) => AluOp::Xor,
                    (0, 5) => AluOp::Srl,
                    (0x20, 5) => AluOp::Sra,
                    (0, 6) => AluOp::Or,
                    (0, 7) => AluOp::And,
                    _ => return None,
                };
                Instr::Op { op, rd, rs1, rs2 }
            }
            1 => {
                let op = [
                    MulOp::Mul,
                    MulOp::Mulh,
                    MulOp::Mulhsu,
                    MulOp::Mulhu,
                    MulOp::Div,
                    MulOp::Divu,
                    MulOp::Rem,
                    MulOp::Remu,
                ][f3 as usize];
                Instr::MulDiv { op, rd, rs1, rs2 }
            }
            _ => return None,
        },
        0x0f => match f3 {
            0 => Instr::Fence {
                pred: ((raw >> 24) & 0xf) as u8,
                succ: ((raw >> 20) & 0xf) as u8,
            },
            1 => Instr::FenceI,
            _ => return None,
        },
        0x73 => match f3 {
            0 => match raw {
                0x0000_0073 => Instr::Ecall,
                0x0010_0073 => Instr::Ebreak,
                0x3020_0073 => Instr::Mret,
                0x1050_0073 => Instr::Wfi,
                _ => return None,
            },
            4 => return None,
            _ => {
                let op = match f3 & 3 {
                    1 => CsrOp::Rw,
                    2 => CsrOp::Rs,
                    _ => CsrOp::Rc,
                };
                let src = if f3 & 4 != 0 {
                    CsrSrc::Imm(rs1)
                } else {
                    CsrSrc::Reg(rs1)
                };
                Instr::Csr {
                    op,
                    rd,
                    csr: (raw >> 20) as u16,
                    src,
                }
            }
        },
        0x2f if f3 == 2 => {
            let op = match raw >> 27 {
                0x02 if rs2 == 0 => AmoOp::Lr,
                0x03 => AmoOp::Sc,
                0x01 => AmoOp::Swap,
                0x00 => AmoOp::Add,
                0x04 => AmoOp::Xor,
                0x0c => AmoOp::And,
                0x08 => AmoOp::Or,
                0x10 => AmoOp::Min,
                0x14 => AmoOp::Max,
                0x18 => AmoOp::Minu,
                0x1c => AmoOp::Maxu,
                _ => return None,
            };
            Instr::Amo {
                op,
                rd,
                rs1,
                rs2,
                aq: raw & (1 << 26) != 0,
                rl: raw & (1 << 25) != 0,
            }
        }
        _ => return None,
    })
}

/// ABI register names, for disassembly.
pub const ABI_NAMES: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "s2",
    "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4", "t5", "t6",
];

fn n(r: Reg) -> &'static str {
    ABI_NAMES[usize::from(r & 31)]
}

impl fmt::Display for Instr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Instr::Lui { rd, imm } => write!(f, "lui {}, {:#x}", n(rd), imm >> 12),
            Instr::Auipc { rd, imm } => write!(f, "auipc {}, {:#x}", n(rd), imm >> 12),
            Instr::Jal { rd, offset } => write!(f, "jal {}, {}", n(rd), offset),
            Instr::Jalr { rd, rs1, offset } => write!(f, "jalr {}, {}({})", n(rd), offset, n(rs1)),
            Instr::Branch { op, rs1, rs2, offset } => {
                let m = match op {
                    BranchOp::Eq => "beq",
                    BranchOp::Ne => "bne",
                    BranchOp::Lt => "blt",
                    BranchOp::Ge => "bge",
                    BranchOp::Ltu => "bltu",
                    BranchOp::Geu => "bgeu",
                };
                write!(f, "{m} {}, {}, {}", n(rs1), n(rs2), offset)
            }
            Instr::Load { op, rd, rs1, offset } => {
                let m = match op {
                    LoadOp::Lb => "lb",
                    LoadOp::Lh => "lh",
                    LoadOp::Lw => "lw",
                    LoadOp::Lbu => "lbu",
                    LoadOp::Lhu => "lhu",
                };
                write!(f, "{m} {}, {}({})", n(rd), offset, n(rs1))
            }
            Instr::Store { op, rs1, rs2, offset } => {
                let m = match op {
                    StoreOp::Sb => "sb",
                    StoreOp::Sh => "sh",
                    StoreOp::Sw => "sw",
                };
                write!(f, "{m} {}, {}({})", n(rs2), offset, n(rs1))
            }
            Instr::OpImm { op, rd, rs1, imm } => {
                let m = match op {
                    AluOp::Add => "addi",
                    AluOp::Slt => "slti",
                    AluOp::Sltu => "sltiu",
                    AluOp::Xor => "xori",
                    AluOp::Or => "ori",
                    AluOp::And => "andi",
                    AluOp::Sll => "slli",
                    AluOp::Srl => "srli",
                    AluOp::Sra => "srai",
                    AluOp::Sub => "subi?",
                };
                write!(f, "{m} {}, {}, {}", n(rd), n(rs1), imm)
            }
            Instr::Op { op, rd, rs1, rs2 } => {
                let m = format!("{op:?}").to_lowercase();
                write!(f, "{m} {}, {}, {}", n(rd), n(rs1), n(rs2))
            }
            Instr::MulDiv { op, rd, rs1, rs2 } => {
                let m = format!("{op:?}").to_lowercase();
                write!(f, "{m} {}, {}, {}", n(rd), n(rs1), n(rs2))
            }
            Instr::Amo {
                op,
                rd,
                rs1,
                rs2,
                aq,
                rl,
            } => {
                let sfx = match (aq, rl) {
                    (true, true) => ".aqrl",
                    (true, false) => ".aq",
                    (false, true) => ".rl",
                    _ => "",
                };
                match op {
                    AmoOp::Lr => write!(f, "lr.w{sfx} {}, ({})", n(rd), n(rs1)),
                    AmoOp::Sc => write!(f, "sc.w{sfx} {}, {}, ({})", n(rd), n(rs2), n(rs1)),
                    _ => {
                        let m = format!("amo{op:?}").to_lowercase();
                        write!(f, "{m}.w{sfx} {}, {}, ({})", n(rd), n(rs2), n(rs1))
                    }
                }
            }
            Instr::Csr { op, rd, csr, src } => {
                let m = match op {
                    CsrOp::Rw => "csrrw",
                    CsrOp::Rs => "csrrs",
                    CsrOp::Rc => "csrrc",
                };
                match src {
                    CsrSrc::Reg(r) => write!(f, "{m} {}, {:#x}, {}", n(rd), csr, n(r)),
                    CsrSrc::Imm(v) => write!(f, "{m}i {}, {:#x}, {}", n(rd), csr, v),
                }
            }
            Instr::Fence { .. } => write!(f, "fence"),
            Instr::FenceI => write!(f, "fence.i"),
            Instr::Ecall => write!(f, "ecall"),
            Instr::Ebreak => write!(f, "ebreak"),
            Instr::Mret => write!(f, "mret"),
            Instr::Wfi => write!(f, "wfi"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::*;

    #[test]
    fn immediates_round_trip() {
        for off in [-4096, -2, 0, 2, 4094] {
            assert_eq!(
                decode(beq(1, 2, off)),
                Some(Instr::Branch {
                    op: BranchOp::Eq,
                    rs1: 1,
                    rs2: 2,
                    offset: off
                })
            );
        }
        for off in [-(1 << 20), -2, 2, (1 << 20) - 2] {
            assert_eq!(decode(jal(1, off)), Some(Instr::Jal { rd: 1, offset: off }));
        }
        for imm in [-2048, -1, 0, 1, 2047] {
            assert_eq!(
                decode(addi(5, 6, imm)),
                Some(Instr::OpImm {
                    op: AluOp::Add,
                    rd: 5,
                    rs1: 6,
                    imm
                })
            );
            assert_eq!(
                decode(sw(7, 8, imm)),
                Some(Instr::Store {
                    op: StoreOp::Sw,
                    rs1: 8,
                    rs2: 7,
                    offset: imm
                })
            );
            assert_eq!(
                decode(lw(7, 8, imm)),
                Some(Instr::Load {
                    op: LoadOp::Lw,
                    rd: 7,
                    rs1: 8,
                    offset: imm
                })
            );
        }
        assert_eq!(
            decode(lui(3, 0xdead_b000)),
            Some(Instr::Lui {
                rd: 3,
                imm: 0xdead_b000
            })
        );
    }

    #[test]
    fn shifts_mul_amo_csr_system() {
        assert_eq!(
            decode(srai(1, 2, 31)),
            Some(Instr::OpImm {
                op: AluOp::Sra,
                rd: 1,
                rs1: 2,
                imm: 31
            })
        );
        assert_eq!(
            decode(divu(1, 2, 3)),
            Some(Instr::MulDiv {
                op: MulOp::Divu,
                rd: 1,
                rs1: 2,
                rs2: 3
            })
        );
        assert_eq!(
            decode(amomaxu_w(1, 2, 3)),
            Some(Instr::Amo {
                op: AmoOp::Maxu,
                rd: 1,
                rs1: 2,
                rs2: 3,
                aq: false,
                rl: false
            })
        );
        assert_eq!(
            decode(csrrsi(4, 0x300, 8)),
            Some(Instr::Csr {
                op: CsrOp::Rs,
                rd: 4,
                csr: 0x300,
                src: CsrSrc::Imm(8)
            })
        );
        assert_eq!(decode(MRET), Some(Instr::Mret));
        assert_eq!(decode(WFI), Some(Instr::Wfi));
        assert_eq!(decode(FENCE_I), Some(Instr::FenceI));
    }

    #[test]
    fn rejects_illegal_encodings() {
        assert_eq!(decode(0), None);
        assert_eq!(decode(0xffff_ffff), None);
        assert_eq!(decode(0x0000_0007), None); // FLW (no F extension)
        assert_eq!(decode(slli(1, 2, 0) | (1 << 25)), None); // RV64-only shamt bit
        assert_eq!(decode(lr_w(1, 2) | (3 << 20)), None); // LR with rs2 != 0
        assert_eq!(decode(0x1050_0073 | (1 << 7)), None); // WFI with rd != 0
    }

    #[test]
    fn disassembles() {
        let d = |raw: u32| decode(raw).unwrap().to_string();
        assert_eq!(d(addi(10, 2, -16)), "addi a0, sp, -16");
        assert_eq!(d(sw(1, 2, 12)), "sw ra, 12(sp)");
        assert_eq!(d(sltu(5, 6, 7)), "sltu t0, t1, t2");
        assert_eq!(d(sc_w(10, 11, 12)), "sc.w a0, a2, (a1)");
        assert_eq!(d(amoadd_w(10, 11, 12)), "amoadd.w a0, a2, (a1)");
        assert_eq!(d(csrrw(0, 0x305, 5)), "csrrw zero, 0x305, t0");
        assert_eq!(d(mulhsu(1, 2, 3)), "mulhsu ra, sp, gp");
    }
}
