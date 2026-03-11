use std::{collections::HashMap, fmt, ops::Range, rc::Rc};

use crate::analyze::{
    ErrorContext, ErrorVec,
    ast::{AST, ExprType, Expression, Item, Statement},
};

pub struct ValidAST(pub AST);

const MAIN_FN: &str = "main";

pub fn analyze(mut ast: AST, source_name: Rc<String>) -> Result<ValidAST, ErrorVec> {
    let analyzer = Analyzer::new(source_name);
    analyzer.analyze(&mut ast)?;

    Ok(ValidAST(ast))
}

struct Analyzer {
    err_ctx: ErrorContext,

    variables: HashMap<String, SemanticType>,
    functions: HashMap<
        String,
        (
            Range<usize>,
            SemanticType,
            Vec<(Range<usize>, SemanticType)>,
        ),
    >,
}

impl Analyzer {
    pub fn new(source_name: Rc<String>) -> Self {
        Self {
            err_ctx: ErrorContext::new(source_name),
            variables: HashMap::new(),
            functions: HashMap::new(),
        }
    }

    pub fn analyze(mut self, ast: &mut AST) -> Result<(), ErrorVec> {
        for item in &ast.items {
            if let Item::Function {
                name,
                ret_type,
                decl_range,
                args,
                ..
            } = item
            {
                let args = args
                    .iter()
                    .map(|(_, t, r)| (r.clone(), t.clone()))
                    .collect();
                if let Some((other_decl_range, _, _)) = self.functions.insert(
                    name.to_owned(),
                    (decl_range.clone(), ret_type.to_owned(), args),
                ) {
                    self.err_ctx
                        .build(decl_range.clone())
                        .with_message("duplicate function definition")
                        .with_label(decl_range.clone(), "defined here")
                        .with_label(other_decl_range.clone(), "first defined here")
                        .report();
                }
            }
        }

        for item in &mut ast.items {
            self.item(item);
        }

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
                decl_range,
                ret_type,
            } => {
                for (arg, typ, _) in args {
                    self.variables.insert(arg.to_owned(), typ.clone());
                }

                let has_return = self.body(body, ret_type, decl_range);

                if !has_return && name == MAIN_FN {
                    self.err_ctx
                        .build(decl_range.clone())
                        .with_message("no return statement found in function main")
                        .with_label(decl_range.clone(), "main must return a value")
                        .report();
                }
            }
            Item::Use { lib, item } => todo!(),
        }
    }

    /// Returns whether this statement contains a return statement
    fn body(
        &mut self,
        body: &mut [Statement],
        fn_ret_type: &SemanticType,
        fn_decl_range: &Range<usize>,
    ) -> bool {
        let mut has_return = false;
        for stmt in body {
            if self.statement(stmt, fn_ret_type, fn_decl_range) {
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
        fn_decl_range: &Range<usize>,
    ) -> bool {
        match stmt {
            Statement::Declare {
                var,
                expr,
                var_range,
            } => {
                let var_type = self.expression(expr);
                if self
                    .variables
                    .insert(var.clone(), var_type.unwrap_or(SemanticType::Unit))
                    .is_some()
                {
                    self.err_ctx
                        .build(var_range.clone())
                        .with_message("duplicate variable declaration")
                        .with_label(var_range.clone(), "variable already defined")
                        .report();
                }
            }
            Statement::Assign {
                var,
                expr,
                var_range,
            } => {
                let assign_type = self.expression(expr);
                let decl_type = self.check_var(var, var_range);

                if let Some(assign_type) = assign_type
                    && let Some(decl_type) = decl_type
                    && assign_type != decl_type
                {
                    self.err_ctx
                        .build(var_range.start..expr.range.end)
                        .with_message("mismatched types")
                        .with_label(var_range.clone(), format!("this is of type {}", decl_type))
                        .with_label(
                            expr.range.clone(),
                            format!("this is of type {}", assign_type),
                        )
                        .report();
                }
            }
            Statement::If { guard, body } => {
                if let Some(typ) = self.expression(guard)
                    && typ != SemanticType::Bool
                {
                    self.err_ctx
                        .build(guard.range.clone())
                        .with_message("unexpected type")
                        .with_label(
                            guard.range.clone(),
                            format!("expected type 'bool', got '{}'", typ),
                        )
                        .report();
                }

                return self.body(body, fn_ret_type, fn_decl_range);
            }
            Statement::Return(expr) | Statement::Expr(expr) => {
                if let Some(typ) = self.expression(expr)
                    && &typ != fn_ret_type
                {
                    self.err_ctx
                        .build(expr.range.clone())
                        .with_message("incompatible types")
                        .with_label(expr.range.clone(), format!("this is of type {}", typ))
                        .with_label(
                            fn_decl_range.clone(),
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
        match &mut expr.expr_type {
            ExprType::Const(_) => Some(SemanticType::I64),
            ExprType::Character(_) => Some(SemanticType::Char),
            ExprType::Bool(_) => Some(SemanticType::Bool),

            ExprType::Variable(var) => self.check_var(var, &expr.range),

            ExprType::Arithmetic(expr1, expr2, _op, expr_sign) => {
                if let Some(type1) = self.expression(expr1)
                    && let Some(type2) = self.expression(expr2)
                {
                    if type1 == type2 {
                        if let Some(type_sign) = type1.sign() {
                            *expr_sign = Some(type_sign);
                            return Some(type1);
                        }

                        self.err_ctx
                            .build(expr1.range.start..expr2.range.end)
                            .with_message("mismatched arithmetic types")
                            .with_label(
                                expr1.range.clone(),
                                "arithmetic only allowed on integer types",
                            )
                            .report();
                    }

                    self.err_ctx
                        .build(expr1.range.start..expr2.range.end)
                        .with_message("mismatched types")
                        .with_label(expr1.range.clone(), format!("this is of type {}", type1))
                        .with_label(expr2.range.clone(), format!("this is of type {}", type2))
                        .report();
                }

                None
            }

            ExprType::Comparison(expr1, expr2, _op, expr_sign) => {
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
                            .build(expr1.range.start..expr2.range.end)
                            .with_message(
                                "mismatched comparison types, must have same sign/no sign",
                            )
                            .with_label(expr1.range.clone(), format!("this is {}", sign1_str))
                            .with_label(expr2.range.clone(), format!("this is {}", sign2_str))
                            .report();
                    }

                    self.err_ctx
                        .build(expr1.range.start..expr2.range.end)
                        .with_message("mismatched types")
                        .with_label(expr1.range.clone(), format!("this is of type {}", type1))
                        .with_label(expr2.range.clone(), format!("this is of type {}", type2))
                        .report();
                }

                None
            }

            ExprType::FnCall(function, call_args) => {
                let call_types: Vec<(SemanticType, Range<usize>)> = call_args
                    .iter_mut()
                    .filter_map(|e| self.expression(e).map(|t| (t, e.range.clone())))
                    .collect();

                if let Some((_, ret_type, decl_args)) = self.functions.get(function) {
                    for ((call_type, call_range), (decl_range, decl_type)) in
                        call_types.iter().zip(decl_args)
                    {
                        if call_type != decl_type {
                            self.err_ctx
                                .build(call_range.clone())
                                .with_message("incompatible types")
                                .with_label(
                                    call_range.clone(),
                                    format!("this is of type {}", call_type),
                                )
                                .with_label(
                                    decl_range.clone(),
                                    format!("function accepts argument of type {}", decl_type),
                                )
                                .report();
                        }
                    }

                    return Some(ret_type.clone());
                } else {
                    self.err_ctx
                        .build(expr.range.clone())
                        .with_message("invalid function call")
                        .with_label(
                            expr.range.clone(),
                            format!("{} is not a function", function),
                        )
                        .report();
                }

                None
            }
        }
    }

    fn check_var(&mut self, symbol: &str, range: &Range<usize>) -> Option<SemanticType> {
        if let Some(typ) = self.variables.get(symbol) {
            return Some(typ.clone());
        }

        self.err_ctx
            .build(range.clone())
            .with_message("undeclared variable")
            .with_label(range.clone(), "this guy doesn't exist")
            .report();

        None
    }
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
            SemanticType::UserType(_) => None,
        }
    }
}

impl<S: AsRef<str>> From<S> for SemanticType {
    fn from(string: S) -> Self {
        match string.as_ref() {
            "()" => Self::Unit,
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
            SemanticType::UserType(typ) => write!(f, "{}", typ),
        }
    }
}
