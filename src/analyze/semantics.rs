use std::{collections::HashMap, ops::Range, rc::Rc};

use crate::analyze::{
    ErrorContext, ErrorVec,
    ast::{AST, ExprType, Expression, Item, SemanticType, Statement},
};

pub struct ValidAST(pub AST);

const MAIN_FN: &str = "main";

pub fn analyze(ast: AST, source_name: Rc<String>) -> Result<ValidAST, ErrorVec> {
    let mut analyzer = Analyzer::new(&ast, source_name);
    analyzer.analyze()?;

    Ok(ValidAST(ast))
}

struct Analyzer<'ast> {
    ast: &'ast AST,
    err_ctx: ErrorContext,

    variables: HashMap<String, SemanticType>,
    functions: HashMap<String, SemanticType>,
}

impl<'ast> Analyzer<'ast> {
    pub fn new(ast: &'ast AST, source_name: Rc<String>) -> Self {
        Self {
            ast,
            err_ctx: ErrorContext::new(source_name),
            variables: HashMap::new(),
            functions: HashMap::new(),
        }
    }

    pub fn analyze(&mut self) -> Result<(), ErrorVec> {
        for item in &self.ast.items {
            let Item::Function {
                name,
                ret_type,
                decl_range,
                ..
            } = item;
            if self
                .functions
                .insert(name.to_owned(), ret_type.to_owned())
                .is_some()
            {
                self.err_ctx
                    .build(decl_range.clone())
                    .with_message("duplicate function definition")
                    .with_label(decl_range.clone(), "already defined")
                    .report();
            }
        }

        for item in &self.ast.items {
            self.item(item);
        }

        if !self.err_ctx.is_empty() {
            return Err(self.err_ctx.take_errors());
        }

        Ok(())
    }

    fn item(&mut self, item: &Item) {
        self.variables.clear();

        let Item::Function {
            name,
            args,
            body,
            decl_range,
            ..
        } = item;

        for (arg, typ) in args {
            self.variables.insert(arg.to_owned(), typ.clone());
        }

        let mut has_return = false;
        for stmt in body {
            if matches!(stmt, Statement::Return(_)) {
                has_return = true;
            }

            self.statement(stmt);
        }

        if !has_return && name == MAIN_FN {
            self.err_ctx
                .build(decl_range.clone())
                .with_message("no return statement found in function main")
                .with_label(decl_range.clone(), "main must return a value")
                .report();
        }
    }

    fn statement(&mut self, stmt: &Statement) {
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
            Statement::Return(expr) | Statement::Expr(expr) => {
                self.expression(expr);
            }
        }
    }

    fn expression(&mut self, expr: &Expression) -> Option<SemanticType> {
        match &expr.expr_type {
            ExprType::Const(_) => Some(SemanticType::I64),
            ExprType::Character(_) => Some(SemanticType::Char),
            ExprType::Variable(var) => self.check_var(var, &expr.range),

            ExprType::Addition(expr1, expr2)
            | ExprType::Multiplication(expr1, expr2)
            | ExprType::Subtraction(expr1, expr2)
            | ExprType::Division(expr1, expr2) => {
                if let Some(type1) = self.expression(expr1)
                    && let Some(type2) = self.expression(expr2)
                {
                    if type1 == type2 {
                        return Some(type1);
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

            ExprType::FnCall(function, args /* TODO */) => {
                if let Some(typ) = self.functions.get(function) {
                    return Some(typ.clone());
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
