use itertools::Itertools;
use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use crate::analyze::ast::CompareOp;

pub mod codegen;
pub mod lifetime;

#[derive(Default)]
pub struct IR {
    pub items: Vec<Item>,
    pub strings: HashMap<String, StrId>,
}

impl IR {
    pub fn alloc_str_literal(&mut self, string: String) -> StrId {
        let len = self.strings.len();
        *self.strings.entry(string).or_insert(len)
    }
}

pub type StrId = usize;
pub type OpIndex = usize;

pub enum Item {
    Function {
        name: String,
        args: Vec<VirtualReg>,
        body: Vec<BasicBlock>,
    },
}

pub struct Body {
    pub blocks: Vec<BasicBlock>,
}

/// A [basic block](BasicBlock) is a sequence of operations that does not affect control flow. Each basic block
/// contains exactly one [terminator](Terminator), which specifies the next block in the control flow graph (CFG).
/// In addition basic blocks can specify arguments, which allows it to use registers passed on from
/// other basic blocks. This is an alternative to LLVMs phi nodes.
///
/// The next basic block is called the successor, and the previous is called the predecessor. A basic block
/// can have multiple succesors and predecessors.
pub struct BasicBlock {
    pub label: Label,
    pub args: Vec<VirtualReg>,
    pub ops: Vec<Operation>,
    pub terminator: Terminator,
}

impl BasicBlock {
    pub fn successors(&self) -> Vec<Label> {
        match self.terminator {
            Terminator::Branch { label } => vec![label],
            Terminator::BranchCond {
                if_true, if_false, ..
            } => vec![if_true, if_false],
            Terminator::Return { .. } => vec![Label::Ret],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Label {
    Entry,
    Anon(u32),
    Ret,
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entry => write!(f, ".entry"),
            Self::Anon(n) => write!(f, ".L{}", n),
            Self::Ret => write!(f, ".end"),
        }
    }
}

pub type Op = Operation;

#[derive(Debug, Clone, Copy)]
pub enum VarSize {
    Zero,
    B8,
    B16,
    B32,
    B64,
}

#[derive(Debug, Clone)]
pub enum Terminator {
    Branch {
        label: Label,
    },
    BranchCond {
        cond: VirtualReg,
        if_true: Label,
        if_false: Label,
    },
    Return {
        value: SourceVal,
    },
}

#[derive(Debug, Clone)]
pub enum Operation {
    Assign {
        src: SourceVal,
        dest: VirtualReg,
    },
    AddressOf {
        val: VirtualReg,
        dest: VirtualReg,
    },
    LoadPointer {
        ptr: VirtualReg,
        size: VarSize,
        dest: VirtualReg,
    },
    StorePointer {
        src: VirtualReg,
        ptr: VirtualReg,
    },
    Add {
        a: VirtualReg,
        b: VirtualReg,
        dest: VirtualReg,
    },
    Subtract {
        a: VirtualReg,
        b: VirtualReg,
        dest: VirtualReg,
    },
    Multiply {
        a: VirtualReg,
        b: VirtualReg,
        dest: VirtualReg,
    },
    Divide {
        a: VirtualReg,
        b: VirtualReg,
        dest: VirtualReg,
    },
    Compare {
        a: VirtualReg,
        b: VirtualReg,
        cond: Condition,
        dest: VirtualReg,
    },
    Call {
        function: String,
        args: Vec<VirtualReg>,
        dest: Option<VirtualReg>,
    },
}

impl Operation {
    pub fn _vregs_used(&self, _out: &mut Vec<VirtualReg>) {}

