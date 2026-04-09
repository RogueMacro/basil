use std::collections::HashMap;

use crate::{
    analyze::{
        ast::{ArithmeticOp, Assignable, ExprInner, Expression, Item as AstItem, Statement},
        semantics::{Sign, ValidAST},
    },
    ir::{BasicBlock, Condition, IR, Item, Label, Op, OpIndex, SourceVal, Terminator, VirtualReg},
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
                let initial_args: Vec<_> = args
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, _, _))| (name, VirtualReg(i as u32)))
                    .collect();

                let vreg_args = initial_args.iter().map(|(_, vreg)| *vreg).collect();

                let body = BlockBuilder::new(&mut ir, initial_args).build(body);

                ir.items.push(Item::Function {
                    name,
                    args: vreg_args,
                    body,
                });
            };
        }

        ir
    }
}

struct BlockBuilder<'ir> {
    ir: &'ir mut IR,
    blocks: Vec<BasicBlock>,
    var_to_vreg: HashMap<String, VirtualReg>,
    vreg_counter: u32,
    label_counter: u32,

    block_label: Label,
    block_args: Vec<VirtualReg>,
    block_ops: Vec<Op>,
    block_decls: Vec<VirtualReg>,
}

impl<'ir> BlockBuilder<'ir> {
    pub fn new(ir: &'ir mut IR, initial_args: Vec<(String, VirtualReg)>) -> Self {
        let block_args = initial_args.iter().map(|(_, vreg)| *vreg).collect();

        Self {
            ir,
            blocks: Vec::new(),
            var_to_vreg: HashMap::from_iter(initial_args),
            vreg_counter: 0,
            label_counter: 0,

            block_label: Label::Entry,
            block_args,
            block_ops: Vec::new(),
            block_decls: Vec::new(),
        }
    }

    pub fn build(mut self, block: Vec<Statement>) -> Vec<BasicBlock> {
        self.consume(block);

        if !self.block_ops.is_empty() {
            self.commit_block(Terminator::Branch { label: Label::Ret }, Label::Ret);
        }

        self.blocks
    }

    fn consume(&mut self, block: Vec<Statement>) {
        for stmt in block {
            match stmt {
                Statement::Declare { var, expr, .. } => {
                    assert!(
                        !self.var_to_vreg.contains_key(&var),
                        "variable declared twice"
                    );

                    let dest = self.get_or_insert_vreg(var);
                    let src = self.unroll_expr(expr, Some(dest));

                    if src != SourceVal::VReg(dest) {
                        self.block_ops.push(Op::Assign { src, dest });
                        self.block_decls.push(dest);
                    }
                }

                Statement::Assign { var, expr, .. } => {
                    let dest = self.get_or_insert_vreg(var.symbol());
                    let src = self.unroll_expr(expr, Some(dest));

                    if !self.block_decls.contains(&dest) {
                        self.block_args.push(dest);
                    }

                    match var {
                        Assignable::Var(_) => {
                            if src.reg() != Some(dest) {
                                self.block_ops.push(Op::Assign { src, dest })
                            }
                        }
                        Assignable::Ptr(_) => {
                            todo!()
                            // let src = self.src_to_vreg(src);
                            // self.ops.push(Op::StorePointer { src, ptr: dest });
                        }
                    }
                }

                Statement::Expr(expr) => {
                    self.unroll_expr(expr, None);
                }

                Statement::If { guard, body } => {
                    let cond = self.unroll_expr(guard, None);
                    let cond = self.src_to_vreg(cond);

                    let if_true = self.next_label();
                    let if_false = self.next_label();

                    self.commit_block(
                        Terminator::BranchCond {
                            cond,
                            if_true,
                            if_false,
                        },
                        if_true,
                    );

                    self.consume(body);
                    self.commit_block(Terminator::Branch { label: if_false }, if_false);
                }
                Statement::Return(expr) => {
                    let value = self.unroll_expr(expr, None);
                    let new_label = self.next_label();
                    self.commit_block(Terminator::Return { value }, new_label);
                }
                Statement::WhileLoop { guard, body } => todo!(),
            }
        }
    }

    fn commit_block(&mut self, terminator: Terminator, new_label: Label) {
        let label = std::mem::replace(&mut self.block_label, new_label);
        let args = std::mem::take(&mut self.block_args);
        let ops = std::mem::take(&mut self.block_ops);

        self.block_decls.clear();

        self.blocks.push(BasicBlock {
            label,
            args,
            ops,
            terminator,
        });
    }

