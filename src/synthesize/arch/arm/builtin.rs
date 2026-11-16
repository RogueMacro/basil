use ux::u12;

use crate::{
    ir::{Condition, VregSize},
    synthesize::arch::{
        Assembler,
        arm::{
            instr::{EitherOffset, EitherReg, ImmShift16, Inst},
            reg::Reg,
        },
    },
};

use super::ArmAssembler;

type BuiltinFn = fn(&mut ArmAssembler);

const PREFIX: &str = "std::syscall::";

pub fn assemble(asm: &mut ArmAssembler) {
    let builtins: &[(&str, BuiltinFn)] = &[
        ("exit", exit),
        ("read", read),
        ("write", write),
        ("open", open),
        ("close", close),
        ("mmap", mmap),
        ("lseek", lseek),
    ];

    for (name, assemble_fn) in builtins {
        let offset_in_bytes = asm.code_size();
        asm.functions
            .insert(format!("{}{}", PREFIX, name), offset_in_bytes / 4);
        asm.code
            .symbols
            .push((String::from(*name), offset_in_bytes as u64));

        assemble_fn(asm);
    }
}

fn load_arg(asm: &mut ArmAssembler, fp_offset: u32, dest: Reg) {
    asm.emit(Inst::Load {
        base: EitherReg::Phys(Reg::SP),
        offset: EitherOffset::Imm(u12::new(fp_offset as u16)),
        dest,
        size: VregSize::Doubleword,
    });
}

pub fn exit(asm: &mut ArmAssembler) {
    load_arg(asm, 0, Reg::X0);
    syscall(asm, SyscallType::Exit);
}

pub fn read(asm: &mut ArmAssembler) {
    load_arg(asm, 0, Reg::X0);
    load_arg(asm, 1, Reg::X1);
    load_arg(asm, 2, Reg::X2);
    syscall(asm, SyscallType::Read);
    set_topmost_bit_if_error(asm);
    asm.emit(Inst::Ret { value: Reg::X0 });
}

pub fn write(asm: &mut ArmAssembler) {
    load_arg(asm, 0, Reg::X0);
    load_arg(asm, 1, Reg::X1);
    load_arg(asm, 2, Reg::X2);
    syscall(asm, SyscallType::Write);
    set_topmost_bit_if_error(asm);
    asm.emit(Inst::Ret { value: Reg::X0 });
}

pub fn open(asm: &mut ArmAssembler) {
    load_arg(asm, 0, Reg::X0);
    load_arg(asm, 1, Reg::X1);
    syscall(asm, SyscallType::Open);
    set_topmost_bit_if_error(asm);
    asm.emit(Inst::Ret { value: Reg::X0 });
}

pub fn close(asm: &mut ArmAssembler) {
    load_arg(asm, 0, Reg::X0);
    syscall(asm, SyscallType::Close);
    set_topmost_bit_if_error(asm);
    asm.emit(Inst::Ret { value: Reg::X0 });
}

pub fn mmap(asm: &mut ArmAssembler) {
    load_arg(asm, 0, Reg::X0);
    load_arg(asm, 1, Reg::X1);
    load_arg(asm, 2, Reg::X2);
    load_arg(asm, 3, Reg::X3);
    load_arg(asm, 4, Reg::X4);
    load_arg(asm, 5, Reg::X5);
    syscall(asm, SyscallType::MMap);
    set_topmost_bit_if_error(asm);
    asm.emit(Inst::Ret { value: Reg::X0 });
}

pub fn lseek(asm: &mut ArmAssembler) {
    load_arg(asm, 0, Reg::X0);
    load_arg(asm, 1, Reg::X1);
    load_arg(asm, 2, Reg::X2);
    syscall(asm, SyscallType::Lseek);
    set_topmost_bit_if_error(asm);
    asm.emit(Inst::Ret { value: Reg::X0 });
}

fn syscall(asm: &mut ArmAssembler, typ: SyscallType) {
    asm.emit(Inst::Movz {
        shift: ImmShift16::L0,
        value: typ as u16,
        dest: Reg::X16,
    });

    asm.emit(Inst::Syscall)
}

fn set_topmost_bit_if_error(asm: &mut ArmAssembler) {
    asm.emit_many([
        Inst::Mov {
            src: EitherReg::Phys(Reg::X0),
            dest: EitherReg::Phys(Reg::X1),
        },
        Inst::Movk {
            shift: ImmShift16::L48,
            value: 0x8000,
            dest: Reg::X1,
        },
        Inst::Csel {
            cond: Condition::UnsignedGreaterOrEqual,
            if_true: Reg::X1,
            if_false: Reg::X0,
            dest: Reg::X0,
        },
    ]);
}

#[repr(u16)]
pub enum SyscallType {
    Exit = 1,
    Read = 3,
    Write = 4,
    Open = 5,
    Close = 6,
    MUnmap = 73,
    MMap = 197,
    Lseek = 199,
}