    /// Gets the virtual registers used in this operation. Both source and destination registers.
    pub fn vregs_used(&self) -> (HashSet<VirtualReg>, Option<VirtualReg>) {
        let mut used = HashSet::new();
        let mut assigned = None;

        let mut push = |vreg: Option<VirtualReg>| {
            if let Some(vreg) = vreg {
                used.insert(vreg);
            }
        };

        match self {
            Operation::Assign { src, dest } => {
                push(src.reg());
                assigned = Some(*dest);
            }
            Operation::AddressOf { val: _, dest } => {
                push(Some(*dest));
            }
            Operation::LoadPointer { ptr, size: _, dest } => {
                push(Some(*ptr));
                assigned = Some(*dest);
            }
            Operation::StorePointer { src, ptr } => {
                push(Some(*src));
                push(Some(*ptr));
            }

            Operation::Add { a, b, dest }
            | Operation::Subtract { a, b, dest }
            | Operation::Multiply { a, b, dest }
            | Operation::Divide { a, b, dest } => {
                push(Some(*a));
                push(Some(*b));
                assigned = Some(*dest);
            }

            Operation::Compare {
                a,
                b,
                cond: _,
                dest,
            } => {
                push(Some(*a));
                push(Some(*b));
                assigned = Some(*dest);
            }

            Operation::Call {
                dest,
                args,
                function: _,
            } => {
                assigned = *dest;
                for vreg in args {
                    push(Some(*vreg));
                }
            }
        }

        (used, assigned)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Condition {
    Equal,
    NotEqual,
    UnsignedGreaterOrEqual,
    UnsignedLess,
    UnsignedGreater,
    UnsignedLessOrEqual,
    SignedGreaterOrEqual,
    SignedLess,
    SignedGreater,
    SignedLessOrEqual,
    Negative,
    PositiveOrZero,
    Overflow,
    NoOverflow,
    Always,
    Never,
}

impl Condition {
    pub fn from_ast_op(op: CompareOp, signed: bool) -> Self {
        match (op, signed) {
            (CompareOp::Equal, _) => Self::Equal,
            (CompareOp::NotEqual, _) => Self::NotEqual,
            (CompareOp::Less, true) => Self::SignedLess,
            (CompareOp::Less, false) => Self::UnsignedLess,
            (CompareOp::LessOrEqual, true) => Self::SignedLessOrEqual,
            (CompareOp::LessOrEqual, false) => Self::UnsignedLessOrEqual,
            (CompareOp::Greater, true) => Self::SignedGreater,
            (CompareOp::Greater, false) => Self::UnsignedGreater,
            (CompareOp::GreaterOrEqual, true) => Self::SignedGreaterOrEqual,
            (CompareOp::GreaterOrEqual, false) => Self::UnsignedGreaterOrEqual,
        }
    }

    pub fn inverted(&self) -> Condition {
        use Condition::*;

        match self {
            Equal => NotEqual,
            NotEqual => Equal,
            UnsignedGreaterOrEqual => UnsignedLess,
            UnsignedLess => UnsignedGreaterOrEqual,
            UnsignedGreater => UnsignedLessOrEqual,
            UnsignedLessOrEqual => UnsignedGreater,
            SignedGreaterOrEqual => SignedLess,
            SignedLess => SignedGreaterOrEqual,
            SignedGreater => SignedLessOrEqual,
            SignedLessOrEqual => SignedGreater,
            Negative => PositiveOrZero,
            PositiveOrZero => Negative,
            Overflow => NoOverflow,
            NoOverflow => Overflow,
            Always => Never,
            Never => Always,
        }
    }
}

/// A value that can be used in an operation as a source, either an immediate operand or a
/// register.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceVal {
    Immediate(i64),
    VReg(VirtualReg),
    String(StrId),
}

impl SourceVal {
    /// Returns the virtual register if the source value is a register.
    pub fn reg(&self) -> Option<VirtualReg> {
        match self {
            Self::VReg(vreg) => Some(*vreg),
            _ => None,
        }
    }
}

impl fmt::Display for SourceVal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceVal::Immediate(n) => write!(f, "{}", n),
            SourceVal::VReg(vreg) => write!(f, "{}", vreg),
            SourceVal::String(str_id) => write!(f, "string #{}", str_id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VirtualReg(pub u32);

impl fmt::Display for VirtualReg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.0)
    }
}

impl fmt::Display for IR {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (string, id) in self.strings.iter() {
            writeln!(f, "#{} => \"{}\"", id, string)?;
        }

        if !self.strings.is_empty() {
            writeln!(f)?;
        }

        for item in self.items.iter() {
            let Item::Function { name, args, body } = item;
            write!(f, "fn {}(", name)?;
            for reg in args.iter().take(1) {
                write!(f, "{}", reg)?;
            }

            for reg in args.iter().skip(1) {
                write!(f, ", {}", reg)?;
            }

            writeln!(f, ") {{")?;

            for block in body {
                let predecessors: Vec<_> = body
                    .iter()
                    .filter_map(|bb| {
                        if bb.successors().contains(&block.label) {
                            Some(bb.label)
                        } else {
                            None
                        }
                    })
                    .collect();

                writeln!(
                    f,
                    "{} ({})  # {}",
                    block.label,
                    block.args.iter().join(", "),
                    predecessors.iter().join(", ")
                )?;

                for op in block.ops.iter() {
                    match op {
                        Operation::Assign { src, dest } => writeln!(f, "    {} = {}", dest, src)?,
                        Operation::AddressOf { val, dest } => {
                            writeln!(f, "    {} = ref {}", dest, val)?
                        }
                        Operation::LoadPointer { ptr, size, dest } => {
                            writeln!(f, "    {} = deref {:?} {}", dest, size, ptr)?
                        }
                        Operation::StorePointer { src, ptr } => {
                            writeln!(f, "    deref {} = {}", ptr, src)?
                        }

                        Operation::Add { a, b, dest } => {
                            writeln!(f, "    {} = {} + {}", dest, a, b)?
                        }
                        Operation::Subtract { a, b, dest } => {
                            writeln!(f, "    {} = {} - {}", dest, a, b)?
                        }
                        Operation::Multiply { a, b, dest } => {
                            writeln!(f, "    {} = {} * {}", dest, a, b)?
                        }
                        Operation::Divide { a, b, dest } => {
                            writeln!(f, "    {} = {} / {}", dest, a, b)?
                        }
                        Operation::Compare { a, b, cond, dest } => {
                            writeln!(f, "    {} = cmp {} {:?} {}", dest, a, cond, b)?
                        }
                        Operation::Call {
                            function,
                            args,
                            dest,
                        } => {
                            if let Some(dest) = dest {
                                write!(f, "    {} = call {}(", dest, function)?
                            } else {
                                write!(f, "    call {}(", function)?
                            }

                            for arg in args.iter().take(1) {
                                write!(f, "{}", arg)?;
                            }

                            for arg in args.iter().skip(1) {
                                write!(f, ", {}", arg)?;
                            }

                            writeln!(f, ")")?
                        }
                    }
                }

                match block.terminator {
                    Terminator::Branch { label } => writeln!(f, "    goto {}", label)?,
                    Terminator::BranchCond {
                        cond,
                        if_true,
                        if_false,
                    } => writeln!(
                        f,
                        "    br cond {} [\n      true => {}\n      false => {}\n    ]",
                        cond, if_true, if_false
                    )?,
                    Terminator::Return { value } => writeln!(f, "    ret {}", value)?,
                }

                writeln!(f)?;
            }

            writeln!(f, "}}\n")?;
        }

        Ok(())
    }
}
