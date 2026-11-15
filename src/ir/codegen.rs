use std::{
    assert_matches,
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use crate::{
    analyze::{
        ast::{
            ArithmeticOp, Assignable, ExprInner, Expression, FnDef, FnPtr, IfElse, Item as AstItem,
            LogicalOp, Statement,
        },
        semantics::{
            Analyzer, ValidAST,
            types::{ParsedType, Sign, TypeId, TypeKind, TypeMap},
        },
    },
    ir::{BasicBlock, Condition, IR, Item, Label, Op, SourceVal, Terminator, VirtualReg, VregSize},
};

pub const ALLOC_FN: &str = "std::alloc::alloc";

impl<'s> IR<'s> {
    pub fn generate(ast: ValidAST<'s>, analyzer: &Analyzer<'s>) -> IR<'s> {
        let ast = ast.0;

        let mut ir = IR::default();

        for item in ast.items {
            match item {
                AstItem::Function(FnDef {
                    name, body, args, ..
                }) => {
                    let initial_args: Vec<_> = args
                        .into_iter()
                        .enumerate()
                        .map(|(i, (name, _, _))| (name, VirtualReg(i as u32)))
                        .collect();

                    let vreg_args = initial_args.iter().map(|(_, vreg)| *vreg).collect();

                    let (body, stack, stack_size, size_map) =
                        BlockBuilder::new(analyzer, &mut ir, initial_args).build(body);

                    ir.items.push(Item::Function {
                        name,
                        args: vreg_args,
                        stack,
                        stack_size,
                        size_map,
                        body,
                    });
                }

                AstItem::Impl {
                    struct_name,
                    functions,
                } => {
                    for fndef in functions {
                        let FnDef {
                            name, body, args, ..
                        } = fndef;

                        let name = format!("{}::{}", struct_name, name);

                        let initial_args: Vec<_> = args
                            .into_iter()
                            .enumerate()
                            .map(|(i, (name, _, _))| (name, VirtualReg(i as u32)))
                            .collect();

                        let vreg_args = initial_args.iter().map(|(_, vreg)| *vreg).collect();

                        let (body, stack, stack_size, size_map) =
                            BlockBuilder::new(analyzer, &mut ir, initial_args).build(body);

                        ir.items.push(Item::Function {
                            name: Cow::Owned(name),
                            args: vreg_args,
                            stack,
                            stack_size,
                            size_map,
                            body,
                        });
                    }
                }

                AstItem::MemorySegment { name, typ: typeid } => {
                    let type_size = analyzer.types.size_of(typeid);
                    ir.static_mem.alloc(name, typeid, type_size);
                }

                AstItem::Struct { .. } => {}

                AstItem::ForwardDecl { .. } | AstItem::ExternLib(_) => {}
            }
        }

        // if let Err(dupes) = crate::ir::ssa::verify_ssa(&ir) {
        //     println!("{}", ir);
        //     panic!("ir is not ssa, duplicates: {:?}", dupes);
        // }

        println!("{}", ir);

        ir
    }
}

struct BlockBuilder<'ir, 'a, 's> {
    analyzer: &'a Analyzer<'s>,
    ir: &'ir mut IR<'s>,

    blocks: Vec<BasicBlock>,
    var_to_vreg: HashMap<&'s str, VirtualReg>,
    proc_args: HashMap<VirtualReg, u32>,
    size_map: HashMap<VirtualReg, VregSize>,
    stack: HashMap<VirtualReg, u32>,
    stack_size: u32,
    vreg_counter: u32,
    label_counter: u32,

    block_label: Label,
    // block_args: Vec<VirtualReg>,
    block_ops: Vec<Op>,
    block_decls: Vec<VirtualReg>,
}

