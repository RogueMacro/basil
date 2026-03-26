use std::collections::HashMap;

use num_traits::FromPrimitive;
use ux::{i7, i12, i19, i21, i26, u12};

use crate::{
    ir::{Condition, IR, Item, Label, OpIndex, Operation, SourceVal, StrId, VirtualReg},
    synthesize::arch::{
        Assembler, MachineCode, UnfinishedCode,
        arm::{
            instr::{ImmShift16, Instruction},
            reg::{Allocator, Reg, Register},
        },
    },
};

pub mod builtin;
pub mod instr;
pub mod reg;

// const MAX_EXIT_CODE: u16 = 255; // On UNIX

const MAIN_FN: &str = "main";

type InstrIndex = usize;

#[derive(Default)]
pub struct ArmAssembler {
    code: MachineCode,
    functions: HashMap<String, InstrIndex>,
    fn_calls: Vec<(String, InstrIndex)>,
    stacks: Vec<i12>,
    str_literal_offsets: HashMap<StrId, usize>,

    lazy_emitters: Vec<Box<dyn Fn(&mut ArmAssembler, usize)>>,
}

impl Assembler for ArmAssembler {
    fn assemble(ir: IR) -> UnfinishedCode<Self> {
        let mut asm = ArmAssembler::default();

        let mut str_offset = 0;
        for (string, id) in ir.strings {
            asm.str_literal_offsets.insert(id, str_offset);
            str_offset += string.len() + 1; // c-string
            asm.code.str_literals.push(string);
        }

        for item in ir.items {
            asm.asm_item(item);
        }

        builtin::assemble(&mut asm);

        let entry_point_offset = asm.current_offset();
        let mut emitter = ScopedEmitter::new(&mut asm, Allocator::default(), HashMap::new());
        emitter.emit_call(MAIN_FN.to_owned(), vec![], None, 0);

        builtin::exit(&mut asm);

        for (function, call_offset) in std::mem::take(&mut asm.fn_calls) {
            let fn_offset = asm
                .functions
                .get(&function)
                .unwrap_or_else(|| panic!("call to unknown function {}", function));

            let rel_offset = (*fn_offset as i32 - call_offset as i32) / 4;
            asm.emit_at(
                call_offset,
                instr::BranchLink {
                    addr: i26::new(rel_offset),
                },
            );
        }

        asm.code.symbols = asm
            .functions
            .iter()
            .map(|(name, &offset)| (name.clone(), offset as u64))
            .collect();

        asm.code
            .symbols
            .push((String::from("_entry_point"), entry_point_offset as u64));

        UnfinishedCode(asm)
    }

    fn current_offset(&self) -> usize {
        self.code.instructions.len()
    }

    fn into_machine_code(mut self, str_literal_offset: usize) -> MachineCode {
        for emit in std::mem::take(&mut self.lazy_emitters) {
            emit(&mut self, str_literal_offset);
        }

        self.code
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

    fn asm_item(&mut self, item: Item) {
        let Item::Function { name, args, bb } = item;
        self.functions.insert(name.clone(), self.current_offset());

        let alloc = reg::allocate(&bb, &args);

        self.begin_stack(alloc.stack_size());

        let mut emitter = ScopedEmitter::new(self, alloc, bb.labels);
        for (idx, op) in bb.ops.into_iter().enumerate() {
            emitter.asm_op(op, idx);
        }

        emitter.end();

        self.end_stack();
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

        self.emit(instr::Ret);
    }

    fn emit_stack_store(&mut self, offset: u12, register: Register) {
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

    fn lazy_emit<F, I>(&mut self, emit: F)
    where
        F: Fn(usize) -> I + 'static,
        I: Instruction,
    {
        let instr_offset = self.current_offset();
        self.lazy_emitters.push(Box::new(move |asm, str_offset| {
            let instr = emit(str_offset);
            asm.emit_at(instr_offset, instr);
        }));

        self.emit_nop();
    }
}

struct ScopedEmitter<'c> {
    asm: &'c mut ArmAssembler,
    alloc: Allocator,
    ir_labels: HashMap<OpIndex, Vec<Label>>,
    mapped_labels: HashMap<Label, InstrIndex>,
    lazy_emits: Vec<Box<dyn FnOnce(&mut ScopedEmitter)>>,
}

impl<'c> ScopedEmitter<'c> {
    pub fn new(
        asm: &'c mut ArmAssembler,
        alloc: Allocator,
        ir_labels: HashMap<OpIndex, Vec<Label>>,
    ) -> Self {
        Self {
            asm,
            alloc,
            ir_labels,
            mapped_labels: HashMap::new(),
            lazy_emits: Vec::new(),
        }
    }

