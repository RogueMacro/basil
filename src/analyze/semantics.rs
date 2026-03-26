use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use crate::analyze::{
    ErrorContext, ErrorVec, Span,
    ast::{AST, Assignable, ExprInner, Expression, Item, Statement},
};

pub struct ValidAST(pub AST);

const MAIN_FN: &str = "main";

pub fn analyze(mut ast: AST) -> Result<ValidAST, ErrorVec> {
    let analyzer = Analyzer::new();
    analyzer.analyze(&mut ast)?;

    Ok(ValidAST(ast))
}

struct Analyzer {
    err_ctx: ErrorContext,

    variables: HashMap<String, SemanticType>,
    functions: HashMap<String, (Span, SemanticType, Vec<(Span, SemanticType)>)>,
    called_funcs: HashSet<String>,
}

impl Analyzer {
    pub fn new() -> Self {
        Self {
            err_ctx: ErrorContext::new(),
            variables: HashMap::new(),
            functions: HashMap::new(),
            called_funcs: HashSet::from([String::from(MAIN_FN)]),
        }
    }

    pub fn analyze(mut self, ast: &mut AST) -> Result<(), ErrorVec> {
        for item in &ast.items {
            if let Item::Function {
                name,
                ret_type,
                decl_span,
                args,
                ..
            }
            | Item::ForwardDecl {
                name,
                ret_type,
                decl_span,
                args,
            } = item
            {
                let args = args
                    .iter()
                    .map(|(_, typ, span)| (span.clone(), typ.clone()))
                    .collect();

                if let Some((other_decl_span, _, _)) = self.functions.insert(
                    name.to_owned(),
                    (decl_span.clone(), ret_type.to_owned(), args),
                ) {
                    self.err_ctx
                        .error(decl_span.clone())
                        .with_message("duplicate function definition")
                        .with_label(decl_span.clone(), "defined here")
                        .with_label(other_decl_span.clone(), "first defined here")
                        .report();
                }
            }
        }

        for item in &mut ast.items {
            self.item(item);
        }

        ast.items.retain(|item| match item {
            Item::Function { name, .. } => self.called_funcs.contains(name),
            Item::ForwardDecl { .. } => false,
            Item::ExternLib(_) => false,
        });

        // for item in &ast.items {
        //     if let Item::Function {
        //         name, decl_span, ..
        //     } = item
        //         && name != MAIN_FN
        //         && !self.called_funcs.contains(name)
        //     {
        //         // self.err_ctx
        //         //     .warn(decl_span.clone())
        //         //     .with_message("unused function")
        //         //     .with_label(decl_span.clone(), "function is never used")
        //         //     .report();
        //     }
        // }

        if !self.err_ctx.is_empty() {
            return Err(self.err_ctx.take_errors());
        }

        Ok(())
    }

    fn item(&mut self, item: &mut Item) {
        self.variables.clear();

        match item {
            Item::Function {
                name,
                args,
                body,
                decl_span,
                ret_type,
            } => {
                for (arg, typ, _) in args {
                    self.variables.insert(arg.to_owned(), typ.clone());
                }

                let has_return = self.body(body, ret_type, decl_span);

                if !has_return && name == MAIN_FN {
                    self.err_ctx
                        .error(decl_span.clone())
                        .with_message("no return statement found in function main")
                        .with_label(decl_span.clone(), "main must return a value")
                        .report();
                }
            }
            Item::ForwardDecl { .. } => {}
            Item::ExternLib(_lib) => (), // TODO: maybe?
        }
    }

    /// Returns whether this statement contains a return statement
    fn body(
        &mut self,
        body: &mut [Statement],
        fn_ret_type: &SemanticType,
        fn_decl_span: &Span,
    ) -> bool {
        let mut has_return = false;
        for stmt in body {
            if self.statement(stmt, fn_ret_type, fn_decl_span) {
                has_return = true;
            }
        }

        has_return
    }

