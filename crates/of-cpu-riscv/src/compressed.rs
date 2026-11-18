//! RVC (C extension) expander for RV32: each 16-bit instruction is rewritten
//! as its 32-bit equivalent, so one decoder and one executor serve both.
//! Floating-point forms (C.FLW, C.FSD, ...) and reserved encodings expand to
//! `None` (illegal instruction).

use crate::asm;

fn bit(x: u16, n: u32) -> u32 {
    (u32::from(x) >> n) & 1
}
fn field(x: u16, hi: u32, lo: u32) -> u32 {
    (u32::from(x) >> lo) & ((1 << (hi - lo + 1)) - 1)
}
fn sext(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}
/// Register from a 3-bit compressed field (x8 to x15).
fn creg(x: u16, lo: u32) -> u8 {
    (field(x, lo + 2, lo) + 8) as u8
}

/// Whether a 16-bit parcel starts a compressed instruction.
pub fn is_compressed(parcel: u16) -> bool {
    parcel & 3 != 3
}

/// Expand a compressed instruction to its 32-bit encoding.
pub fn expand(c: u16) -> Option<u32> {
    let rd = field(c, 11, 7) as u8;
    let rs2 = field(c, 6, 2) as u8;
    let imm6 = sext((bit(c, 12) << 5) | field(c, 6, 2), 6);
    let f3 = field(c, 15, 13);
    Some(match (c & 3, f3) {
        // Quadrant 0
        (0, 0) => {
            // C.ADDI4SPN
            let imm = (field(c, 12, 11) << 4) | (field(c, 10, 7) << 6) | (bit(c, 6) << 2) | (bit(c, 5) << 3);
            if imm == 0 {
                return None; // includes the all-zero illegal instruction
            }
            asm::addi(creg(c, 2), 2, imm as i32)
        }
        (0, 2) | (0, 6) => {
            let off = ((field(c, 12, 10) << 3) | (bit(c, 6) << 2) | (bit(c, 5) << 6)) as i32;
            if f3 == 2 {
                asm::lw(creg(c, 2), creg(c, 7), off) // C.LW
            } else {
                asm::sw(creg(c, 2), creg(c, 7), off) // C.SW
            }
        }
        // Quadrant 1
        (1, 0) => asm::addi(rd, rd, imm6), // C.ADDI / C.NOP (hints included)
        (1, 1) | (1, 5) => {
            // C.JAL / C.J
            let off = sext(
                (bit(c, 12) << 11)
                    | (bit(c, 11) << 4)
                    | (field(c, 10, 9) << 8)
                    | (bit(c, 8) << 10)
                    | (bit(c, 7) << 6)
                    | (bit(c, 6) << 7)
                    | (field(c, 5, 3) << 1)
                    | (bit(c, 2) << 5),
                12,
            );
            asm::jal(if f3 == 1 { 1 } else { 0 }, off)
        }
        (1, 2) => asm::addi(rd, 0, imm6), // C.LI
        (1, 3) if rd == 2 => {
            // C.ADDI16SP
            let imm = sext(
                (bit(c, 12) << 9) | (bit(c, 6) << 4) | (bit(c, 5) << 6) | (field(c, 4, 3) << 7) | (bit(c, 2) << 5),
                10,
            );
            if imm == 0 {
                return None;
            }
            asm::addi(2, 2, imm)
        }
        (1, 3) => {
            // C.LUI
            if imm6 == 0 {
                return None;
            }
            asm::lui(rd, (imm6 as u32) << 12)
        }
        (1, 4) => {
            let r = creg(c, 7);
            match field(c, 11, 10) {
                0 | 1 => {
                    if bit(c, 12) != 0 {
                        return None; // shamt[5] must be zero on RV32
                    }
                    let sh = field(c, 6, 2);
                    if field(c, 11, 10) == 0 {
                        asm::srli(r, r, sh)
                    } else {
                        asm::srai(r, r, sh)
                    }
                }
                2 => asm::andi(r, r, imm6),
                _ => {
                    if bit(c, 12) != 0 {
                        return None; // C.SUBW / C.ADDW are RV64-only
                    }
                    let r2 = creg(c, 2);
                    match field(c, 6, 5) {
                        0 => asm::sub(r, r, r2),
                        1 => asm::xor(r, r, r2),
                        2 => asm::or(r, r, r2),
                        _ => asm::and(r, r, r2),
                    }
                }
            }
        }
        (1, 6) | (1, 7) => {
            // C.BEQZ / C.BNEZ
            let off = sext(
                (bit(c, 12) << 8)
                    | (field(c, 11, 10) << 3)
                    | (field(c, 6, 5) << 6)
                    | (field(c, 4, 3) << 1)
                    | (bit(c, 2) << 5),
                9,
            );
            if f3 == 6 {
                asm::beq(creg(c, 7), 0, off)
            } else {
                asm::bne(creg(c, 7), 0, off)
            }
        }
        // Quadrant 2
        (2, 0) => {
            if bit(c, 12) != 0 {
                return None;
            }
            asm::slli(rd, rd, field(c, 6, 2)) // C.SLLI
        }
        (2, 2) => {
            // C.LWSP
            if rd == 0 {
                return None;
            }
            let off = (bit(c, 12) << 5) | (field(c, 6, 4) << 2) | (field(c, 3, 2) << 6);
            asm::lw(rd, 2, off as i32)
        }
        (2, 4) => match (bit(c, 12), rd, rs2) {
            (0, 0, 0) => return None,          // C.JR with rs1 = 0 is reserved
            (0, _, 0) => asm::jalr(0, rd, 0),  // C.JR
            (0, _, _) => asm::add(rd, 0, rs2), // C.MV
            (_, 0, 0) => asm::EBREAK,          // C.EBREAK
            (_, _, 0) => asm::jalr(1, rd, 0),  // C.JALR
            _ => asm::add(rd, rd, rs2),        // C.ADD
        },
        (2, 6) => {
            // C.SWSP
            let off = (field(c, 12, 9) << 2) | (field(c, 8, 7) << 6);
            asm::sw(rs2, 2, off as i32)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::*;

    #[test]
    fn expands_common_forms() {
        // Encodings checked against the RISC-V unprivileged spec, chapter "C".
        assert_eq!(expand(0x0001), Some(NOP)); // c.nop
        assert_eq!(expand(0x4505), Some(addi(10, 0, 1))); // c.li a0, 1
        assert_eq!(expand(0x1141), Some(addi(2, 2, -16))); // c.addi sp, -16
        assert_eq!(expand(0x0028), Some(addi(10, 2, 8))); // c.addi4spn a0, sp, 8
        assert_eq!(expand(0x4188), Some(lw(10, 11, 0))); // c.lw a0, 0(a1)
        assert_eq!(expand(0xc188), Some(sw(10, 11, 0))); // c.sw a0, 0(a1)
        assert_eq!(expand(0x8082), Some(jalr(0, 1, 0))); // c.jr ra (ret)
        assert_eq!(expand(0x852e), Some(add(10, 0, 11))); // c.mv a0, a1
        assert_eq!(expand(0x952e), Some(add(10, 10, 11))); // c.add a0, a1
        assert_eq!(expand(0x9002), Some(EBREAK)); // c.ebreak
        assert_eq!(expand(0x9582), Some(jalr(1, 11, 0))); // c.jalr a1
        assert_eq!(expand(0x6105), Some(addi(2, 2, 32))); // c.addi16sp sp, 32
        assert_eq!(expand(0x6505), Some(lui(10, 0x1000))); // c.lui a0, 1
        assert_eq!(expand(0x050a), Some(slli(10, 10, 2))); // c.slli a0, 2
        assert_eq!(expand(0x8505), Some(srai(10, 10, 1))); // c.srai a0, 1
        assert_eq!(expand(0x8105), Some(srli(10, 10, 1))); // c.srli a0, 1
        assert_eq!(expand(0x8905), Some(andi(10, 10, 1))); // c.andi a0, 1
        assert_eq!(expand(0x8d0d), Some(sub(10, 10, 11))); // c.sub a0, a1
        assert_eq!(expand(0x8d2d), Some(xor(10, 10, 11))); // c.xor a0, a1
        assert_eq!(expand(0x8d4d), Some(or(10, 10, 11))); // c.or a0, a1
        assert_eq!(expand(0x8d6d), Some(and(10, 10, 11))); // c.and a0, a1
        assert_eq!(expand(0x4502), Some(lw(10, 2, 0))); // c.lwsp a0, 0(sp)
        assert_eq!(expand(0xc22a), Some(sw(10, 2, 4))); // c.swsp a0, 4(sp)
    }

    #[test]
    fn expands_jumps_and_branches_with_signed_offsets() {
        assert_eq!(expand(0xa001), Some(jal(0, 0))); // c.j 0
        assert_eq!(expand(0xbffd), Some(jal(0, -2))); // c.j -2
        assert_eq!(expand(0x2001), Some(jal(1, 0))); // c.jal 0
        assert_eq!(expand(0xc111), Some(beq(10, 0, 4))); // c.beqz a0, 4
        assert_eq!(expand(0xfd75), Some(bne(10, 0, -4))); // c.bnez a0, -4
    }

    #[test]
    fn rejects_reserved_and_fp_forms() {
        assert_eq!(expand(0x0000), None); // defined illegal instruction
        assert_eq!(expand(0x6101), None); // c.addi16sp with imm 0
        assert_eq!(expand(0x6501), None); // c.lui with imm 0
        assert_eq!(expand(0x8002), None); // c.jr x0
        assert_eq!(expand(0x4002), None); // c.lwsp x0
        assert_eq!(expand(0x1502), None); // c.slli with shamt[5] = 1
        assert_eq!(expand(0x2000), None); // c.fld
        assert_eq!(expand(0x9d0d), None); // c.subw (RV64)
        assert!(is_compressed(0x4505));
        assert!(!is_compressed(0x0013));
    }
}