    fn map_reg_use(&mut self, vreg: VirtualReg, instr_index: usize) -> Register {
        self.alloc.map(vreg, instr_index).unwrap(self.asm)
    }

    fn map_reg_assign(&mut self, dest: VirtualReg, idx: usize) -> (Register, u12) {
        let reg = self.alloc.map(dest, idx).unwrap(self.asm);
        let stack_idx = self.alloc.stack_index_of(&dest);
        (reg, stack_idx)
    }

    fn asm_op(&mut self, op: Operation, idx: OpIndex) {
        if let Some(labels) = self.ir_labels.get(&idx) {
            for label in labels {
                self.mapped_labels.insert(*label, self.asm.current_offset());
                println!(
                    "inserted label {} at {} (op: {:?}",
                    label,
                    self.asm.current_offset() / 4,
                    op
                );
            }
        }

        match op {
            Operation::Assign { src, dest } => self.emit_assign(src, dest, idx),
            Operation::AddressOf { val, dest } => self.emit_addr_of(val, dest, idx),
            Operation::LoadPointer { ptr, dest } => self.emit_load_ptr(ptr, dest, idx),
            Operation::StorePointer { src, ptr } => self.emit_store_ptr(src, ptr, idx),
            Operation::LoadStr { str_id, dest } => todo!(),

            Operation::Add { a, b, dest } => self.emit_add(a, b, dest, idx),
            Operation::Subtract { a, b, dest } => self.emit_sub(a, b, dest, idx),
            Operation::Multiply { a, b, dest } => self.emit_mul(a, b, dest, idx),
            Operation::Divide { a, b, dest } => self.emit_div(a, b, dest, idx),

            Operation::Compare { a, b, cond, dest } => self.emit_cmp(a, b, cond, dest, idx),

            Operation::Branch { label } => self.emit_jump(label),
            Operation::BranchIf { cond, label } => self.emit_branch_if(cond, label, idx),
            Operation::BranchIfNot { cond, label } => self.emit_branch_if_not(cond, label, idx),

            Operation::Return { value } => self.emit_return(value, idx),
            Operation::Call {
                function,
                args,
                dest,
            } => self.emit_call(function, args, dest, idx),
        }
    }

    fn emit_assign(&mut self, src: SourceVal, dest: VirtualReg, idx: usize) {
        let (dest, stack_ptr) = self.map_reg_assign(dest, idx);

        match src {
            SourceVal::Immediate(n) => {
                self.asm.emit_movz(n, dest);
            }
            SourceVal::VReg(vreg) => {
                let src = self.map_reg_use(vreg, idx);
                if src != dest {
                    self.asm.emit(instr::MovReg { src, dest });
                }
            }
            SourceVal::String(str_id) => {
                let rel_str_offset = *self
                    .asm
                    .str_literal_offsets
                    .get(&str_id)
                    .unwrap_or_else(|| panic!("no string found for str_id #{}", str_id));

                let cur_offset = self.asm.current_offset();
                // let rel_str_offset = rel_str_offset + 0x100000000;

                self.asm.lazy_emit(move |str_table_offset| {
                    let abs_offset = str_table_offset + rel_str_offset;
                    let page_addr = i21::new((abs_offset / 4096) as i32);

                    instr::Adrp { page_addr, dest }
                });

                self.asm.lazy_emit(move |str_table_offset| {
                    let abs_offset = str_table_offset + rel_str_offset;
                    let in_page_offset = i12::new((abs_offset % 4096) as i16);

                    instr::Add {
                        a: dest,
                        b: instr::Input::Imm(in_page_offset),
                        dest,
                    }
                });
            }
        }

        self.asm.emit_stack_store(stack_ptr, dest);
    }