    /// Returns whether this statement contains a return statement
    fn statement(
        &mut self,
        stmt: &mut Statement,
        fn_ret_type: &SemanticType,
        fn_decl_span: &Span,
    ) -> bool {
        match stmt {
            Statement::Declare {
                var,
                expr,
                var_span,
            } => {
                let var_type = self.expression(expr);
                if self
                    .variables
                    .insert(var.clone(), var_type.unwrap_or(SemanticType::Unit))
                    .is_some()
                {
                    self.err_ctx
                        .error(var_span.clone())
                        .with_message("duplicate variable declaration")
                        .with_label(var_span.clone(), "variable already defined")
                        .report();
                }
            }
            Statement::Assign {
                var,
                expr,
                var_span,
            } => {
                let assign_type = self.expression(expr);
                let decl_type = match var {
                    Assignable::Var(var) => self.check_var(var, var_span),
                    Assignable::Ptr(ptr) => self.check_ptr(ptr, var_span),
                };

                if let Some(assign_type) = assign_type
                    && let Some(decl_type) = decl_type
                    && assign_type != decl_type
                {
                    self.err_ctx
                        .error(combine_span(var_span, &expr.span))
                        .with_message("mismatched types")
                        .with_label(var_span.clone(), format!("this is of type {}", decl_type))
                        .with_label(
                            expr.span.clone(),
                            format!("this is of type {}", assign_type),
                        )
                        .report();
                }
            }
            Statement::If { guard, body } | Statement::WhileLoop { guard, body } => {
                if let Some(typ) = self.expression(guard)
                    && typ != SemanticType::Bool
                {
                    self.err_ctx
                        .error(guard.span.clone())
                        .with_message("unexpected type")
                        .with_label(
                            guard.span.clone(),
                            format!("expected type 'bool', got '{}'", typ),
                        )
                        .report();
                }

                return self.body(body, fn_ret_type, fn_decl_span);
            }
            Statement::Expr(expr) => {
                self.expression(expr);
            }
            Statement::Return(expr) => {
                if let Some(typ) = self.expression(expr)
                    && &typ != fn_ret_type
                {
                    self.err_ctx
                        .error(expr.span.clone())
                        .with_message("incompatible types")
                        .with_label(expr.span.clone(), format!("this is of type {}", typ))
                        .with_label(
                            fn_decl_span.clone(),
                            format!("function returns {}", fn_ret_type),
                        )
                        .report();
                }

                return true;
            }
        }

        false
    }

    fn expression(&mut self, expr: &mut Expression) -> Option<SemanticType> {
        match &mut expr.inner {
            ExprInner::Const(_) => Some(SemanticType::I64),
            ExprInner::Character(_) => Some(SemanticType::Char),
            ExprInner::String(_) => Some(SemanticType::Pointer(Box::new(SemanticType::Char))),
            ExprInner::Bool(_) => Some(SemanticType::Bool),

            ExprInner::Variable(var) => self.check_var(var, &expr.span),
            ExprInner::Pointer(var) => self
                .check_var(var, &expr.span)
                .map(|t| SemanticType::Pointer(Box::new(t))),
            ExprInner::Deref(var) => self.check_ptr(var, &expr.span),

            ExprInner::Arithmetic(expr1, expr2, _op, expr_sign) => {
                if let Some(type1) = self.expression(expr1)
                    && let Some(type2) = self.expression(expr2)
                {
                    if type1 == type2 {
                        if let Some(type_sign) = type1.sign() {
                            *expr_sign = Some(type_sign);
                            return Some(type1);
                        }

                        self.err_ctx
                            .error(combine_span(&expr1.span, &expr2.span))
                            .with_message("mismatched arithmetic types")
                            .with_label(
                                expr1.span.clone(),
                                "arithmetic only allowed on integer types",
                            )
                            .report();
                    }

                    self.err_ctx
                        .error(combine_span(&expr1.span, &expr2.span))
                        .with_message("mismatched types")
                        .with_label(expr1.span.clone(), format!("this is of type {}", type1))
                        .with_label(expr2.span.clone(), format!("this is of type {}", type2))
                        .report();
                }

                None
            }

            ExprInner::Comparison(expr1, expr2, _op, expr_sign) => {
                if let Some(type1) = self.expression(expr1)
                    && let Some(type2) = self.expression(expr2)
                {
                    if type1 == type2 {
                        let sign1 = type1.sign();
                        let sign2 = type2.sign();
                        if sign1 == sign2 {
                            *expr_sign = sign1;
                            return Some(SemanticType::Bool);
                        }

                        let sign1_str = match sign1 {
                            Some(Sign::Signed) => "a signed integer",
                            Some(Sign::Unsigned) => "an unsigned integer",
                            None => "not an integer",
                        };

                        let sign2_str = match sign2 {
                            Some(Sign::Signed) => "a signed integer",
                            Some(Sign::Unsigned) => "an unsigned integer",
                            None => "not an integer",
                        };

                        self.err_ctx
                            .error(combine_span(&expr1.span, &expr2.span))
                            .with_message(
                                "mismatched comparison types, must have same sign/no sign",
                            )
                            .with_label(expr1.span.clone(), format!("this is {}", sign1_str))
                            .with_label(expr2.span.clone(), format!("this is {}", sign2_str))
                            .report();
                    }

                    self.err_ctx
                        .error(combine_span(&expr1.span, &expr2.span))
                        .with_message("mismatched types")
                        .with_label(expr1.span.clone(), format!("this is of type {}", type1))
                        .with_label(expr2.span.clone(), format!("this is of type {}", type2))
                        .report();
                }

                None
            }

            ExprInner::Cast(expr, cast_to) => {
                if let Some(expr_type) = self.expression(expr) {
                    if expr_type.can_cast_to(cast_to) {
                        return Some(cast_to.clone());
                    }

                    self.err_ctx
                        .error(expr.span.clone())
                        .with_message("invalid type cast")
                        .with_label(
                            expr.span.clone(),
                            format!("cannot cast from {} to {}", expr_type, cast_to),
                        )
                        .report();
                }

                None
            }

            ExprInner::FnCall(function, call_args) => {
                let call_types: Vec<(SemanticType, Span)> = call_args
                    .iter_mut()
                    .filter_map(|e| self.expression(e).map(|t| (t, e.span.clone())))
                    .collect();

                if let Some((fn_decl_span, ret_type, decl_args)) = self.functions.get(function) {
                    if decl_args.len() != call_args.len() {
                        self.err_ctx
                            .error(expr.span.clone())
                            .with_message("invalid argument count")
                            .with_label(
                                expr.span.clone(),
                                format!(
                                    "expected {} arguments, got {}",
                                    decl_args.len(),
                                    call_args.len()
                                ),
                            )
                            .with_label(fn_decl_span.clone(), "function defined here")
                            .report();
                    }

                    for ((call_type, call_span), (decl_span, decl_type)) in
                        call_types.iter().zip(decl_args)
                    {
                        if call_type != decl_type {
                            self.err_ctx
                                .error(call_span.clone())
                                .with_message("incompatible types")
                                .with_label(
                                    call_span.clone(),
                                    format!("this is of type {}", call_type),
                                )
                                .with_label(
                                    decl_span.clone(),
                                    format!("function accepts argument of type {}", decl_type),
                                )
                                .report();
                        }
                    }

                    if !self.called_funcs.contains(function) {
                        self.called_funcs.insert(function.to_owned());
                    }

                    return Some(ret_type.clone());
                } else {
                    self.err_ctx
                        .error(expr.span.clone())
                        .with_message("invalid function call")
                        .with_label(expr.span.clone(), format!("{} is not a function", function))
                        .report();
                }

                None
            }
        }
    }

