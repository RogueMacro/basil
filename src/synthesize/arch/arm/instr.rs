#![allow(clippy::unusual_byte_groupings)]

use ux::{i7, i12, i26, u12};

use super::reg::Register;

/// An Armv8 instruction.
///
/// All Armv8 instructions are 32 bits long.
pub trait Instruction: std::fmt::Debug {
    fn encode(&self) -> u32;
}

impl Instruction for u32 {
    fn encode(&self) -> u32 {
        *self
    }
}

/// An immediate shift specifier with a 16-bit step size.
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum ImmShift16 {
    L0 = 0,
    L16 = 1,
    L32 = 2,
    L48 = 3,
}

/// Input to an instruction that has variants for both immediate values and registers.
#[derive(Debug, Clone, Copy)]
pub enum Input<I> {
    Reg(Register),
    Imm(I),
}

/// ADD instruction.
///
/// Encoding:
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  0  0  0  1  0  1  1  shift 0  Rm             imm6              Rn             Rd
///
/// - shift: (00) LSL (01) LSR (10) ASR (11) Reserved
/// - imm6: shift amount (0-63)
/// - Rn: first source register
/// - Rm: second source register
/// - Rd: destination register
#[derive(Debug, Clone, Copy)]
pub struct Add {
    pub a: Register,
    pub b: Input<i12>,
    pub dest: Register,
}

impl Instruction for Add {
    fn encode(&self) -> u32 {
        let a = self.a as u32;
        let dest = self.dest as u32;

        match self.b {
            Input::Reg(reg) => (0b10001011 << 24) | (a << 16) | ((reg as u32) << 5) | dest,
            Input::Imm(imm) => {
                let imm: i32 = imm.into();
                (0b1001000100 << 22) | ((imm as u32) << 10) | (a << 5) | dest
            }
        }
    }
}

/// Branch with link instruction.
///
/// Stores pc+4 in lr and jumps to address.
///
/// Encoding:
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  0  0  1  0  1  imm26
///
/// - imm26: pc relative offset (in instructions, not bytes) to jump to
#[derive(Debug, Clone, Copy)]
pub struct BranchLink {
    pub addr: i26,
}

impl Instruction for BranchLink {
    fn encode(&self) -> u32 {
        let addr = (to_u32(self.addr)) & 0b000000_11111111111111111111111111;

        0b100101_00000000000000000000000000 | addr
    }
}

/// SDIV or UDIV instruction.
///
/// Encoding (SDIV):
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  0  0  1  1  0  1  0  1  1  0  Rm             0  0  0  0  1  1  Rn             Rd
///
/// - Rn: first source register
/// - Rm: second source register
/// - Rd: destination register
#[derive(Debug, Clone, Copy)]
pub struct Div {
    pub a: Register,
    pub b: Register,
    pub dest: Register,
    pub signed: bool,
}

impl Instruction for Div {
    fn encode(&self) -> u32 {
        let a = self.a as u32;
        let b = self.b as u32;
        let dest = self.dest as u32;

        if self.signed {
            (0b10011010110_00000_000011 << 10) | (b << 16) | (a << 5) | dest
        } else {
            todo!()
        }
    }
}

/// LDR instruction.
///
/// Encoding (unsigned offset):
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  1  1  1  1  0  0  1  0  1  imm12                               Rn             Rt
///
/// - imm12: offset from base (stored as a multiple of 8)
/// - Rn: base pointer
/// - Rt: destination register
#[derive(Debug, Clone, Copy)]
pub struct Load {
    pub stack_offset: u12,
    pub dest: Register,
}

impl Instruction for Load {
    fn encode(&self) -> u32 {
        let offset: u32 = self.stack_offset.into();
        let offset = offset / 8;
        let base = Register::SP as u32;
        let dest = self.dest as u32;

        (0b1111100101 << 22) | (offset << 10) | (base << 5) | dest
    }
}

/// LDP instruction.
///
/// Loads two registers from memory and updates the base register.
///
/// Encoding (post-index):
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  0  1  0  1  0  0  0  1  1  imm7                 Rt2            Rn             Rt
///
/// - imm7: signed offset (scaled by 8 bytes)
/// - Rn: base register (writeback)
/// - Rt: first destination register
/// - Rt2: second destination register
#[derive(Debug, Clone, Copy)]
pub struct LoadPair {
    pub base: Register,
    pub first: Register,
    pub second: Register,
    pub offset: i7,
}

