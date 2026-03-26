use std::collections::HashMap;

use crate::{
    analyze::{
        ast::{ArithmeticOp, Assignable, ExprInner, Expression, Item as AstItem, Statement},
        semantics::{Sign, ValidAST},
    },
    ir::{BasicBlock, Condition, IR, Item, Label, Op, OpIndex, SourceVal, VirtualReg},
};

impl IR {
    pub fn generate(ast: ValidAST) -> IR {
        let ast = ast.0;

        let mut ir = IR::default();

        for item in ast.items {
            if let AstItem::Function {
                name, body, args, ..
            } = item
            {
                let mut block_builder = BlockBuilder::new(&mut ir);
                let args = args
                    .iter()
                    .map(|(arg, _, _)| block_builder.get_or_insert_vreg(arg))
                    .collect();

                let bb = block_builder.build(body);
                ir.items.push(Item::Function { name, args, bb });
            };
        }

        ir
    }
}

struct BlockBuilder<'ir> {
    vregs: HashMap<String, VirtualReg>,
    vreg_counter: u32,
    labels: HashMap<OpIndex, Vec<Label>>,
    label_counter: u32,
    ops: Vec<Op>,
    ir: &'ir mut IR,
}

impl<'ir> BlockBuilder<'ir> {
    pub fn new(ir: &'ir mut IR) -> Self {
        Self {
            vregs: HashMap::new(),
            vreg_counter: 0,
            labels: HashMap::new(),
            label_counter: 0,
            ops: Vec::new(),
            ir,
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
                Statement::Declare { var, expr, .. } => {
                    assert!(!self.vregs.contains_key(&var), "variable declared twice");

                    let dest = self.get_or_insert_vreg(var);
                    let src = self.unroll_expr(expr, Some(dest));

                    if src != SourceVal::VReg(dest) {
                        self.ops.push(Op::Assign { src, dest });
                    }
                }
                Statement::Assign { var, expr, .. } => {
                    let dest = self.get_or_insert_vreg(var.symbol());
                    let src = self.unroll_expr(expr, Some(dest));

                    match var {
                        Assignable::Var(_) => {
                            if src.reg() != Some(dest) {
                                self.ops.push(Op::Assign { src, dest })
                            }
                        }
                        Assignable::Ptr(_) => {
                            let src = self.src_to_vreg(src);
                            self.ops.push(Op::StorePointer { src, ptr: dest });
                        }
                    }
                }
                Statement::Return(expr) => {
                    let value = self.unroll_expr(expr, None);
                    self.ops.push(Op::Return { value });
                }

                Statement::If { guard, body } => {
                    let cond = self.unroll_expr(guard, None);
                    let cond = self.src_to_vreg(cond);
                    let label = self.reserve_label();
                    self.ops.push(Op::BranchIfNot { cond, label });

                    let outer_vregs = self.vregs.clone();
                    let outer_vreg_counter = self.vreg_counter;

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

                Statement::WhileLoop { guard, body } => {
                    let cond_label = self.reserve_label();

                    let outer_vregs = self.vregs.clone();
                    let outer_vreg_counter = self.vreg_counter;

                    self.ops.push(Op::Branch { label: cond_label });

                    let body_label = self.insert_label();
                    self.consume_block(body);

                    self.set_label_here(cond_label);
                    let cond = self.unroll_expr(guard, None);
                    let cond = self.src_to_vreg(cond);
                    self.ops.push(Op::BranchIf {
                        cond,
                        label: body_label,
                    });

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

                    self.vregs = outer_vregs;
                    self.vreg_counter = outer_vreg_counter;
                }

                Statement::Expr(expr) => {
                    self.unroll_expr(expr, None);
                }
            }
        }
    }

    fn unroll_expr(&mut self, expr: Expression, dest: Option<VirtualReg>) -> SourceVal {
        match expr.inner {
            ExprInner::Const(num) => SourceVal::Immediate(num),
            ExprInner::Character(c) => SourceVal::Immediate(c as i64),
            ExprInner::String(string) => {
                let str_id = self.ir.alloc_str(string);
                SourceVal::String(str_id)
            }
            ExprInner::Bool(b) => SourceVal::Immediate(b as i64),

            ExprInner::Variable(var) => SourceVal::VReg(self.expect_vreg(&var)),
            ExprInner::Pointer(var) => {
                let val = self.expect_vreg(&var);
                let dest = dest.unwrap_or_else(|| self.get_vreg());

                self.ops.push(Op::AddressOf { val, dest });
                SourceVal::VReg(dest)
            }
            ExprInner::Deref(var) => {
                let ptr = self.expect_vreg(&var);
                let dest = dest.unwrap_or_else(|| self.get_vreg());

                self.ops.push(Op::LoadPointer { ptr, dest });
                SourceVal::VReg(dest)
            }

            ExprInner::Arithmetic(expr1, expr2, op, _sign) => {
                // TODO: sign
                let a = self.unroll_expr(*expr1, None);
                let b = self.unroll_expr(*expr2, None);

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
            ExprInner::Comparison(expr1, expr2, op, sign) => {
                let expr1 = self.unroll_expr(*expr1, None);
                let expr2 = self.unroll_expr(*expr2, None);

                let expr1 = self.src_to_vreg(expr1);
                let expr2 = self.src_to_vreg(expr2);

                let dest = dest.unwrap_or_else(|| self.get_vreg());

                self.ops.push(Op::Compare {
                    a: expr1,
                    b: expr2,
                    cond: Condition::from_ast_op(op, matches!(sign, Some(Sign::Signed))),
                    dest,
                });

                SourceVal::VReg(dest)
            }

            ExprInner::FnCall(function, args) => {
                let args = args
                    .into_iter()
                    .map(|e| {
                        let src = self.unroll_expr(e, None);
                        self.src_to_vreg(src)
                    })
                    .collect();

                self.ops.push(Op::Call {
                    function: function.clone(),
                    args,
                    dest,
                });

                if let Some(dest) = dest {
                    SourceVal::VReg(dest)
                } else {
                    SourceVal::Immediate(0)
                }
            }

            ExprInner::Cast(expr, _typ) => self.unroll_expr(*expr, dest),
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
            .unwrap_or_else(|| panic!("undefined variable '{}'", var))
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
            SourceVal::String(str_id) => {
                let dest = self.get_vreg();
                self.ops.push(Op::LoadStr { str_id, dest });
                dest
            }
        }
    }

    fn reserve_label(&mut self) -> Label {
        self.label_counter += 1;
        Label::N(self.label_counter - 1)
    }

    fn set_label_here(&mut self, label: Label) {
        self.labels.entry(self.ops.len()).or_default().push(label);
    }

    fn insert_label(&mut self) -> Label {
        let label = self.reserve_label();
        self.set_label_here(label);
        label
    }
}
