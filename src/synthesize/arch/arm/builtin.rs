use ux::u12;

use crate::synthesize::arch::{
    Assembler,
    arm::{
        instr::{self, ImmShift16},
        reg::Reg,
    },
};

use super::ArmAssembler;

type BuiltinFn = fn(&mut ArmAssembler);

const PREFIX: &str = "std::";

const PAGE_SIZE: u64 = 16384;

pub fn assemble(asm: &mut ArmAssembler) {
    let builtins: &[(&str, BuiltinFn)] = &[("exit", exit), ("write", write)];

    for (name, assemble_fn) in builtins {
        asm.functions
            .insert(format!("{}{}", PREFIX, name), asm.current_offset());
        assemble_fn(asm);
    }
}

pub fn write(asm: &mut ArmAssembler) {
    asm.begin_stack(u12::new(0));
    syscall(asm, SyscallType::Write);
    asm.end_stack();
}

pub fn exit(asm: &mut ArmAssembler) {
    syscall(asm, SyscallType::Exit);
}

fn syscall(asm: &mut ArmAssembler, typ: SyscallType) {
    asm.emit(instr::Movz {
        shift: ImmShift16::L16,
        imm_value: 1 << 9,
        dest: Reg::X16,
    });
    asm.emit(instr::Add {
        a: Reg::X16,
        b: instr::Input::Imm(ux::i12::new(typ as u16 as i16)),
        dest: Reg::X16,
    });

    asm.emit(instr::Syscall)
}

#[repr(u16)]
enum SyscallType {
    Exit = 1,
    Write = 4,
    MUnmap = 73,
    MMap = 197,
}