impl Instruction for LoadPair {
    fn encode(&self) -> u32 {
        let base = self.base as u32;
        let first = self.first as u32;
        let second = self.second as u32;
        let imm7: i8 = self.offset.into();
        let imm7 = imm7 as u32 & 0b1111111;

        0b1010100011 << 22 | imm7 << 15 | (second << 10) | (base << 5) | first
    }
}

/// MOV instruction.
///
/// Copies the value in the source register to the destination register.
///
/// Encoding (register to register):
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  0  1  0  1  0  1  0  0  0  0  Rm             0  0  0  0  0  0  1  1  1  1  1  Rd
///
/// Encoding (to/from SP):
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  0  0  1  0  0  0  1  0  0  0  0  0  0  0  0  0  0  0  0  0  0  Rn             Rd
///
/// - Rm: source register
/// - Rd: destination register
#[derive(Debug, Clone, Copy)]
pub struct MovReg {
    pub src: Register,
    pub dest: Register,
}

impl Instruction for MovReg {
    fn encode(&self) -> u32 {
        let src = self.src as u32;
        let dest = self.dest as u32;

        if self.src == Register::SP || self.dest == Register::SP {
            (0b1001000100000000000000 << 10) | (src << 5) | dest
        } else {
            (0b10101010000_00000_00000011111 << 5) | (src << 16) | dest
        }
    }
}

/// MOVZ instruction.
///
/// Moves a 16-bit immediate value into destination register, zeroing all "non-affected" bits. Can
/// be combined with [MOVK](Movk) to move a 32 or 64-bit value.
///
/// Encoding:
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  1  0  1  0  0  1  0  1  hw    imm16                                           Rd
///
/// - hw: shift left (0/16/32/48 encoded as 0/1/2/3)
/// - imm16: 16-bit immediate value to (optionally shift) into destination register
/// - Rd: destination register
#[derive(Debug, Clone, Copy)]
pub struct Movz {
    pub shift: ImmShift16,
    pub imm_value: u16,
    pub dest: Register,
}

impl Instruction for Movz {
    fn encode(&self) -> u32 {
        (0b110100101 << 23)
            | ((self.shift as u32) << 21)
            | ((self.imm_value as u32) << 5)
            | self.dest as u32
    }
}

/// MUL instruction. (alias of MADD)
///
/// Rd = Rn * Rm
///
/// Encoding:
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  0  0  1  1  0  1  1  0  0  0  Rm             0  1  1  1  1  1  Rn             Rd
///
/// - Rn: first value register
/// - Rm: second value register
/// - Rd: destination register
#[derive(Debug, Clone, Copy)]
pub struct Mul {
    pub a: Register,
    pub b: Register,
    pub dest: Register,
}

impl Instruction for Mul {
    fn encode(&self) -> u32 {
        let a = self.a as u32;
        let b = self.b as u32;
        let dest = self.dest as u32;

        (0b10011011000_00000_011111 << 10) | (a << 16) | (b << 5) | dest
    }
}

/// NOP instruction.
///
/// Does nothing except advance the program counter. Can be used for alignment.
///
/// Encoding:
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  1  0  1  0  1  0  1  0  0  0  0  0  0  1  1  0  0  1  0  0  0  0  0  0  0  0  1  1  1  1  1
#[derive(Debug, Clone, Copy)]
pub struct Nop;

impl Instruction for Nop {
    fn encode(&self) -> u32 {
        0b11010101000000110010000000011111
    }
}

/// Return from subroutine to offset stored in link register.
///
/// Encoding:
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  1  0  1  0  1  1  0  0  1  0  1  1  1  1  1  0  0  0  0  0  0  Rn             0  0  0  0  0
///
/// - Rn: register containing jump address (always set to X30/LR)
#[derive(Debug, Clone, Copy)]
pub struct Ret;

impl Instruction for Ret {
    fn encode(&self) -> u32 {
        // 0b11110 -> 30 -> Link register
        // This is equivalent to: mov pc, lr
        0b1101011001011111000000_11110_00000
    }
}

