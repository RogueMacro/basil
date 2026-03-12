use std::collections::HashMap;

use crate::{
    analyze::{
        ast::{ArithmeticOp, ExprType, Expression, Item as AstItem, Statement},
        semantics::{Sign, ValidAST},
    },
    ir::{BasicBlock, Condition, IR, Item, Label, Op, OpIndex, SourceVal, VirtualReg},
};

impl IR {
    pub fn generate(ast: ValidAST) -> IR {
        let ast = ast.0;

        let mut items = Vec::new();

        for item in ast.items {
            if let AstItem::Function {
                name, body, args, ..
            } = item
            {
                let mut block_builder = BlockBuilder::new();
                let args = args
                    .iter()
                    .map(|(arg, _, _)| block_builder.get_or_insert_vreg(arg))
                    .collect();

                items.push(Item::Function {
                    name,
                    args,
                    bb: block_builder.build(body),
                });
            };
        }

        IR { items }
    }
}

struct BlockBuilder {
    vregs: HashMap<String, VirtualReg>,
    vreg_counter: u32,
    labels: HashMap<OpIndex, Vec<Label>>,
    label_counter: u32,
    ops: Vec<Op>,
}

impl BlockBuilder {
    pub fn new() -> Self {
        Self {
            vregs: HashMap::new(),
            vreg_counter: 0,
            labels: HashMap::new(),
            label_counter: 0,
            ops: Vec::new(),
        }
    }

    pub fn build(mut self, block: Vec<Statement>) -> BasicBlock {
        self.consume_block(block);
        BasicBlock {
            ops: self.ops,
            labels: self.labels,
        }
    }

    fn consume_block(&mut self, block: Vec<Statement>) {
        self.ops.reserve(block.len());

        for stmt in block {
            match stmt {
                Statement::Declare { var, expr, .. } | Statement::Assign { var, expr, .. } => {
                    assert!(!self.vregs.contains_key(&var), "variable declared twice");

                    let dest = self.get_or_insert_vreg(var);
                    let src = self.unroll_expr(&expr, Some(dest));

                    if src != SourceVal::VReg(dest) {
                        self.ops.push(Op::Assign { src, dest });
                    }
                }
                Statement::Return(expr) => {
                    let value = self.unroll_expr(&expr, None);
                    self.ops.push(Op::Return { value });
                }
                Statement::If { guard, body } => {
                    let cond = self.unroll_expr(&guard, None);
                    let cond = self.src_to_vreg(cond);
                    let label = self.reserve_label();
                    self.ops.push(Op::BranchIfFalse { cond, label });

                    // let sub_vregs = self
                    //     .vregs
                    //     .iter()
                    //     .map(|(k, v)| (k.to_owned(), VirtualReg(v.0 * 2)))
                    //     .collect();

                    let outer_vregs = std::mem::take(&mut self.vregs);
                    let outer_vreg_counter = std::mem::take(&mut self.vreg_counter);

                    self.consume_block(body);

                    for (name, inner) in self.vregs.iter() {
                        if let Some(outer) = outer_vregs.get(name)
                            && inner != outer
                        {
                            self.ops.push(Op::Assign {
                                src: SourceVal::VReg(*inner),
                                dest: *outer,
                            });
                        }
                    }

                    self.set_label_here(label);

                    self.vregs = outer_vregs;
                    self.vreg_counter = outer_vreg_counter;
                }
                Statement::Expr(expr) => {
                    self.unroll_expr(&expr, None);
                }
            }
        }
    }

    fn unroll_expr(&mut self, expr: &Expression, dest: Option<VirtualReg>) -> SourceVal {
        match &expr.expr_type {
            ExprType::Const(num) => SourceVal::Immediate(*num),
            ExprType::Character(c) => SourceVal::Immediate(*c as i64),
            ExprType::Bool(b) => SourceVal::Immediate(*b as i64),
            ExprType::Variable(var, ..) => SourceVal::VReg(self.expect_vreg(var)),
            ExprType::Arithmetic(expr1, expr2, op, _sign) => {
                // TODO: sign
                let a = self.unroll_expr(expr1.as_ref(), None);
                let b = self.unroll_expr(expr2.as_ref(), None);

                let a = self.src_to_vreg(a);
                let b = self.src_to_vreg(b);

                let dest = dest.unwrap_or_else(|| self.get_vreg());

                match op {
                    ArithmeticOp::Add => self.ops.push(Op::Add { a, b, dest }),
                    ArithmeticOp::Sub => self.ops.push(Op::Subtract { a, b, dest }),
                    ArithmeticOp::Mult => self.ops.push(Op::Multiply { a, b, dest }),
                    ArithmeticOp::Div => self.ops.push(Op::Divide { a, b, dest }),
                }

                SourceVal::VReg(dest)
            }
            ExprType::Comparison(expr1, expr2, op, sign) => {
                let expr1 = self.unroll_expr(expr1, None);
                let expr2 = self.unroll_expr(expr2, None);

                let expr1 = self.src_to_vreg(expr1);
                let expr2 = self.src_to_vreg(expr2);

                let dest = dest.unwrap_or_else(|| self.get_vreg());

                self.ops.push(Op::Compare {
                    a: expr1,
                    b: expr2,
                    cond: Condition::from_ast_op(*op, matches!(sign, Some(Sign::Signed))),
                    dest,
                });

                SourceVal::VReg(dest)
            }
            ExprType::FnCall(function, args) => {
                let args = args
                    .iter()
                    .map(|e| {
                        let src = self.unroll_expr(e, None);
                        self.src_to_vreg(src)
                    })
                    .collect();

                let dest = dest.unwrap_or_else(|| self.get_vreg());
                self.ops.push(Op::Call {
                    function: function.clone(),
                    args,
                    dest: Some(dest),
                });

                SourceVal::VReg(dest)
            }
        }
    }

    fn get_or_insert_vreg<S: Into<String> + AsRef<str>>(&mut self, var: S) -> VirtualReg {
        if let Some(&vreg) = self.vregs.get(var.as_ref()) {
            vreg
        } else {
            let vreg = self.get_vreg();
            self.vregs.insert(var.into(), vreg);
            vreg
        }
    }

    fn expect_vreg(&self, var: &str) -> VirtualReg {
        *self
            .vregs
            .get(var)
            .expect("undefined variable (compiler bug)")
    }

    fn get_vreg(&mut self) -> VirtualReg {
        let vreg = VirtualReg(self.vreg_counter);
        self.vreg_counter += 1;
        vreg
    }

    fn src_to_vreg(&mut self, src: SourceVal) -> VirtualReg {
        match src {
            SourceVal::Immediate(_) => {
                let dest = self.get_vreg();
                self.ops.push(Op::Assign { src, dest });
                dest
            }
            SourceVal::VReg(vreg) => vreg,
        }
    }

    fn reserve_label(&mut self) -> Label {
        self.label_counter += 1;
        Label(self.label_counter - 1)
    }

    fn set_label_here(&mut self, label: Label) {
        self.labels.entry(self.ops.len()).or_default().push(label);
    }
}
