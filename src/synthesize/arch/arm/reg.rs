use std::collections::{BTreeMap, HashMap};

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use strum::EnumIter;
use ux::u12;

use crate::{
    ir::{BasicBlock, Op, VirtualReg},
    synthesize::arch::arm::{
        ArmAssembler,
        instr::{self, Input},
    },
};

pub type Reg = Register;

/// All general-purpose registers + stack pointer on the ARM architecture.
#[repr(u32)]
#[derive(EnumIter, FromPrimitive, ToPrimitive, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Register {
    X0 = 0,   // 1st argument / return value
    X1 = 1,   // 2nd argument
    X2 = 2,   // 3rd argument
    X3 = 3,   // 4th argument
    X4 = 4,   // 5th argument
    X5 = 5,   // 6th argument
    X6 = 6,   // 7th argument
    X7 = 7,   // 8th argument
    X8 = 8,   // indirect result
    X9 = 9,   // caller-saved
    X10 = 10, // caller-saved
    X11 = 11, // caller-saved
    X12 = 12, // caller-saved
    X13 = 13, // caller-saved
    X14 = 14, // caller-saved
    X15 = 15, // caller-saved
    X16 = 16, // IP0
    X17 = 17, // IP1
    X18 = 18, // platform register
    X19 = 19, // callee-saved
    X20 = 20, // callee-saved
    X21 = 21, // callee-saved
    X22 = 22, // callee-saved
    X23 = 23, // callee-saved
    X24 = 24, // callee-saved
    X25 = 25, // callee-saved
    X26 = 26, // callee-saved
    X27 = 27, // callee-saved
    X28 = 28, // callee-saved
    FP = 29,  // frame pointer (X29)
    LR = 30,  // link register (X30)
    SP = 31,  // stack pointer (X31) (not general purpose)
}

/// Used in register allocation when mapping a virtual register to a physical register. This
/// protects a register if using the register for a value requires loading that value from stack,
/// saving the existing register value to the stack, or both.
///
/// To use this register in an operation, call the [unwrap()](Self::unwrap) method.
#[derive(Debug, Clone, Copy)]
pub enum RegisterGuard {
    Ready(Register),
    Load(u12, Register),
    Save(u12, Register),
    /// (save to, load from, reg)
    SaveAndLoad(u12, u12, Register),
}

impl RegisterGuard {
    /// Returns the register that this guard protects.
    ///
    /// Do not use this register if you are not sure it doesn't overwrite a value and where the
    /// virtual register is located.
    pub fn inner_reg(&self) -> Register {
        match *self {
            RegisterGuard::Ready(reg) => reg,
            RegisterGuard::Load(_, reg) => reg,
            RegisterGuard::Save(_, reg) => reg,
            RegisterGuard::SaveAndLoad(_, _, reg) => reg,
        }
    }

    /// Unwraps the inner register by potentially emitting a load and/or store instruction. Calling
    /// this function will ensure the value ends up in the returned register, and that the old
    /// value in the register is saved to the stack if necessary.
    pub fn unwrap(&self, asm: &mut ArmAssembler) -> Register {
        match *self {
            Self::Ready(reg) => reg,
            Self::Load(stack_offset, dest) => {
                asm.emit(instr::Load { stack_offset, dest });
                dest
            }
            Self::Save(stack_offset, dest) => {
                asm.emit_store(stack_offset, dest);
                dest
            }
            Self::SaveAndLoad(save_to, load_from, dest) => {
                asm.emit(instr::Store {
                    base: Reg::SP,
                    offset: Input::Imm(save_to),
                    register: dest,
                });

                asm.emit(instr::Load {
                    stack_offset: load_from,
                    dest,
                });
                dest
            }
        }
    }
}

/// Location of a virtual register at any given time.
#[derive(Debug, Clone, Copy)]
pub enum Location {
    Register(Register),
    Stack(u12),
}

#[derive(Default)]
struct Stack {
    stack_size: u12,
    free_slots: Vec<u12>,
}

impl Stack {
    pub fn alloc(&mut self, bytes: u16) -> u12 {
        if let Some(slot) = self.free_slots.pop() {
            slot
        } else {
            self.stack_size = self.stack_size + u12::new(bytes / 4);
            self.stack_size - u12::new(bytes / 4)
        }
    }

    pub fn free(&mut self, slot: u12) {
        assert!(slot < self.stack_size);
        assert!(!self.free_slots.contains(&slot));

        self.free_slots.push(slot);
    }
}

use Register::*;
const CALLER_SAVED_REGS: &[Register] = &[
    X0, X1, X2, X3, X4, X5, X6, X7, X8, X9, X10, X11, X12, X13, X14, X15,
];