/// # STR instruction.
///
/// Calculates an address from a base pointer/stack pointer and an offset, and saves a register
/// value to that address.
///
/// ## Encoding (register):
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  1  1  1  1  0  0  0  0  0  1  Rm             1  1  1  0  1  0  Rn             Rt
///
/// - Rm: offset register
/// - Rn: base pointer
/// - Rt: source register
///
/// ## Encoding (immediate, unsigned offset):
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  1  1  1  1  0  0  1  0  0  imm12                               Rn             Rt
///
/// - imm12: offset (stored as a multiple of 8)
/// - Rn: base pointer
/// - Rt: source register
#[derive(Debug, Clone, Copy)]
pub struct Store {
    pub base: Register,

    /// Multiple of 8 bytes.
    pub offset: Input<u12>,

    pub register: Register,
}

impl Instruction for Store {
    fn encode(&self) -> u32 {
        let base = self.base as u32;
        let register = self.register as u32;

        match self.offset {
            Input::Reg(reg) => {
                (0b11111000001_00000_111010 << 10) | ((reg as u32) << 16) | (base << 5) | register
            }
            Input::Imm(imm) => {
                let imm: u32 = imm.into();
                (0b1111100100 << 22) | (imm << 10) | (base << 5) | register
            }
        }
    }
}

/// STP instruction.
///
/// Stores two registers to memory and updates the base register.
///
/// Encoding (pre-index):
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  0  1  0  1  0  0  1  1  0  imm7                 Rt2            Rn             Rt
///
/// - imm7: signed offset (scaled by 8 bytes)
/// - Rn: base register (writeback)
/// - Rt: first source register
/// - Rt2: second source register
#[derive(Debug, Clone, Copy)]
pub struct StorePair {
    pub base: Register,
    pub first: Register,
    pub second: Register,
    pub offset: i7,
}

impl Instruction for StorePair {
    fn encode(&self) -> u32 {
        let base = self.base as u32;
        let first = self.first as u32;
        let second = self.second as u32;
        let imm7: i8 = self.offset.into();
        let imm7 = imm7 as u32 & 0b1111111;

        (0b1010100110 << 22) | (imm7 << 15) | (second << 10) | (base << 5) | first
    }
}

/// SUB instruction.
///
/// Subtracts immediate value from register.
/// Rd = Rn - imm12
///
/// Encoding:
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  1  0  1  0  0  0  1  0  sh imm12                               Rn             Rd
///
/// Encoding:
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  1  0  0  1  0  1  1  shift 0  Rm             imm6              Rn             Rd
///
/// - sh: 0: no shift, 1: shift left by 12 bits
/// - shift: (00) LSL (01) LSR (10) ASR
/// - imm12: immediate value
/// - imm6: shift amount
/// - Rn: first source register
/// - Rm: second source register
/// - Rd: destination register
#[derive(Debug, Clone, Copy)]
pub struct Sub {
    pub a: Register,
    pub b: Input<i12>,
    pub dest: Register,
}

impl Instruction for Sub {
    fn encode(&self) -> u32 {
        let a = self.a as u32;
        let dest = self.dest as u32;

        match self.b {
            Input::Reg(b) => {
                let b = b as u32;
                (0b11001011_00_0 << 21) | (b << 16) | (a << 5) | dest
            }
            Input::Imm(imm) => {
                let imm: i16 = imm.into();
                (0b110100010_0 << 22) | ((imm as u32) << 10) | (a << 5) | dest
            }
        }
    }
}

/// SVC instruction.
///
/// Supervisor call. 0x80 counts as a valid immediate value. Call number should be stored in X16.
///
/// Encoding:
/// 31 30 29 28 27 26 25 24 23 22 21 20 19 18 17 16 15 14 13 12 11 10 9  8  7  6  5  4  3  2  1  0
/// 1  1  0  1  0  1  0  0  0  0  0  imm16                                           0  0  0  0  1
///
/// - imm16: 16-bit immediate value
#[derive(Debug, Clone, Copy)]
pub struct Svc {
    imm: u16,
}

impl Instruction for Svc {
    fn encode(&self) -> u32 {
        0b11010100000_0000000000000000_00001 | ((self.imm as u32) << 5)
    }
}

/// Alias for [SVC](Svc).
///
/// Uses (by convention) 0x80 for the svc immediate. Syscall number is stored in X16.
#[derive(Debug, Clone, Copy)]
pub struct Syscall;

impl Instruction for Syscall {
    fn encode(&self) -> u32 {
        Svc { imm: 0x80 }.encode()
    }
}

fn to_u32(n: impl Into<i32>) -> u32 {
    n.into() as u32
}