    fn next_label(&mut self) -> Label {
        let label = Label::Anon(self.label_counter);
        self.label_counter += 1;
        label
    }

    fn unroll_expr(&mut self, expr: Expression, dest: Option<VirtualReg>) -> SourceVal {
        match expr.inner {
            ExprInner::Const(num) => SourceVal::Immediate(num),
            ExprInner::Character(c) => SourceVal::Immediate(c as i64),
            ExprInner::String(string) => {
                let str_id = self.ir.alloc_str_literal(string);
                SourceVal::String(str_id)
            }
            ExprInner::Bool(b) => SourceVal::Immediate(b as i64),

            ExprInner::Variable(var) => SourceVal::VReg(self.expect_vreg(&var)),
            ExprInner::Pointer(var) => {
                let val = self.expect_vreg(&var);
                let dest = dest.unwrap_or_else(|| self.get_vreg());

                self.block_ops.push(Op::AddressOf { val, dest });
                SourceVal::VReg(dest)
            }
            ExprInner::Deref(var, typ) => {
                let ptr = self.expect_vreg(&var);
                let dest = dest.unwrap_or_else(|| self.get_vreg());

                self.block_ops.push(Op::LoadPointer {
                    ptr,
                    size: typ.unwrap().size(),
                    dest,
                });
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
                    ArithmeticOp::Add => self.block_ops.push(Op::Add { a, b, dest }),
                    ArithmeticOp::Sub => self.block_ops.push(Op::Subtract { a, b, dest }),
                    ArithmeticOp::Mult => self.block_ops.push(Op::Multiply { a, b, dest }),
                    ArithmeticOp::Div => self.block_ops.push(Op::Divide { a, b, dest }),
                }

                SourceVal::VReg(dest)
            }
            ExprInner::Comparison(expr1, expr2, op, sign) => {
                let expr1 = self.unroll_expr(*expr1, None);
                let expr2 = self.unroll_expr(*expr2, None);

                let expr1 = self.src_to_vreg(expr1);
                let expr2 = self.src_to_vreg(expr2);

                let dest = dest.unwrap_or_else(|| self.get_vreg());

                self.block_ops.push(Op::Compare {
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

                let dest = dest.unwrap_or_else(|| self.get_vreg());

                self.block_ops.push(Op::Call {
                    function: function.clone(),
                    args,
                    dest: Some(dest),
                });

                SourceVal::VReg(dest)
            }

            ExprInner::Cast(expr, _typ) => self.unroll_expr(*expr, dest),
        }
    }

    fn get_or_insert_vreg<S: Into<String> + AsRef<str>>(&mut self, var: S) -> VirtualReg {
        if let Some(&vreg) = self.var_to_vreg.get(var.as_ref()) {
            vreg
        } else {
            let vreg = self.get_vreg();
            self.var_to_vreg.insert(var.into(), vreg);
            vreg
        }
    }

    fn expect_vreg(&self, var: &str) -> VirtualReg {
        *self
            .var_to_vreg
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
            SourceVal::Immediate(_) | SourceVal::String(_) => {
                let dest = self.get_vreg();
                self.block_ops.push(Op::Assign { src, dest });
                dest
            }
            SourceVal::VReg(vreg) => vreg,
        }
    }
}