/// Allocates physical registers for each virtual register at any given instruction.
///
/// # Returns
///
/// The generated allocation map ([Allocator])
pub fn allocate(bb: &BasicBlock, args: &[VirtualReg]) -> Allocator {
    // TODO: Use callee-saved registers

    // General-purpose caller-saved registers
    let mut phys_regs: Vec<Register> = CALLER_SAVED_REGS.to_vec();

    let mut location_map: BTreeMap<VirtualReg, Location> = args
        .iter()
        .enumerate()
        .map(|(i, &vreg)| (vreg, Location::Register(Register::from_usize(i).unwrap())))
        .collect();

    let mut lifetimes = bb.lifetimes();
    let lifetimes_imm = lifetimes.clone();

    let last_uses: Vec<(VirtualReg, usize)> = lifetimes
        .iter()
        .map(|(vreg, lifetime)| (*vreg, lifetime.end().unwrap() - 1))
        .collect();

    let mut regmap = RegMap::new();
    let mut stack = Stack::default();
    let mut stack_saves: HashMap<usize, Vec<(Register, u12)>> = HashMap::new();

    for (op_idx, op) in bb.ops.iter().enumerate() {
        let mut retired_regs = Vec::new();

        if let Op::Call { .. } = op {
            let regs_to_save: Vec<(Register, u12)> = location_map
                .values_mut()
                .filter_map(|l| match *l {
                    Location::Register(r) => {
                        let stack_offset = stack.alloc(8);
                        *l = Location::Stack(stack_offset);
                        Some((r, stack_offset))
                    }
                    _ => None,
                })
                .collect();

            if !regs_to_save.is_empty() {
                stack_saves.insert(op_idx, regs_to_save);
                phys_regs.clear();
                phys_regs.extend_from_slice(CALLER_SAVED_REGS);
            }
        }

        for (&vreg, lifetime) in lifetimes.iter_mut() {
            if let Some(interval) = lifetime.at_mut(op_idx) {
                // This vreg overlaps (is active) at this op index
                let reg_guard: RegisterGuard =
                    match (interval.register, location_map.get(&vreg).copied()) {
                        (Some(reg), _) => {
                            // This interval has already been allocated.
                            interval.register = Some(reg);
                            RegisterGuard::Ready(Register::from_u32(reg).unwrap())
                        }
                        (None, Some(Location::Register(reg))) => {
                            // This value already exists in a register from a previous allocation.
                            interval.register = Some(reg as u32);
                            RegisterGuard::Ready(reg)
                        }

                        (None, location) => {
                            let location = location.map(|l| {
                                let Location::Stack(offset) = l else {
                                    unreachable!()
                                };
                                offset
                            });

                            match (phys_regs.pop(), location) {
                                (Some(reg), Some(offset)) => {
                                    interval.register = Some(reg as u32);
                                    RegisterGuard::Load(offset, reg)
                                }
                                (Some(reg), None) => {
                                    interval.register = Some(reg as u32);
                                    RegisterGuard::Ready(reg)
                                }
                                (None, location) => {
                                    // All physical registers busy, look for dead virtual registers before pushing
                                    // one onto the stack.

                                    let mut dead_vreg = None;

                                    for (vreg, loc) in location_map.iter() {
                                        if let Location::Register(reg) = loc {
                                            let (_, end) =
                                                last_uses.iter().find(|(v, _)| v == vreg).unwrap();
                                            if *end <= op_idx {
                                                dead_vreg = Some((*vreg, *reg));
                                                break;
                                            }
                                        }
                                    }

                                    match (dead_vreg, location) {
                                        (Some((dead_vreg, reg)), Some(offset)) => {
                                            retired_regs.push(dead_vreg);
                                            interval.register = Some(reg as u32);
                                            RegisterGuard::Load(offset, reg)
                                        }
                                        (Some((dead_vreg, reg)), None) => {
                                            retired_regs.push(dead_vreg);
                                            interval.register = Some(reg as u32);
                                            RegisterGuard::Ready(reg)
                                        }
                                        (None, location) => {
                                            // All busy virtual registers are to be used in the future.
                                            // Therefor we must save one register to the stack (preferrably the one to be
                                            // used last) that is not currently overlapping.

                                            let (&furthest_use_vreg, &reg, _) = location_map
                                                .iter()
                                                .filter_map(|(vreg, loc)| {
                                                    if let Location::Register(reg) = loc {
                                                        let next_use = lifetimes_imm
                                                            .get(vreg)
                                                            .unwrap()
                                                            .next_use_after(op_idx)
                                                            .unwrap_or(usize::MAX);

                                                        Some((vreg, reg, next_use))
                                                    } else {
                                                        None
                                                    }
                                                })
                                                .max_by_key(|(_, _, next_use)| *next_use)
                                                .unwrap();

                                            location_map.insert(
                                                furthest_use_vreg,
                                                Location::Stack(stack.alloc(8)),
                                            );
                                            interval.register = Some(reg as u32);

                                            if let Some(offset) = location {
                                                RegisterGuard::SaveAndLoad(
                                                    stack.alloc(8),
                                                    offset,
                                                    reg,
                                                )
                                            } else {
                                                RegisterGuard::Save(stack.alloc(8), reg)
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    };

                match reg_guard {
                    RegisterGuard::Load(slot, _) | RegisterGuard::SaveAndLoad(_, slot, _) => {
                        stack.free(slot);
                    }
                    _ => (),
                }

                location_map.insert(vreg, Location::Register(reg_guard.inner_reg()));
                regmap.insert((vreg, op_idx), reg_guard);
            }
        }

        for vreg in retired_regs {
            location_map.remove(&vreg);
        }
    }

    // TODO
    // difference between dest/src registers:
    // dead src reg can only be reused for a dest reg
    // on the same instruction, not another src reg.

    // crate::ir::lifetime::print_lifetimes(&lifetimes);

    Allocator {
        regmap,
        stack_size: stack.stack_size,
        stack_saves,
    }
}

/// A map from (vreg, instruction position) to a [RegisterGuard].
type RegMap = HashMap<(VirtualReg, usize), RegisterGuard>;

#[derive(Debug, Default)]
pub struct Allocator {
    regmap: RegMap,
    stack_size: u12,
    stack_saves: HashMap<usize, Vec<(Register, u12)>>,
}

impl Allocator {
    pub fn map(&self, vreg: VirtualReg, instr_index: usize) -> RegisterGuard {
        *self.regmap.get(&(vreg, instr_index)).unwrap_or_else(|| {
            panic!(
                "no physical register mapped to {} at index {}",
                vreg, instr_index
            )
        })
    }

    pub fn stack_size(&self) -> u12 {
        self.stack_size
    }

    pub fn stack_save(&self, instr_index: usize) -> Option<&Vec<(Register, u12)>> {
        self.stack_saves.get(&instr_index)
    }
}

#[cfg(test)]
mod tests {
    use ux::u12;

    use super::*;
    use crate::ir::{BasicBlock, Operation, SourceVal, VirtualReg};

    /// Constructs a [BasicBlock] from a list of operations for use in tests.
    fn make_bb(ops: Vec<Operation>) -> BasicBlock {
        BasicBlock { ops }
    }

    // ---- Stack ----

    #[test]
    fn stack_alloc_starts_at_zero() {
        let mut stack = Stack::default();
        assert_eq!(stack.stack_size, u12::new(0));
        let slot = stack.alloc(8);
        assert_eq!(slot, u12::new(0));
        assert_eq!(stack.stack_size, u12::new(1));
    }

    #[test]
    fn stack_alloc_increments_each_call() {
        let mut stack = Stack::default();
        assert_eq!(stack.alloc(8), u12::new(0));
        assert_eq!(stack.alloc(8), u12::new(1));
        assert_eq!(stack.alloc(8), u12::new(2));
        assert_eq!(stack.stack_size, u12::new(3));
    }

    #[test]
    fn stack_free_recycles_slot() {
        let mut stack = Stack::default();
        let s0 = stack.alloc(8);
        let _s1 = stack.alloc(8);
        stack.free(s0);
        // stack_size does not shrink
        assert_eq!(stack.stack_size, u12::new(2));
        // next alloc reuses the freed slot
        let reused = stack.alloc(8);
        assert_eq!(reused, s0);
        assert_eq!(stack.stack_size, u12::new(2));
    }

    #[test]
    #[should_panic]
    fn stack_free_out_of_bounds_panics() {
        let mut stack = Stack::default();
        stack.free(u12::new(0)); // nothing allocated yet
    }

    #[test]
    #[should_panic]
    fn stack_double_free_panics() {
        let mut stack = Stack::default();
        let slot = stack.alloc(8);
        stack.free(slot);
        stack.free(slot); // second free of the same slot
    }

    // ---- allocate() ----

    #[test]
    fn allocate_empty_block_has_zero_stack_size() {
        let alloc = allocate(&make_bb(vec![]), &[]);
        assert_eq!(alloc.stack_size(), u12::new(0));
    }

    #[test]
    fn allocate_single_vreg_uses_same_register() {
        // v0 is defined at op 0 and used at op 1; both uses should map to the
        // same physical register with no spill.
        let bb = make_bb(vec![
            Operation::Assign {
                src: SourceVal::Immediate(42),
                dest: VirtualReg(0),
            },
            Operation::Return {
                value: SourceVal::VReg(VirtualReg(0)),
            },
        ]);
        let alloc = allocate(&bb, &[]);
        assert_eq!(alloc.stack_size(), u12::new(0));
        let g0 = alloc.map(VirtualReg(0), 0);
        let g1 = alloc.map(VirtualReg(0), 1);
        assert!(matches!(g0, RegisterGuard::Ready(_)));
        assert!(matches!(g1, RegisterGuard::Ready(_)));
        assert_eq!(g0.inner_reg(), g1.inner_reg());
    }

    #[test]
    fn allocate_simultaneously_live_vregs_get_distinct_registers() {
        // v0 and v1 are both live at the Add (op 2), so they must occupy
        // different physical registers.
        let bb = make_bb(vec![
            Operation::Assign {
                src: SourceVal::Immediate(1),
                dest: VirtualReg(0),
            },
            Operation::Assign {
                src: SourceVal::Immediate(2),
                dest: VirtualReg(1),
            },
            Operation::Add {
                a: SourceVal::VReg(VirtualReg(0)),
                b: SourceVal::VReg(VirtualReg(1)),
                dest: VirtualReg(2),
            },
            Operation::Return {
                value: SourceVal::VReg(VirtualReg(2)),
            },
        ]);
        let alloc = allocate(&bb, &[]);
        assert_eq!(alloc.stack_size(), u12::new(0));
        let g0 = alloc.map(VirtualReg(0), 2);
        let g1 = alloc.map(VirtualReg(1), 2);
        let g2 = alloc.map(VirtualReg(2), 2);
        assert!(matches!(g0, RegisterGuard::Ready(_)));
        assert!(matches!(g1, RegisterGuard::Ready(_)));
        assert!(matches!(g2, RegisterGuard::Ready(_)));
        assert_ne!(g0.inner_reg(), g1.inner_reg());
        assert_ne!(g0.inner_reg(), g2.inner_reg());
        assert_ne!(g1.inner_reg(), g2.inner_reg());
    }

    #[test]
    fn allocate_uses_caller_saved_registers() {
        let bb = make_bb(vec![
            Operation::Assign {
                src: SourceVal::Immediate(5),
                dest: VirtualReg(0),
            },
            Operation::Return {
                value: SourceVal::VReg(VirtualReg(0)),
            },
        ]);
        let alloc = allocate(&bb, &[]);
        let reg = alloc.map(VirtualReg(0), 0).inner_reg();
        assert!(CALLER_SAVED_REGS.contains(&reg));
    }

    // ---- Allocator::stack_save ----

    #[test]
    fn allocator_stack_save_none_for_non_call_ops() {
        let bb = make_bb(vec![
            Operation::Assign {
                src: SourceVal::Immediate(1),
                dest: VirtualReg(0),
            },
            Operation::Return {
                value: SourceVal::VReg(VirtualReg(0)),
            },
        ]);
        let alloc = allocate(&bb, &[]);
        assert!(alloc.stack_save(0).is_none());
        assert!(alloc.stack_save(1).is_none());
    }

    #[test]
    fn allocator_stack_save_some_for_call_when_registers_are_live() {
        // v0 is assigned before a Call, so its register must be preserved.
        let bb = make_bb(vec![
            Operation::Assign {
                src: SourceVal::Immediate(1),
                dest: VirtualReg(0),
            },
            Operation::Call {
                function: String::from("foo"),
                args: vec![],
                dest: None,
            },
            Operation::Return {
                value: SourceVal::VReg(VirtualReg(0)),
            },
        ]);
        let alloc = allocate(&bb, &[]);
        let saves = alloc.stack_save(1);
        assert!(saves.is_some());
        assert!(!saves.unwrap().is_empty());
    }

    #[test]
    fn allocator_stack_save_none_for_call_when_no_registers_are_live() {
        // No vregs have been assigned before the Call, so nothing needs saving.
        let bb = make_bb(vec![Operation::Call {
            function: String::from("foo"),
            args: vec![],
            dest: None,
        }]);
        let alloc = allocate(&bb, &[]);
        assert!(alloc.stack_save(0).is_none());
    }

    // ---- Allocator::map ----

    #[test]
    #[should_panic]
    fn allocator_map_panics_for_unknown_vreg() {
        let alloc = allocate(&make_bb(vec![]), &[]);
        alloc.map(VirtualReg(99), 0);
    }
}