    fn emit_addr_of(&mut self, val: VirtualReg, dest: VirtualReg, idx: usize) {
        let stack_idx = self.alloc.stack_index_of(&val);
        let stack_idx: u16 = stack_idx.into();
        let stack_idx = i12::new(stack_idx as i16 * 8);

        let (dest, stack_ptr) = self.map_reg_assign(dest, idx);

        self.asm.emit(instr::Add {
            a: Register::SP,
            b: instr::Input::Imm(stack_idx),
            dest,
        });

        self.asm.emit_stack_store(stack_ptr, dest);
    }

    fn emit_load_ptr(&mut self, ptr: VirtualReg, dest: VirtualReg, idx: usize) {
        let ptr = self.map_reg_use(ptr, idx);
        let (dest, store_stack_ptr) = self.map_reg_assign(dest, idx);

        self.asm.emit(instr::Load {
            base: ptr,
            offset: u12::new(0),
            dest,
        });

        self.asm.emit_stack_store(store_stack_ptr, dest);
    }

    fn emit_store_ptr(&mut self, src: VirtualReg, ptr: VirtualReg, idx: usize) {
        let ptr = self.map_reg_use(ptr, idx);
        let src = self.map_reg_use(src, idx);

        self.asm.emit(instr::Store {
            base: ptr,
            offset: instr::Input::Imm(u12::new(0)),
            register: src,
        });
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
                self.asm.emit_stack_store(*offset, *reg);
            }
        }

        if args.len() > 8 {
            todo!();
        }

        for (i, &arg) in args.iter().enumerate() {
            let src = self.map_reg_use(arg, instr_index);
            let dest = Register::from_usize(i).unwrap();

            if (src as u32) < (i as u32) {
                // source register has been overwritten
                self.asm.emit(instr::Load {
                    base: Reg::SP,
                    offset: self.alloc.stack_index_of(&arg),
                    dest,
                });
            } else {
                self.asm.emit(instr::MovReg { src, dest });
            }
        }

        let offset = self.asm.current_offset();
        self.asm.emit_nop();
        self.asm.fn_calls.push((function.clone(), offset));