// struct _BlockBuilder<'ir> {
//     ir: &'ir mut IR,
//     vregs: HashMap<String, VirtualReg>,
//     vreg_counter: u32,
//     labels: HashMap<OpIndex, Vec<Label>>,
//     label_counter: u32,
//     ops: Vec<Op>,
// }
//
// impl<'ir> _BlockBuilder<'ir> {
//     pub fn new(ir: &'ir mut IR) -> Self {
//         Self {
//             vregs: HashMap::new(),
//             vreg_counter: 0,
//             labels: HashMap::new(),
//             label_counter: 0,
//             ops: Vec::new(),
//             ir,
//         }
//     }
//
//     pub fn build(mut self, block: Vec<Statement>) -> BasicBlock {
//         self.consume_block(block);
//         BasicBlock {
//             ops: self.ops,
//             labels: self.labels,
//         }
//     }
//
//     fn consume_block(&mut self, block: Vec<Statement>) {
//         self.ops.reserve(block.len());
//
//         for stmt in block {
//             match stmt {
//                 Statement::Declare { var, expr, .. } => {
//                     assert!(!self.vregs.contains_key(&var), "variable declared twice");
//
//                     let dest = self.get_or_insert_vreg(var);
//                     let src = self.unroll_expr(expr, Some(dest));
//
//                     if src != SourceVal::VReg(dest) {
//                         self.ops.push(Op::Assign { src, dest });
//                     }
//                 }
//                 Statement::Assign { var, expr, .. } => {
//                     let dest = self.get_or_insert_vreg(var.symbol());
//                     let src = self.unroll_expr(expr, Some(dest));
//
//                     match var {
//                         Assignable::Var(_) => {
//                             if src.reg() != Some(dest) {
//                                 self.ops.push(Op::Assign { src, dest })
//                             }
//                         }
//                         Assignable::Ptr(_) => {
//                             let src = self.src_to_vreg(src);
//                             self.ops.push(Op::StorePointer { src, ptr: dest });
//                         }
//                     }
//                 }
//                 Statement::Return(expr) => {
//                     let value = self.unroll_expr(expr, None);
//                     self.ops.push(Op::Return { value });
//                 }
//
//                 Statement::If { guard, body } => {
//                     let cond = self.unroll_expr(guard, None);
//                     let cond = self.src_to_vreg(cond);
//                     let label = self.reserve_label();
//                     self.ops.push(Op::BranchIfNot { cond, label });
//
//                     let outer_vregs = self.vregs.clone();
//                     let outer_vreg_counter = self.vreg_counter;
//
//                     self.consume_block(body);
//
//                     for (name, inner) in self.vregs.iter() {
//                         if let Some(outer) = outer_vregs.get(name)
//                             && inner != outer
//                         {
//                             self.ops.push(Op::Assign {
//                                 src: SourceVal::VReg(*inner),
//                                 dest: *outer,
//                             });
//                         }
//                     }
//
//                     self.set_label_here(label);
//
//                     self.vregs = outer_vregs;
//                     self.vreg_counter = outer_vreg_counter;
//                 }
//
//                 Statement::WhileLoop { guard, body } => {
//                     let cond_label = self.reserve_label();
//
//                     let outer_vregs = self.vregs.clone();
//                     let outer_vreg_counter = self.vreg_counter;
//
//                     self.ops.push(Op::Branch { label: cond_label });
//
//                     let body_label = self.insert_label();
//                     self.consume_block(body);
//
//                     self.set_label_here(cond_label);
//                     let cond = self.unroll_expr(guard, None);
//                     let cond = self.src_to_vreg(cond);
//                     self.ops.push(Op::BranchIf {
//                         cond,
//                         label: body_label,
//                     });
//
//                     for (name, inner) in self.vregs.iter() {
//                         if let Some(outer) = outer_vregs.get(name)
//                             && inner != outer
//                         {
//                             self.ops.push(Op::Assign {
//                                 src: SourceVal::VReg(*inner),
//                                 dest: *outer,
//                             });
//                         }
//                     }
//
//                     self.vregs = outer_vregs;
//                     self.vreg_counter = outer_vreg_counter;
//                 }
//
//                 Statement::Expr(expr) => {
//                     self.unroll_expr(expr, None);
//                 }
//             }
//         }
//     }
//
//     fn unroll_expr(&mut self, expr: Expression, dest: Option<VirtualReg>) -> SourceVal {
//         match expr.inner {
//             ExprInner::Const(num) => SourceVal::Immediate(num),
//             ExprInner::Character(c) => SourceVal::Immediate(c as i64),
//             ExprInner::String(string) => {
//                 let str_id = self.ir.alloc_str_literal(string);
//                 SourceVal::String(str_id)
//             }
//             ExprInner::Bool(b) => SourceVal::Immediate(b as i64),
//
//             ExprInner::Variable(var) => SourceVal::VReg(self.expect_vreg(&var)),
//             ExprInner::Pointer(var) => {
//                 let val = self.expect_vreg(&var);
//                 let dest = dest.unwrap_or_else(|| self.get_vreg());
//
//                 self.ops.push(Op::AddressOf { val, dest });
//                 SourceVal::VReg(dest)
//             }
//             ExprInner::Deref(var, typ) => {
//                 let ptr = self.expect_vreg(&var);
//                 let dest = dest.unwrap_or_else(|| self.get_vreg());
//
//                 self.ops.push(Op::LoadPointer {
//                     ptr,
//                     size: typ.unwrap().size(),
//                     dest,
//                 });
//                 SourceVal::VReg(dest)
//             }
//
//             ExprInner::Arithmetic(expr1, expr2, op, _sign) => {
//                 // TODO: sign
//                 let a = self.unroll_expr(*expr1, None);
//                 let b = self.unroll_expr(*expr2, None);
//
//                 let a = self.src_to_vreg(a);
//                 let b = self.src_to_vreg(b);
//
//                 let dest = dest.unwrap_or_else(|| self.get_vreg());
//
//                 match op {
//                     ArithmeticOp::Add => self.ops.push(Op::Add { a, b, dest }),
//                     ArithmeticOp::Sub => self.ops.push(Op::Subtract { a, b, dest }),
//                     ArithmeticOp::Mult => self.ops.push(Op::Multiply { a, b, dest }),
//                     ArithmeticOp::Div => self.ops.push(Op::Divide { a, b, dest }),
//                 }
//
//                 SourceVal::VReg(dest)
//             }
//             ExprInner::Comparison(expr1, expr2, op, sign) => {
//                 let expr1 = self.unroll_expr(*expr1, None);
//                 let expr2 = self.unroll_expr(*expr2, None);
//
//                 let expr1 = self.src_to_vreg(expr1);
//                 let expr2 = self.src_to_vreg(expr2);
//
//                 let dest = dest.unwrap_or_else(|| self.get_vreg());
//
//                 self.ops.push(Op::Compare {
//                     a: expr1,
//                     b: expr2,
//                     cond: Condition::from_ast_op(op, matches!(sign, Some(Sign::Signed))),
//                     dest,
//                 });
//
//                 SourceVal::VReg(dest)
//             }
//
//             ExprInner::FnCall(function, args) => {
//                 let args = args
//                     .into_iter()
//                     .map(|e| {
//                         let src = self.unroll_expr(e, None);
//                         self.src_to_vreg(src)
//                     })
//                     .collect();
//
//                 let dest = dest.unwrap_or_else(|| self.get_vreg());
//
//                 println!("call to {} ret {:?}", function, dest);
//                 self.ops.push(Op::Call {
//                     function: function.clone(),
//                     args,
//                     dest: Some(dest),
//                 });
//
//                 SourceVal::VReg(dest)
//             }
//
//             ExprInner::Cast(expr, _typ) => self.unroll_expr(*expr, dest),
//         }
//     }
//
//     fn get_or_insert_vreg<S: Into<String> + AsRef<str>>(&mut self, var: S) -> VirtualReg {
//         if let Some(&vreg) = self.vregs.get(var.as_ref()) {
//             vreg
//         } else {
//             let vreg = self.get_vreg();
//             self.vregs.insert(var.into(), vreg);
//             vreg
//         }
//     }
//
//     fn expect_vreg(&self, var: &str) -> VirtualReg {
//         *self
//             .vregs
//             .get(var)
//             .unwrap_or_else(|| panic!("undefined variable '{}'", var))
//     }
//
//     fn get_vreg(&mut self) -> VirtualReg {
//         let vreg = VirtualReg(self.vreg_counter);
//         self.vreg_counter += 1;
//         vreg
//     }
//
//     fn src_to_vreg(&mut self, src: SourceVal) -> VirtualReg {
//         match src {
//             SourceVal::Immediate(_) | SourceVal::String(_) => {
//                 let dest = self.get_vreg();
//                 self.ops.push(Op::Assign { src, dest });
//                 dest
//             }
//             SourceVal::VReg(vreg) => vreg,
//         }
//     }
//
//     fn reserve_label(&mut self) -> Label {
//         self.label_counter += 1;
//         Label::Anon(self.label_counter - 1)
//     }
//
//     fn set_label_here(&mut self, label: Label) {
//         self.labels.entry(self.ops.len()).or_default().push(label);
//     }
//
//     fn insert_label(&mut self) -> Label {
//         let label = self.reserve_label();
//         self.set_label_here(label);
//         label
//     }
// }
