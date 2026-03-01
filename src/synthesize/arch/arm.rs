use std::collections::HashMap;

use num_traits::FromPrimitive;
use ux::{i7, i12, i26, u12};

use crate::{
    ir::{IR, Item, Operation, SourceVal, VirtualReg},
    synthesize::arch::{
        Assemble, MachineCode,
        arm::{
            instr::{ImmShift16, Instruction},
            reg::{Allocator, Reg, Register},
        },
    },
};

pub mod instr;
pub mod reg;

// const MAX_EXIT_CODE: u16 = 255; // On UNIX

const MAIN_FN: &str = "main";

#[derive(Default)]
pub struct ArmAssembler {
    code: MachineCode,
    functions: HashMap<String, usize>,
    fn_calls: Vec<(String, usize)>,
    stacks: Vec<i12>,
}

impl Assemble for ArmAssembler {
    fn assemble(ir: IR) -> MachineCode {
        let mut asm = ArmAssembler::default();

        for item in ir.items {
            asm.asm_item(item);
        }

        let mut emitter = OpEmitter::new(&mut asm, Allocator::default());
        emitter.emit_call(MAIN_FN.to_owned(), vec![], None, 0);

        asm.emit(instr::Movz {
            shift: ImmShift16::L0,
            imm_value: 0x1,
            dest: Reg::X16,
        });

        asm.emit(instr::Syscall);

        for (function, call_offset) in std::mem::take(&mut asm.fn_calls) {
            let fn_offset = asm
                .functions
                .get(&function)
                .expect("trying to assemble call to unknown function");

            let rel_offset = (*fn_offset as i32 - call_offset as i32) / 4;
            asm.emit_at(
                call_offset,
                instr::BranchLink {
                    addr: i26::new(rel_offset),
                },
            );
        }

        asm.code
    }
}

impl ArmAssembler {
    fn emit(&mut self, instr: impl Instruction) {
        self.code.instructions.extend(instr.encode().to_le_bytes());
    }

    fn emit_at(&mut self, offset: usize, instr: impl Instruction) {
        let bytes = instr.encode().to_le_bytes();
        self.code.instructions[offset..(offset + 4)].copy_from_slice(&bytes);
    }

    fn current_offset(&self) -> usize {
        self.code.instructions.len()
    }

    fn asm_item(&mut self, item: Item) {
        let Item::Function { name, args, bb } = item;
        self.functions.insert(name.clone(), self.current_offset());

        let alloc = reg::allocate(&bb, &args);

        self.begin_stack(alloc.stack_size());

        let mut emitter = OpEmitter::new(self, alloc);
        for (idx, op) in bb.ops.into_iter().enumerate() {
            emitter.asm_op(op, idx);
        }
    }

    fn begin_stack(&mut self, stack_size: u12) {
        self.emit(instr::StorePair {
            base: Reg::SP,
            first: Reg::FP,
            second: Reg::LR,
            offset: i7::new(-2),
        });

        self.emit(instr::MovReg {
            src: Reg::SP,
            dest: Reg::FP,
        });

        if stack_size != u12::new(0) {
            let mut stack_size: u16 = stack_size.into();
            // align to 16 bytes
            if !stack_size.is_multiple_of(2) {
                stack_size += 1;
            }

            let stack_size = stack_size as i16;
            let stack_size = i12::new(stack_size * 8);

            self.stacks.push(stack_size);

            self.emit(instr::Sub {
                a: Reg::SP,
                b: instr::Input::Imm(stack_size),
                dest: Reg::SP,
            });
        }
    }

    fn end_stack(&mut self) {
        if let Some(stack_size) = self.stacks.pop()
            && stack_size != i12::new(0)
        {
            self.emit(instr::Add {
                a: Reg::SP,
                b: instr::Input::Imm(stack_size),
                dest: Reg::SP,
            });
        }

        self.emit(instr::LoadPair {
            base: Reg::SP,
            first: Reg::FP,
            second: Reg::LR,
            offset: i7::new(2),
        });
    }

    fn emit_store(&mut self, offset: u12, register: Register) {
        self.emit(instr::Store {
            base: Reg::SP,
            offset: instr::Input::Imm(offset),
            register,
        });
    }

    fn emit_movz(&mut self, n: i64, dest: Register) {
        self.emit(instr::Movz {
            shift: ImmShift16::L0,
            imm_value: n as u16,
            dest,
        });
    }

    fn emit_nop(&mut self) {
        self.emit(instr::Nop);
    }
}

struct OpEmitter<'c> {
    asm: &'c mut ArmAssembler,
    alloc: Allocator,
}

impl<'c> OpEmitter<'c> {
    pub fn new(asm: &'c mut ArmAssembler, alloc: Allocator) -> Self {
        Self { asm, alloc }
    }

    fn map_reg(&mut self, vreg: VirtualReg, instr_index: usize) -> Register {
        self.alloc.map(vreg, instr_index).unwrap(self.asm)
    }