impl<'ir, 'a, 's> BlockBuilder<'ir, 'a, 's> {
    pub fn new(
        analyzer: &'a Analyzer<'s>,
        ir: &'ir mut IR<'s>,
        initial_args: Vec<(&'s str, VirtualReg)>,
    ) -> Self {
        // let block_args = initial_args.iter().map(|(_, vreg)| *vreg).collect();
        let n_args = initial_args.len();
        // let stack = HashMap::from_iter(
        //     initial_args
        //         .iter()
        //         .enumerate()
        //         .map(|(i, (var, _))| (var.clone(), (i as u32) * 8)),
        // );
        let proc_args = HashMap::from_iter(
            initial_args
                .iter()
                .enumerate()
                .map(|(i, (_, vreg))| (*vreg, (i as u32))),
        );
        // let stack_size = (stack.len() as u32) * 8;

        Self {
            analyzer,
            ir,

            blocks: Vec::new(),
            var_to_vreg: initial_args.into_iter().collect(),
            proc_args,
            size_map: HashMap::new(),
            stack: HashMap::new(),
            stack_size: 0,
            vreg_counter: n_args as u32,
            label_counter: 0,

            block_label: Label::Entry,
            // block_args,
            block_ops: Vec::new(),
            block_decls: Vec::new(),
        }
    }

    pub fn build(
        mut self,
        block: Vec<Statement<'s, TypeId>>,
    ) -> (
        Vec<BasicBlock>,
        HashMap<VirtualReg, u32>,
        u32,
        HashMap<VirtualReg, VregSize>,
    ) {
        self.consume(block);
        self.commit_block(Terminator::Branch { label: Label::End }, Label::End);

        let mut run_again = true;
        while run_again {
            run_again = false;

            let mut succ_args = Vec::new();
            for bb in self.blocks.iter() {
                let mut args: HashSet<VirtualReg> = HashSet::new();
                for succ_label in bb.successors() {
                    if let Some(_args) = self
                        .blocks
                        .iter()
                        .find(|bb2| bb2.label == succ_label)
                        .map(|bb2| &bb2.args)
                    {
                        args.extend(_args.iter().filter(|vreg| !bb.decls.contains(vreg)));
                    }
                }

                succ_args.push(args);
            }

            for (bb, append_args) in self.blocks.iter_mut().zip(succ_args) {
                let len = bb.args.len();
                bb.args.extend(append_args);
                if bb.args.len() > len {
                    run_again = true;
                }
            }
        }

        // self.stack.extend(
        //     self.proc_args
        //         .into_iter()
        //         .enumerate()
        //         .map(|(i, vreg)| (vreg, (i as u32) + self.stack_size + 16)),
        // );

        (self.blocks, self.stack, self.stack_size, self.size_map)
    }

    fn consume(&mut self, block: Vec<Statement<'s, TypeId>>) {
        for stmt in block {
            match stmt {
                Statement::Declare { var, expr, .. } => {
                    assert!(
                        !self.var_to_vreg.contains_key(&var),
                        "variable declared twice"
                    );

                    if expr.typeid == Some(TypeId::unit()) {
                        continue;
                    }

                    let size =
                        VregSize::from_bytes(self.analyzer.types.size_of(expr.typeid.unwrap()));

                    let is_construct = matches!(expr.inner, ExprInner::Construct { .. });

                    let (dest, stack_offset) = self.get_or_insert_stack_var(var, size);
                    let src = self.flatten_expr(expr, Some(dest));

                    if src != SourceVal::VReg(dest) {
                        self.block_ops.push(Op::Assign { src, dest });
                        self.block_decls.push(dest);
                    }

                    if !is_construct && size.to_bytes() > 8 {
                        todo!()
                    } else if !is_construct {
                        self.block_ops.push(Op::Store {
                            src: dest,
                            stack_offset,
                        });
                    }
                }

                Statement::Assign { var, expr, .. } => {
                    let size =
                        VregSize::from_bytes(self.analyzer.types.size_of(expr.typeid.unwrap()));
                    let src = self.flatten_expr(expr, None);

                    match var {
                        Assignable::Var(var) => {
                            if let Some((offset, typ)) = self.ir.static_mem.get(var) {
                                let offset = *offset;
                                let typeid = *typ;

                                let ptr = self.get_vreg(VregSize::Doubleword);
                                let src = self.src_to_vreg(src);

                                self.block_ops.push(Op::Assign {
                                    src: SourceVal::StaticMem(offset),
                                    dest: ptr,
                                });

                                let size = self.analyzer.types.size_of(typeid);
                                if size <= 8 {
                                    self.block_ops.push(Op::StorePointer {
                                        src,
                                        ptr,
                                        size: VregSize::from_bytes(size),
                                        offset: 0,
                                    });
                                } else {
                                    todo!();
                                }
                            } else {
                                let (dest, stack_offset) = self.get_or_insert_stack_var(var, size);

                                self.block_ops.push(Op::Assign { src, dest });
                                self.block_ops.push(Op::Store {
                                    src: dest,
                                    stack_offset,
                                });
                            }
                        }
                        Assignable::Ptr(var, size) => {
                            let var_vreg = self.var_to_vreg.get(&var).copied().unwrap();
                            let src = self.src_to_vreg(src);
                            self.block_ops.push(Op::StorePointer {
                                src,
                                ptr: var_vreg,
                                size: size.unwrap(),
                                offset: 0,
                            });
                        }
                        Assignable::Index {
                            data,
                            index,
                            val_size,
                        } => {
                            let index = self.flatten_expr(*index, None);
                            let index_vreg = self.src_to_vreg(index);

                            // let array_vreg = self.get_vreg(ValSize::Doubleword);
                            // self.load_var(&data, array_vreg);
                            let array = self.flatten_expr(*data, None);
                            let array_vreg = self.src_to_vreg(array);

                            let ptr = self.get_vreg(VregSize::Doubleword);

                            let src = self.src_to_vreg(src);

                            let val_size = val_size.unwrap();

                            let size_vreg =
                                self.src_to_vreg(SourceVal::Immediate(val_size.to_bytes()));

                            let final_index_vreg = self.get_vreg(val_size);

                            self.block_ops.push(Op::Multiply {
                                a: index_vreg,
                                b: size_vreg,
                                dest: final_index_vreg,
                            });

                            self.block_ops.push(Op::Add {
                                a: array_vreg,
                                b: final_index_vreg,
                                dest: ptr,
                            });

                            self.block_ops.push(Op::StorePointer {
                                src,
                                ptr,
                                size: val_size,
                                offset: 0,
                            });
                        }
                        Assignable::MemberAccess(parent, member) => {
                            let TypeKind::Pointer(value_typeid) =
                                self.analyzer.types.get(parent.typeid.unwrap()).kind
                            else {
                                panic!()
                            };

                            let offset = self
                                .analyzer
                                .types
                                .offset_of_member(value_typeid, member)
                                .unwrap();

                            let parent = self.flatten_expr(*parent, None);
                            let parent = self.src_to_vreg(parent);

                            let src = self.src_to_vreg(src);

                            self.block_ops.push(Op::StorePointer {
                                src,
                                ptr: parent,
                                size,
                                offset: offset as u32,
                            })
                        }
                    }
                }

                Statement::Expr(expr) => {
                    self.flatten_expr(expr, None);
                }

                Statement::If(ifelse) => {
                    assert_matches!(ifelse, IfElse::If { .. });

                    let end_if = self.next_label();
                    self.if_else(ifelse, end_if);

                    // self.commit_block(Terminator::Branch { label: end_if }, end_if);
                }
                Statement::Return(expr) => {
                    let value = self.flatten_expr(expr, None);
                    let value = self.src_to_vreg(value);
                    let new_label = self.next_label();
                    self.commit_block(Terminator::Return { value }, new_label);
                }
                Statement::WhileLoop { guard, body } => {
                    let guard_label = self.next_label();
                    let body_label = self.next_label();
                    let end_label = self.next_label();

                    self.commit_block(Terminator::Branch { label: guard_label }, guard_label);

                    let cond = self.flatten_expr(guard, None);
                    let cond = self.src_to_vreg(cond);
                    self.commit_block(
                        Terminator::BranchCond {
                            cond,
                            if_true: body_label,
                            if_false: end_label,
                        },
                        body_label,
                    );

                    self.consume(body);
                    self.commit_block(Terminator::Branch { label: guard_label }, end_label);
                }
            }
        }
    }

    fn if_else(&mut self, ifelse: IfElse<'s, TypeId>, end_if: Label) {
        match ifelse {
            IfElse::If { guard, body, next } => {
                let cond = self.flatten_expr(guard, None);
                let cond = self.src_to_vreg(cond);

                let if_true = self.next_label();
                let if_false = if next.is_some() {
                    self.next_label()
                } else {
                    end_if
                };

                self.commit_block(
                    Terminator::BranchCond {
                        cond,
                        if_true,
                        if_false,
                    },
                    if_true,
                );

                self.consume(body);

                self.commit_block(Terminator::Branch { label: end_if }, if_false);

                if let Some(next) = next {
                    self.if_else(*next, end_if);
                }
            }
            IfElse::Else { body } => {
                self.consume(body);
                self.commit_block(Terminator::Branch { label: end_if }, end_if);
            }
        }
    }

    fn commit_block(&mut self, terminator: Terminator, new_label: Label) {
        let label = std::mem::replace(&mut self.block_label, new_label);
        // let args = std::mem::take(&mut self.block_args);
        let ops = std::mem::take(&mut self.block_ops);

        let mut args = HashSet::new();
        if let Some(vreg) = terminator.vreg_used() {
            args.insert(vreg);
        }

        for op in ops.iter() {
            let (uses, assigned) = op.vregs_used();
            args.extend(uses);
            if let Some(assigned) = assigned {
                args.insert(assigned);
            }
        }

        for vreg in self.block_decls.iter() {
            args.remove(vreg);
        }

        let decls = std::mem::take(&mut self.block_decls);

        self.blocks.push(BasicBlock {
            label,
            args,
            ops,
            decls,
            terminator,
        });
    }

    fn next_label(&mut self) -> Label {
        let label = Label::Anon(self.label_counter);
        self.label_counter += 1;
        label
    }

    fn flatten_expr(
        &mut self,
        expr: Expression<'s, TypeId>,
        dest: Option<VirtualReg>,
    ) -> SourceVal {
        let type_size = self.analyzer.types.size_of(expr.typeid.unwrap());
        let type_size = VregSize::from_bytes(type_size);

        match expr.inner {
            ExprInner::Const(num, explicit_type) => SourceVal::Immediate(num),
            ExprInner::Character(c) => SourceVal::Immediate(c as u64),
            ExprInner::String(string) => {
                let str_id = self.ir.insert_str_literal(string);
                SourceVal::String(str_id)
            }
            ExprInner::Bool(b) => SourceVal::Immediate(b as u64),

            ExprInner::Ident(var) => {
                let dest = self.get_vreg(type_size);

                self.load_var(var, dest);

                SourceVal::VReg(dest)
            }
            ExprInner::Pointer(var) => {
                // let stack_offset = self.get_stack_offset(&var);
                let val = *self.var_to_vreg.get(&var).unwrap();
                let dest = dest.unwrap_or_else(|| self.get_vreg(type_size));

                if let Some(&offset) = self.proc_args.get(&val) {
                    self.block_ops.push(Op::AddressOfArg { offset, dest });
                } else {
                    self.block_ops.push(Op::AddressOf { val, dest });
                }

                SourceVal::VReg(dest)
            }
            ExprInner::Deref(var, typ) => {
                let ptr = self.flatten_expr(*var, None);
                let ptr = self.src_to_vreg(ptr);

                // let ptr = self.get_vreg(ValSize::Doubleword);
                let dest = dest.unwrap_or_else(|| self.get_vreg(type_size));
                // let dest = dest.unwrap_or_else(|| {
                //     self.get_vreg(
                //         type_size.unwrap_or_else(|| panic!("expected deref type for {}", var)),
                //     )
                // });

                // self.load_var(var, ptr);

                let size = self.analyzer.types.size_of(typ.unwrap());
                if size > 8 {
                    todo!()
                }
                self.block_ops.push(Op::LoadPointer {
                    ptr,
                    size: VregSize::from_bytes(size),
                    dest,
                });
                SourceVal::VReg(dest)
            }

            ExprInner::Arithmetic(expr1, expr2, op, _sign) => {
                // TODO: sign
                let a = self.flatten_expr(*expr1, None);
                let b = self.flatten_expr(*expr2, None);

                let a = self.src_to_vreg(a);
                let b = self.src_to_vreg(b);

                let dest = dest.unwrap_or_else(|| self.get_vreg(type_size));

                match op {
                    ArithmeticOp::Add => self.block_ops.push(Op::Add { a, b, dest }),
                    ArithmeticOp::Sub => self.block_ops.push(Op::Subtract { a, b, dest }),
                    ArithmeticOp::Mul => self.block_ops.push(Op::Multiply { a, b, dest }),
                    ArithmeticOp::Div => self.block_ops.push(Op::Divide { a, b, dest }),
                    ArithmeticOp::Mod => self.block_ops.push(Op::Modulo { a, b, dest }),
                }

                SourceVal::VReg(dest)
            }
            ExprInner::Comparison(expr1, expr2, op, sign) => {
                let expr1 = self.flatten_expr(*expr1, None);
                let expr2 = self.flatten_expr(*expr2, None);

                let expr1 = self.src_to_vreg(expr1);
                let expr2 = self.src_to_vreg(expr2);

                let dest = dest.unwrap_or_else(|| self.get_vreg(type_size));

                self.block_ops.push(Op::Compare {
                    a: expr1,
                    b: expr2,
                    cond: Condition::from_ast_op(op, matches!(sign, Some(Sign::Signed))),
                    dest,
                });

                SourceVal::VReg(dest)
            }
            ExprInner::Logical(lhs, rhs, op) => {
                let dest = dest.unwrap_or_else(|| self.get_vreg(type_size));

                let lhs = self.flatten_expr(*lhs, None);
                let lhs = self.src_to_vreg(lhs);

                let second_test = self.next_label();
                let test_true = self.next_label();
                let test_false = self.next_label();
                let end = self.next_label();

                let term = match op {
                    LogicalOp::And => Terminator::BranchCond {
                        cond: lhs,
                        if_true: second_test,
                        if_false: test_false,
                    },
                    LogicalOp::Or => Terminator::BranchCond {
                        cond: lhs,
                        if_true: test_true,
                        if_false: second_test,
                    },
                };

                self.commit_block(term, second_test);

                let rhs = self.flatten_expr(*rhs, None);
                let rhs = self.src_to_vreg(rhs);

                self.commit_block(
                    Terminator::BranchCond {
                        cond: rhs,
                        if_true: test_true,
                        if_false: test_false,
                    },
                    test_true,
                );

                self.block_decls.push(dest);
                self.block_ops.push(Op::Assign {
                    src: SourceVal::Immediate(1),
                    dest,
                });

                self.commit_block(Terminator::Branch { label: end }, test_false);

                self.block_decls.push(dest);
                self.block_ops.push(Op::Assign {
                    src: SourceVal::Immediate(0),
                    dest,
                });

                self.commit_block(Terminator::Branch { label: end }, end);

                SourceVal::VReg(dest)
            }

            ExprInner::Not(expr) => {
                let val = self.flatten_expr(*expr, dest);
                let val = self.src_to_vreg(val);
                let dest = dest.unwrap_or_else(|| self.get_vreg(type_size));

                self.block_ops.push(Op::Not { val, dest });

                SourceVal::VReg(dest)
            }

            ExprInner::Negate(expr) => {
                let val = self.flatten_expr(*expr, dest);
                let val = self.src_to_vreg(val);
                let dest = dest.unwrap_or_else(|| self.get_vreg(type_size));

                self.block_ops.push(Op::Negate { val, dest });

                SourceVal::VReg(dest)
            }

            ExprInner::Cast(expr, _typ) => self.flatten_expr(*expr, dest),

            ExprInner::Index {
                data,
                index,
                val_size,
            } => {
                let index = self.flatten_expr(*index, None);
                let index_vreg = self.src_to_vreg(index);

                let array = self.flatten_expr(*data, None);
                let array_vreg = self.src_to_vreg(array);

                let ptr = self.get_vreg(VregSize::Doubleword);

                let dest = dest.unwrap_or_else(|| self.get_vreg(type_size));

                let val_size = val_size.unwrap();

                let size_vreg = self.src_to_vreg(SourceVal::Immediate(val_size.to_bytes()));

                let final_index_vreg = self.get_vreg(val_size);

                self.block_ops.push(Op::Multiply {
                    a: index_vreg,
                    b: size_vreg,
                    dest: final_index_vreg,
                });

                self.block_ops.push(Op::Add {
                    a: array_vreg,
                    b: final_index_vreg,
                    dest: ptr,
                });

                self.block_ops.push(Op::LoadPointer {
                    ptr,
                    size: val_size,
                    dest,
                });

                SourceVal::VReg(dest)
            }

            ExprInner::MemberAccess(parent, member, typeid) => {
                let typeid = typeid.unwrap();

                let offset = self
                    .analyzer
                    .types
                    .offset_of_member(typeid, member)
                    .unwrap_or_else(|| {
                        panic!(
                            "type {} does not have member {}",
                            self.analyzer.types.display(typeid),
                            member
                        )
                    });

                let dest = dest.unwrap_or_else(|| self.get_vreg(type_size));

                if self.analyzer.types.get(parent.typeid.unwrap()).is_ptr() {
                    let parent = self.flatten_expr(*parent, None);
                    let parent_vreg = self.src_to_vreg(parent);

                    self.block_ops.push(Op::LoadOffset {
                        base: parent_vreg,
                        offset: offset as u32,
                        dest,
                    });
                } else {
                    let stack_offset = self.stack_offset_of_expr(*parent);

                    self.block_ops.push(Op::Load {
                        stack_offset: stack_offset + offset as u32,
                        dest,
                    })
                }

                SourceVal::VReg(dest)
            }

            ExprInner::FnCall(function, args) => {
                let args = args
                    .into_iter()
                    .map(|e| {
                        let src = self.flatten_expr(e, None);
                        self.src_to_vreg(src)
                    })
                    .collect();

                let dest = dest.unwrap_or_else(|| self.get_vreg(type_size));

                let FnPtr::Named(function) = function else {
                    panic!()
                };

                self.block_ops.push(Op::Call {
                    function: function.into_owned(),
                    args,
                    dest: Some(dest),
                });

                SourceVal::VReg(dest)
            }

            ExprInner::Construct {
                typeid,
                fields,
                heap,
            } => {
                // TODO: get rid of VregSize, just use u32 instead

                println!("- CONSTRUCT TYPE {} -", self.analyzer.types.display(typeid));

                let type_size = VregSize::Bytes(self.analyzer.types.size_of(typeid));

                if !heap {
                    println!("=> heap: false");
                    let dest = dest.unwrap_or_else(|| {
                        let vreg = self.get_vreg(type_size);
                        self.stack.insert(vreg, self.stack_size);
                        self.stack_size += type_size.to_bytes() as u32;
                        vreg
                    });
                    assert!(self.size_map.get(&dest).unwrap().to_bytes() == type_size.to_bytes());
                    println!("=> vreg: {}", dest);
                    println!("=> size: {}", type_size.to_bytes());

                    let offset = *self.stack.get(&dest).unwrap();
                    println!("=> stack offset: {}", offset);

                    self.flatten_construct(typeid, fields, ConstructPlacement::Stack { offset });

                    SourceVal::VReg(dest)
                } else {
                    println!("=> heap: true");
                    let size_vreg = self.get_vreg(VregSize::Doubleword);
                    let ptr_vreg = dest.unwrap_or_else(|| self.get_vreg(VregSize::Doubleword));

                    self.block_ops.push(Op::Assign {
                        src: SourceVal::Immediate(type_size.to_bytes()),
                        dest: size_vreg,
                    });
                    self.block_ops.push(Op::Call {
                        function: ALLOC_FN.to_owned(),
                        args: vec![size_vreg],
                        dest: Some(ptr_vreg),
                    });

                    self.flatten_construct(
                        typeid,
                        fields,
                        ConstructPlacement::Heap {
                            base: ptr_vreg,
                            offset: 0,
                        },
                    );

                    SourceVal::VReg(ptr_vreg)
                }
            }

            ExprInner::SizeOf(typ) => {
                let size = self.analyzer.types.size_of(typ);
                SourceVal::Immediate(size)
            }
        }
    }

    fn flatten_construct(
        &mut self,
        typeid: TypeId,
        fields: Vec<(&'s str, Expression<'s, TypeId>)>,
        placement: ConstructPlacement,
    ) {
        for (name, expr) in fields {
            let field_offset = self.analyzer.types.offset_of_member(typeid, name).unwrap() as u32;
            println!(
                "=> member (+{}): {}  {}",
                field_offset,
                name,
                self.analyzer.types.display(expr.typeid.unwrap())
            );

            match expr.inner {
                ExprInner::Construct {
                    typeid,
                    fields,
                    heap,
                } => {
                    match (heap, placement) {
                        (false, ConstructPlacement::Stack { offset }) => self.flatten_construct(
                            typeid,
                            fields,
                            ConstructPlacement::Stack {
                                offset: offset + field_offset,
                            },
                        ),
                        (true, ConstructPlacement::Stack { offset }) => {
                            let size_vreg = self.get_vreg(VregSize::Doubleword);
                            let ptr_vreg = self.get_vreg(VregSize::Doubleword);

                            self.block_ops.push(Op::Assign {
                                src: SourceVal::Immediate(self.analyzer.types.size_of(typeid)),
                                dest: size_vreg,
                            });
                            self.block_ops.push(Op::Call {
                                function: ALLOC_FN.to_owned(),
                                args: vec![size_vreg],
                                dest: Some(ptr_vreg),
                            });
                            self.block_ops.push(Op::Store {
                                src: ptr_vreg,
                                stack_offset: field_offset + offset,
                            });

                            self.flatten_construct(
                                typeid,
                                fields,
                                ConstructPlacement::Heap {
                                    base: ptr_vreg,
                                    offset: 0,
                                },
                            );
                        }
                        _ => todo!(),
                    }

                    // if heap {
                    //     let size = self.analyzer.types.size_of(typeid);
                    //
                    //     let size_vreg = self.get_vreg(VregSize::Doubleword);
                    //     let ptr_vreg = self.get_vreg(VregSize::Doubleword);
                    //
                    //     self.block_ops.push(Op::Assign {
                    //         src: SourceVal::Immediate(size),
                    //         dest: size_vreg,
                    //     });
                    //     self.block_ops.push(Op::Call {
                    //         function: ALLOC_FN.to_owned(),
                    //         args: vec![size_vreg],
                    //         dest: Some(ptr_vreg),
                    //     });
                    //
                    //     self.flatten_construct(
                    //         typeid,
                    //         fields,
                    //         ConstructPlacement::Heap {
                    //             base: ptr_vreg,
                    //             offset: 0,
                    //         },
                    //     );
                    // } else {
                    //     todo!()
                    // }

                    // match placement {
                    //     ConstructPlacement::Heap { base, offset } => {
                    //         self.block_ops.push(Op::StorePointer {
                    //             src: todo!(),
                    //             ptr: todo!(),
                    //             size: todo!(),
                    //             offset: todo!(),
                    //         });
                    //     }
                    //     ConstructPlacement::Stack { offset } => {
                    //         todo!()
                    //         // self.block_ops.push(Op::Store {
                    //         //     src: SourceVal::VReg(ptr_vreg),
                    //         //     stack_offset: offset + field_offset,
                    //         // });
                    //     }
                    // }
                }
                _ => {
                    let field_size = self.analyzer.types.size_of(expr.typeid.unwrap());
                    assert!(field_size <= 8); // if bigger, copy all bytes

                    let field_size = VregSize::from_bytes(field_size);

                    let src = self.flatten_expr(expr, None);
                    let src = self.src_to_vreg(src);

                    match placement {
                        ConstructPlacement::Heap { base, offset } => {
                            self.block_ops.push(Op::StorePointer {
                                src,
                                ptr: base,
                                size: field_size,
                                offset: offset + field_offset,
                            })
                        }
                        ConstructPlacement::Stack { offset } => self.block_ops.push(Op::Store {
                            src,
                            stack_offset: offset + field_offset,
                        }),
                    }
                }
            }
        }
    }

    fn load_var(&mut self, var: &str, dest: VirtualReg) {
        if let Some((offset, typeid)) = self.ir.static_mem.get(var).copied() {
            let ptr = self.get_vreg(VregSize::Doubleword);
            self.block_ops.push(Op::Assign {
                src: SourceVal::StaticMem(offset),
                dest: ptr,
            });

            let size = self.analyzer.types.size_of(typeid);
            if size > 8 {
                todo!()
            }
            self.block_ops.push(Op::LoadPointer {
                ptr,
                size: VregSize::from_bytes(size),
                dest,
            });

            return;
        }

        let vreg = *self.var_to_vreg.get(var).unwrap();

        if let Some(&offset) = self.proc_args.get(&vreg) {
            self.block_ops.push(Op::LoadArg { offset, dest });
        } else if let Some(offset) = self.ir.static_mem.get(var) {
            todo!()
        } else {
            let offset = self.get_stack_offset(var);
            self.block_ops.push(Op::Load {
                stack_offset: offset,
                dest,
            });
        }
    }

    fn get_or_insert_stack_var(&mut self, var: &'s str, size: VregSize) -> (VirtualReg, u32) {
        if self.ir.static_mem.get(var).is_some() {
            panic!()
        }

        let vreg = self.var_to_vreg.get(var).copied().unwrap_or_else(|| {
            let vreg = self.get_vreg(size);
            self.var_to_vreg.insert(var, vreg);
            vreg
        });

        let offset = self.stack.get(&vreg).copied().unwrap_or_else(|| {
            self.stack.insert(vreg, self.stack_size);
            let offset = self.stack_size;
            self.stack_size += size.to_bytes().max(8) as u32;
            offset
        });

        (vreg, offset)
    }

    fn get_stack_offset(&self, var: &str) -> u32 {
        let vreg = self
            .var_to_vreg
            .get(var)
            .unwrap_or_else(|| panic!("undefined variable '{}'", var));

        *self
            .stack
            .get(vreg)
            .unwrap_or_else(|| panic!("variable {} is not on the stack", var))
    }

    fn stack_offset_of_expr(&self, expr: Expression<'s, TypeId>) -> u32 {
        match expr.inner {
            ExprInner::Ident(var) => self.get_stack_offset(var),
            ExprInner::MemberAccess(expr, member, typeid) => {
                self.stack_offset_of_expr(*expr)
                    + self
                        .analyzer
                        .types
                        .offset_of_member(typeid.unwrap(), member)
                        .unwrap() as u32
            }
            _ => panic!("cannot get stack offset of expression"),
        }
    }

    fn get_vreg(&mut self, size: VregSize) -> VirtualReg {
        let vreg = VirtualReg(self.vreg_counter);
        self.vreg_counter += 1;
        self.block_decls.push(vreg);
        self.size_map.insert(vreg, size);
        vreg
    }

    fn src_to_vreg(&mut self, src: SourceVal) -> VirtualReg {
        match src {
            SourceVal::Immediate(_) | SourceVal::String(_) | SourceVal::StaticMem(_) => {
                let dest = self.get_vreg(VregSize::Doubleword);
                self.block_ops.push(Op::Assign { src, dest });
                dest
            }
            SourceVal::VReg(vreg) => vreg,
        }
    }
}

#[derive(Copy, Clone)]
enum ConstructPlacement {
    Heap { base: VirtualReg, offset: u32 },
    Stack { offset: u32 },
}