    fn check_var(&mut self, symbol: &str, span: &Span) -> Option<SemanticType> {
        if let Some(typ) = self.variables.get(symbol) {
            return Some(typ.clone());
        }

        self.err_ctx
            .error(span.clone())
            .with_message("undeclared variable")
            .with_label(span.clone(), "this guy doesn't exist")
            .report();

        None
    }

    fn check_ptr(&mut self, symbol: &str, span: &Span) -> Option<SemanticType> {
        if let Some(typ) = self.check_var(symbol, &span) {
            match typ {
                SemanticType::Pointer(typ) => return Some(*typ),
                typ => {
                    self.err_ctx
                        .error(span.clone())
                        .with_message("invalid pointer deref")
                        .with_label(span.clone(), format!("cannot derefence type {}", typ))
                        .report();
                }
            }
        }

        None
    }
}

fn combine_span(span: &Span, span_2: &Span) -> Span {
    (span.0.clone(), span.1.start..span_2.1.end)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sign {
    Signed,
    Unsigned,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SemanticType {
    Unit,
    I64,
    U64,
    Char,
    Bool,
    Pointer(Box<SemanticType>),
    UserType(String),
}

impl SemanticType {
    pub fn sign(&self) -> Option<Sign> {
        match self {
            SemanticType::Unit => None,
            SemanticType::I64 => Some(Sign::Signed),
            SemanticType::U64 => Some(Sign::Unsigned),
            SemanticType::Char => Some(Sign::Unsigned),
            SemanticType::Bool => None,
            SemanticType::Pointer(typ) => typ.sign(),
            SemanticType::UserType(_) => None,
        }
    }

    pub fn can_cast_to(&self, other: &SemanticType) -> bool {
        use SemanticType::*;

        matches!((self, other), (Char, I64) | (I64, Char) | (Pointer(_), I64))
    }
}

impl<S: AsRef<str>> From<S> for SemanticType {
    fn from(string: S) -> Self {
        let string = string.as_ref();

        if let Some(typ) = string.strip_suffix('*') {
            let typ = SemanticType::from(typ);
            return SemanticType::Pointer(Box::new(typ));
        }

        match string {
            "i64" => Self::I64,
            "u64" => Self::U64,
            "char" => Self::Char,
            "bool" => Self::Bool,
            name => Self::UserType(name.to_owned()),
        }
    }
}

impl fmt::Display for SemanticType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SemanticType::Unit => write!(f, "()"),
            SemanticType::I64 => write!(f, "i64"),
            SemanticType::U64 => write!(f, "u64"),
            SemanticType::Char => write!(f, "char"),
            SemanticType::Bool => write!(f, "bool"),
            SemanticType::Pointer(typ) => write!(f, "&{}", typ),
            SemanticType::UserType(typ) => write!(f, "{}", typ),
        }
    }
}