    fn asm_op(&mut self, op: Operation, idx: usize) {
        match op {
            Operation::Assign { src, dest } => self.emit_assign(src, dest, idx),
            Operation::Add { a, b, dest } => self.emit_add(a, b, dest, idx),
            Operation::Subtract { a, b, dest } => self.emit_sub(a, b, dest, idx),
            Operation::Multiply { a, b, dest } => self.emit_mul(a, b, dest, idx),
            Operation::Divide { a, b, dest } => self.emit_div(a, b, dest, idx),
            Operation::Return { value } => self.emit_return(value, idx),
            Operation::Call {
                function,
                args,
                dest,
            } => self.emit_call(function, args, dest, idx),
        }
    }

    fn emit_assign(&mut self, src: SourceVal, dest: VirtualReg, idx: usize) {
        let dest = self.map_reg(dest, idx);
        match src {
            SourceVal::Immediate(n) => self.asm.emit_movz(n, dest),
            SourceVal::VReg(vreg) => {
                let src = self.map_reg(vreg, idx);

                if src == dest {
                    println!("emit same reg move");
                }

                self.asm.emit(instr::MovReg { src, dest });
            }
        }
    }

    fn emit_call(
        &mut self,
        function: String,
        args: Vec<VirtualReg>,
        dest: Option<VirtualReg>,
        instr_index: usize,
    ) {
        if let Some(regs_to_save) = self.alloc.stack_save(instr_index) {
            for (reg, offset) in regs_to_save {
                self.asm.emit_store(*offset, *reg);
            }
        }

        if args.len() > 8 {
            todo!();
        }

        for (i, &arg) in args.iter().enumerate() {
            let src = self.map_reg(arg, instr_index);
            let dest = Register::from_usize(i).unwrap();
            self.asm.emit(instr::MovReg { src, dest });
        }

        let offset = self.asm.current_offset();
        self.asm.emit_nop();
        self.asm.fn_calls.push((function.clone(), offset));

        if let Some(dest) = dest {
            let dest = self.map_reg(dest, instr_index);
            self.asm.emit(instr::MovReg { src: Reg::X0, dest });
        }
    }

    fn emit_add(&mut self, a: SourceVal, b: SourceVal, dest: VirtualReg, idx: usize) {
        let dest = self.map_reg(dest, idx);
        match (a, b) {
            (SourceVal::Immediate(a), SourceVal::Immediate(b)) => self.asm.emit_movz(a + b, dest),
            (SourceVal::Immediate(n), SourceVal::VReg(vreg))
            | (SourceVal::VReg(vreg), SourceVal::Immediate(n)) => {
                assert!(n <= i16::MAX as i64);
                let a = self.map_reg(vreg, idx);
                self.asm.emit(instr::Add {
                    a,
                    b: instr::Input::Imm(i12::new(n as i16)),
                    dest,
                });
            }
            (SourceVal::VReg(a), SourceVal::VReg(b)) => {
                let a = self.map_reg(a, idx);
                let b = self.map_reg(b, idx);
                self.asm.emit(instr::Add {
                    a,
                    b: instr::Input::Reg(b),
                    dest,
                })
            }
        }
    }

    fn emit_sub(&mut self, a: SourceVal, b: SourceVal, dest: VirtualReg, idx: usize) {
        let dest = self.map_reg(dest, idx);
        match (a, b) {
            (SourceVal::Immediate(a), SourceVal::Immediate(b)) => self.asm.emit_movz(a - b, dest),
            (SourceVal::Immediate(n), SourceVal::VReg(vreg))
            | (SourceVal::VReg(vreg), SourceVal::Immediate(n)) => {
                assert!(n <= i16::MAX as i64);
                let a = self.map_reg(vreg, idx);
                self.asm.emit(instr::Sub {
                    a,
                    b: instr::Input::Imm(i12::new(n as i16)),
                    dest,
                });
            }
            (SourceVal::VReg(a), SourceVal::VReg(b)) => {
                let a = self.map_reg(a, idx);
                let b = self.map_reg(b, idx);
                self.asm.emit(instr::Sub {
                    a,
                    b: instr::Input::Reg(b),
                    dest,
                })
            }
        }
    }

    fn emit_mul(&mut self, a: VirtualReg, b: VirtualReg, dest: VirtualReg, idx: usize) {
        let dest = self.map_reg(dest, idx);
        let a = self.map_reg(a, idx);
        let b = self.map_reg(b, idx);
        self.asm.emit(instr::Mul { a, b, dest });
    }

    fn emit_div(&mut self, a: VirtualReg, b: VirtualReg, dest: VirtualReg, idx: usize) {
        let dest = self.map_reg(dest, idx);
        let a = self.map_reg(a, idx);
        let b = self.map_reg(b, idx);
        self.asm.emit(instr::Div {
            a,
            b,
            dest,
            signed: true,
        });
    }

    fn emit_return(&mut self, src: SourceVal, idx: usize) {
        match src {
            SourceVal::Immediate(n) => self.asm.emit_movz(n, Reg::X0),
            SourceVal::VReg(vreg) => {
                let src = self.map_reg(vreg, idx);
                self.asm.emit(instr::MovReg { src, dest: Reg::X0 });
            }
        }

        self.asm.end_stack();
        self.asm.emit(instr::Ret);
    }
}
