//! Instruction encoders: a minimal assembler for tests, tools and the
//! compressed-instruction expander. Register arguments are `x` numbers
//! (0 to 31); immediates are byte offsets or values as in assembly.

#![allow(missing_docs)] // one-line encoders named after their mnemonics

fn r(f7: u32, rs2: u8, rs1: u8, f3: u32, rd: u8, op: u32) -> u32 {
    (f7 << 25) | (u32::from(rs2) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (u32::from(rd) << 7) | op
}
fn i(imm: i32, rs1: u8, f3: u32, rd: u8, op: u32) -> u32 {
    ((imm as u32 & 0xfff) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (u32::from(rd) << 7) | op
}
fn s(imm: i32, rs2: u8, rs1: u8, f3: u32, op: u32) -> u32 {
    let imm = imm as u32;
    (((imm >> 5) & 0x7f) << 25)
        | (u32::from(rs2) << 20)
        | (u32::from(rs1) << 15)
        | (f3 << 12)
        | ((imm & 0x1f) << 7)
        | op
}
fn b(imm: i32, rs2: u8, rs1: u8, f3: u32) -> u32 {
    let imm = imm as u32;
    (((imm >> 12) & 1) << 31)
        | (((imm >> 5) & 0x3f) << 25)
        | (u32::from(rs2) << 20)
        | (u32::from(rs1) << 15)
        | (f3 << 12)
        | (((imm >> 1) & 0xf) << 8)
        | (((imm >> 11) & 1) << 7)
        | 0x63
}
fn u(imm: u32, rd: u8, op: u32) -> u32 {
    (imm & 0xffff_f000) | (u32::from(rd) << 7) | op
}
fn j(imm: i32, rd: u8) -> u32 {
    let imm = imm as u32;
    (((imm >> 20) & 1) << 31)
        | (((imm >> 1) & 0x3ff) << 21)
        | (((imm >> 11) & 1) << 20)
        | (((imm >> 12) & 0xff) << 12)
        | (u32::from(rd) << 7)
        | 0x6f
}

/// `lui rd, imm` where `imm` is the full 32-bit value (low 12 bits ignored).
pub fn lui(rd: u8, imm: u32) -> u32 {
    u(imm, rd, 0x37)
}
/// `auipc rd, imm` where `imm` is the full 32-bit value (low 12 bits ignored).
pub fn auipc(rd: u8, imm: u32) -> u32 {
    u(imm, rd, 0x17)
}
pub fn jal(rd: u8, off: i32) -> u32 {
    j(off, rd)
}
pub fn jalr(rd: u8, rs1: u8, off: i32) -> u32 {
    i(off, rs1, 0, rd, 0x67)
}
pub fn beq(rs1: u8, rs2: u8, off: i32) -> u32 {
    b(off, rs2, rs1, 0)
}
pub fn bne(rs1: u8, rs2: u8, off: i32) -> u32 {
    b(off, rs2, rs1, 1)
}
pub fn blt(rs1: u8, rs2: u8, off: i32) -> u32 {
    b(off, rs2, rs1, 4)
}
pub fn bge(rs1: u8, rs2: u8, off: i32) -> u32 {
    b(off, rs2, rs1, 5)
}
pub fn bltu(rs1: u8, rs2: u8, off: i32) -> u32 {
    b(off, rs2, rs1, 6)
}
pub fn bgeu(rs1: u8, rs2: u8, off: i32) -> u32 {
    b(off, rs2, rs1, 7)
}
pub fn lb(rd: u8, rs1: u8, off: i32) -> u32 {
    i(off, rs1, 0, rd, 0x03)
}
pub fn lh(rd: u8, rs1: u8, off: i32) -> u32 {
    i(off, rs1, 1, rd, 0x03)
}
pub fn lw(rd: u8, rs1: u8, off: i32) -> u32 {
    i(off, rs1, 2, rd, 0x03)
}
pub fn lbu(rd: u8, rs1: u8, off: i32) -> u32 {
    i(off, rs1, 4, rd, 0x03)
}
pub fn lhu(rd: u8, rs1: u8, off: i32) -> u32 {
    i(off, rs1, 5, rd, 0x03)
}
/// `sb rs2, off(rs1)`
pub fn sb(rs2: u8, rs1: u8, off: i32) -> u32 {
    s(off, rs2, rs1, 0, 0x23)
}
/// `sh rs2, off(rs1)`
pub fn sh(rs2: u8, rs1: u8, off: i32) -> u32 {
    s(off, rs2, rs1, 1, 0x23)
}
/// `sw rs2, off(rs1)`
pub fn sw(rs2: u8, rs1: u8, off: i32) -> u32 {
    s(off, rs2, rs1, 2, 0x23)
}
pub fn addi(rd: u8, rs1: u8, imm: i32) -> u32 {
    i(imm, rs1, 0, rd, 0x13)
}
pub fn slti(rd: u8, rs1: u8, imm: i32) -> u32 {
    i(imm, rs1, 2, rd, 0x13)
}
pub fn sltiu(rd: u8, rs1: u8, imm: i32) -> u32 {
    i(imm, rs1, 3, rd, 0x13)
}
pub fn xori(rd: u8, rs1: u8, imm: i32) -> u32 {
    i(imm, rs1, 4, rd, 0x13)
}
pub fn ori(rd: u8, rs1: u8, imm: i32) -> u32 {
    i(imm, rs1, 6, rd, 0x13)
}
pub fn andi(rd: u8, rs1: u8, imm: i32) -> u32 {
    i(imm, rs1, 7, rd, 0x13)
}
pub fn slli(rd: u8, rs1: u8, sh: u32) -> u32 {
    i((sh & 31) as i32, rs1, 1, rd, 0x13)
}
pub fn srli(rd: u8, rs1: u8, sh: u32) -> u32 {
    i((sh & 31) as i32, rs1, 5, rd, 0x13)
}
pub fn srai(rd: u8, rs1: u8, sh: u32) -> u32 {
    i((0x400 | (sh & 31)) as i32, rs1, 5, rd, 0x13)
}
pub fn add(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(0, rs2, rs1, 0, rd, 0x33)
}
pub fn sub(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(0x20, rs2, rs1, 0, rd, 0x33)
}
pub fn sll(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(0, rs2, rs1, 1, rd, 0x33)
}
pub fn slt(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(0, rs2, rs1, 2, rd, 0x33)
}
pub fn sltu(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(0, rs2, rs1, 3, rd, 0x33)
}
pub fn xor(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(0, rs2, rs1, 4, rd, 0x33)
}
pub fn srl(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(0, rs2, rs1, 5, rd, 0x33)
}
pub fn sra(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(0x20, rs2, rs1, 5, rd, 0x33)
}
pub fn or(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(0, rs2, rs1, 6, rd, 0x33)
}
pub fn and(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(0, rs2, rs1, 7, rd, 0x33)
}
pub fn mul(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(1, rs2, rs1, 0, rd, 0x33)
}
pub fn mulh(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(1, rs2, rs1, 1, rd, 0x33)
}
pub fn mulhsu(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(1, rs2, rs1, 2, rd, 0x33)
}
pub fn mulhu(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(1, rs2, rs1, 3, rd, 0x33)
}
pub fn div(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(1, rs2, rs1, 4, rd, 0x33)
}
pub fn divu(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(1, rs2, rs1, 5, rd, 0x33)
}
pub fn rem(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(1, rs2, rs1, 6, rd, 0x33)
}
pub fn remu(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(1, rs2, rs1, 7, rd, 0x33)
}
fn amo(f5: u32, rd: u8, rs1: u8, rs2: u8) -> u32 {
    r(f5 << 2, rs2, rs1, 2, rd, 0x2f)
}
pub fn lr_w(rd: u8, rs1: u8) -> u32 {
    amo(0x02, rd, rs1, 0)
}
/// `sc.w rd, rs2, (rs1)`
pub fn sc_w(rd: u8, rs1: u8, rs2: u8) -> u32 {
    amo(0x03, rd, rs1, rs2)
}
pub fn amoswap_w(rd: u8, rs1: u8, rs2: u8) -> u32 {
    amo(0x01, rd, rs1, rs2)
}
pub fn amoadd_w(rd: u8, rs1: u8, rs2: u8) -> u32 {
    amo(0x00, rd, rs1, rs2)
}
pub fn amoxor_w(rd: u8, rs1: u8, rs2: u8) -> u32 {
    amo(0x04, rd, rs1, rs2)
}
pub fn amoand_w(rd: u8, rs1: u8, rs2: u8) -> u32 {
    amo(0x0c, rd, rs1, rs2)
}
pub fn amoor_w(rd: u8, rs1: u8, rs2: u8) -> u32 {
    amo(0x08, rd, rs1, rs2)
}
pub fn amomin_w(rd: u8, rs1: u8, rs2: u8) -> u32 {
    amo(0x10, rd, rs1, rs2)
}
pub fn amomax_w(rd: u8, rs1: u8, rs2: u8) -> u32 {
    amo(0x14, rd, rs1, rs2)
}
pub fn amominu_w(rd: u8, rs1: u8, rs2: u8) -> u32 {
    amo(0x18, rd, rs1, rs2)
}
pub fn amomaxu_w(rd: u8, rs1: u8, rs2: u8) -> u32 {
    amo(0x1c, rd, rs1, rs2)
}
pub fn csrrw(rd: u8, csr: u16, rs1: u8) -> u32 {
    i(i32::from(csr), rs1, 1, rd, 0x73)
}
pub fn csrrs(rd: u8, csr: u16, rs1: u8) -> u32 {
    i(i32::from(csr), rs1, 2, rd, 0x73)
}
pub fn csrrc(rd: u8, csr: u16, rs1: u8) -> u32 {
    i(i32::from(csr), rs1, 3, rd, 0x73)
}
pub fn csrrwi(rd: u8, csr: u16, uimm: u8) -> u32 {
    i(i32::from(csr), uimm & 31, 5, rd, 0x73)
}
pub fn csrrsi(rd: u8, csr: u16, uimm: u8) -> u32 {
    i(i32::from(csr), uimm & 31, 6, rd, 0x73)
}
pub fn csrrci(rd: u8, csr: u16, uimm: u8) -> u32 {
    i(i32::from(csr), uimm & 31, 7, rd, 0x73)
}
pub const ECALL: u32 = 0x0000_0073;
pub const EBREAK: u32 = 0x0010_0073;
pub const MRET: u32 = 0x3020_0073;
pub const WFI: u32 = 0x1050_0073;
/// `fence iorw, iorw`
pub const FENCE: u32 = 0x0ff0_000f;
pub const FENCE_I: u32 = 0x0000_100f;
pub const NOP: u32 = 0x0000_0013;

/// `li rd, value` as `lui` + `addi` (always two instructions).
pub fn li(rd: u8, value: u32) -> [u32; 2] {
    let lo = ((value << 20) as i32) >> 20; // sign-extended low 12 bits
    let hi = value.wrapping_sub(lo as u32);
    [lui(rd, hi), addi(rd, rd, lo)]
}
