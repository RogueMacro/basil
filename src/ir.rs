use std::{collections::HashMap, fmt};

use crate::{
    analyze::ast::CompareOp,
    ir::lifetime::{Interval, Lifetime},
};

pub mod codegen;
pub mod lifetime;

pub struct IR {
    pub items: Vec<Item>,
}

pub enum Item {
    Function {
        name: String,
        args: Vec<VirtualReg>,
        bb: BasicBlock,
    },
}

pub type OpIndex = usize;

pub struct BasicBlock {
    pub labels: HashMap<OpIndex, Vec<Label>>,
    pub ops: Vec<Operation>,
}

impl BasicBlock {
    /// Generates a registry mapping virtual registers to a lifetime.
    pub fn lifetimes(&self) -> HashMap<VirtualReg, Lifetime> {
        let mut lifetimes: HashMap<VirtualReg, Lifetime> = HashMap::new();
        let mut active: Vec<(VirtualReg, Interval)> = Vec::new();
        let mut uses = Vec::new();

        for (i, op) in self.ops.iter().enumerate() {
            uses.clear();
            op.vregs_used(&mut uses);

            active.retain_mut(|(vreg, interval)| {
                if let Some(u) = uses.iter().position(|r| r == vreg) {
                    uses.swap_remove(u);
                    interval.range.end = i + 1;

                    true
                } else {
                    let lifetime = lifetimes.entry(*vreg).or_default();
                    lifetime.insert_interval(interval.clone());

                    false
                }
            });

            // existing uses removed in previous step
            for vreg in &uses {
                let interval = Interval {
                    range: i..(i + 1),
                    register: None,
                };

                active.push((*vreg, interval));
            }
        }

        for (vreg, interval) in active {
            let lifetime = lifetimes.entry(vreg).or_default();
            lifetime.insert_interval(interval.clone());
        }

        lifetimes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label(u32);

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ".L{}", self.0)
    }
}

pub type Op = Operation;

#[derive(Debug, Clone)]
pub enum Operation {
    Assign {
        src: SourceVal,
        dest: VirtualReg,
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
    BranchIfFalse {
        cond: VirtualReg,
        label: Label,
    },
    Return {
        value: SourceVal,
    },
    Call {
        function: String,
        args: Vec<VirtualReg>,
        dest: Option<VirtualReg>,
    },
}

impl Operation {
    /// Gets the virtual registers used in this operation. Both source and destination registers.
    pub fn vregs_used(&self, out: &mut Vec<VirtualReg>) {
        let mut push = |vreg: Option<VirtualReg>| {
            if let Some(vreg) = vreg
                && !out.contains(&vreg)
            {
                out.push(vreg);
            }
        };

        match self {
            Operation::Assign { src, dest } => {
                push(src.reg());
                push(Some(*dest));
            }
            Operation::Add { a, b, dest } | Operation::Subtract { a, b, dest } => {
                // push(a.reg());
                // push(b.reg());
                push(Some(*a));
                push(Some(*b));
                push(Some(*dest));
            }
            Operation::Multiply { a, b, dest } | Operation::Divide { a, b, dest } => {
                push(Some(*a));
                push(Some(*b));
                push(Some(*dest));
            }

            Operation::Compare {
                a,
                b,
                cond: _,
                dest,
            } => {
                push(Some(*a));
                push(Some(*b));
                push(Some(*dest));
            }
            Operation::BranchIfFalse { cond, label: _ } => {
                push(Some(*cond));
            }

            Operation::Return { value } => push(value.reg()),
            Operation::Call {
                dest,
                args,
                function: _,
            } => {
                push(*dest);
                for vreg in args {
                    push(Some(*vreg));
                }
            }
        }
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
        for item in self.items.iter() {
            let Item::Function { name, args, bb } = item;
            write!(f, "fn {}(", name)?;
            for reg in args.iter().take(1) {
                write!(f, "{}", reg)?;
            }

            for reg in args.iter().skip(1) {
                write!(f, ", {}", reg)?;
            }

            writeln!(f, ") {{")?;

            for (i, op) in bb.ops.iter().enumerate() {
                if let Some(labels) = bb.labels.get(&i) {
                    for label in labels {
                        write!(f, "{}", label)?;
                    }
                    writeln!(f)?;
                }

                match op {
                    Operation::Assign { src, dest } => writeln!(f, "    {} = {}", dest, src)?,
                    Operation::Add { a, b, dest } => writeln!(f, "    {} = {} + {}", dest, a, b)?,
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
                    Operation::BranchIfFalse { cond, label } => {
                        writeln!(f, "    if not {} goto {}", cond, label)?;
                    }
                    Operation::Return { value } => writeln!(f, "    ret {}", value)?,
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

            writeln!(f, "}}\n")?;
        }

        Ok(())
    }
}