        if let Some(dest) = dest {
            let (dest, stack_ptr) = self.map_reg_assign(dest, instr_index);

            self.asm.emit(instr::MovReg { src: Reg::X0, dest });
            self.asm.emit_stack_store(stack_ptr, dest);
        }
    }

    fn emit_add(&mut self, a: VirtualReg, b: VirtualReg, dest: VirtualReg, idx: usize) {
        let (dest, stack_ptr) = self.map_reg_assign(dest, idx);
        let a = self.map_reg_use(a, idx);
        let b = self.map_reg_use(b, idx);

        self.asm.emit(instr::Add {
            a,
            b: instr::Input::Reg(b),
            dest,
        });
        self.asm.emit_stack_store(stack_ptr, dest);
    }

    fn emit_sub(&mut self, a: VirtualReg, b: VirtualReg, dest: VirtualReg, idx: usize) {
        let (dest, stack_ptr) = self.map_reg_assign(dest, idx);
        let a = self.map_reg_use(a, idx);
        let b = self.map_reg_use(b, idx);

        self.asm.emit(instr::Sub {
            a,
            b: instr::Input::Reg(b),
            dest,
        });
        self.asm.emit_stack_store(stack_ptr, dest);
    }

    fn emit_mul(&mut self, a: VirtualReg, b: VirtualReg, dest: VirtualReg, idx: usize) {
        let (dest, stack_ptr) = self.map_reg_assign(dest, idx);
        let a = self.map_reg_use(a, idx);
        let b = self.map_reg_use(b, idx);

        self.asm.emit(instr::Mul { a, b, dest });
        self.asm.emit_stack_store(stack_ptr, dest);
    }

    fn emit_div(&mut self, a: VirtualReg, b: VirtualReg, dest: VirtualReg, idx: usize) {
        let (dest, stack_ptr) = self.map_reg_assign(dest, idx);
        let a = self.map_reg_use(a, idx);
        let b = self.map_reg_use(b, idx);

        self.asm.emit(instr::Div {
            a,
            b,
            dest,
            signed: true,
        });
        self.asm.emit_stack_store(stack_ptr, dest);
    }

    fn emit_cmp(
        &mut self,
        a: VirtualReg,
        b: VirtualReg,
        cond: Condition,
        dest: VirtualReg,
        idx: usize,
    ) {
        let a = self.map_reg_use(a, idx);
        let b = self.map_reg_use(b, idx);
        let dest = self.map_reg_use(dest, idx);

        self.asm.emit(instr::Cmp { a, b });
        self.asm.emit(instr::BranchCond {
            cond,
            offset: i19::new(3),
        });
        self.asm.emit_movz(1, dest);
        self.asm.emit(instr::Branch {
            offset: i26::new(2),
        });
        self.asm.emit_movz(0, dest);
    }

    fn emit_branch_if(&mut self, cond: VirtualReg, label: Label, idx: OpIndex) {
        let cond = self.map_reg_use(cond, idx);

        let instr_idx = self.asm.current_offset();
        self.lazy_emit(label, move |offset| instr::BranchNotZero {
            addr: i19::new((offset as i32 - instr_idx as i32) / 4),
            reg: cond,
        });
    }

    fn emit_branch_if_not(&mut self, cond: VirtualReg, label: Label, idx: OpIndex) {
        let cond = self.map_reg_use(cond, idx);

        let instr_idx = self.asm.current_offset();
        self.lazy_emit(label, move |offset| instr::BranchZero {
            addr: i19::new((offset as i32 - instr_idx as i32) / 4),
            reg: cond,
        });
    }

    fn emit_jump(&mut self, label: Label) {
        let instr_idx = self.asm.current_offset();
        self.lazy_emit(label, move |offset| {
            let offset = i26::new((offset - instr_idx) as i32 / 4);
            instr::Branch { offset }
        });
    }

    fn emit_return(&mut self, src: SourceVal, idx: usize) {
        match src {
            SourceVal::Immediate(n) => self.asm.emit_movz(n, Reg::X0),
            SourceVal::VReg(vreg) => {
                let src = self.map_reg_use(vreg, idx);
                self.asm.emit(instr::MovReg { src, dest: Reg::X0 });
            }
            SourceVal::String(str_id) => todo!(),
        }

        self.emit_jump(Label::FnRet);
    }

    fn lazy_emit<F, I>(&mut self, label: Label, emit: F)
    where
        F: FnOnce(InstrIndex) -> I + 'static,
        I: Instruction,
    {
        let offset = self.asm.current_offset();
        self.asm.emit_nop();
        self.lazy_emits.push(Box::new(move |emitter| {
            let mapped = emitter.mapped_labels.get(&label).unwrap();
            let instr = emit(*mapped);
            emitter.asm.emit_at(offset, instr);
        }));
    }

    pub fn end(mut self) {
        self.mapped_labels
            .insert(Label::FnRet, self.asm.current_offset());
        for lazy_emit in std::mem::take(&mut self.lazy_emits) {
            lazy_emit(&mut self);
        }
    }
}
