use std::ops::Range;

use crate::analyze::semantics::{SemanticType, Sign};

pub mod parse;

#[derive(Default, Debug)]
pub struct AST {
    pub items: Vec<Item>,
}

impl AST {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn add_item(&mut self, item: Item) {
        self.items.push(item);
    }

    pub fn imports(&self) -> impl Iterator<Item = (&str, &str)> {
        self.items.iter().filter_map(|i| {
            if let Item::Use { lib, item } = i {
                Some((lib.as_str(), item.as_str()))
            } else {
                None
            }
        })
    }
}

#[derive(Debug)]
pub enum Item {
    Function {
        name: String,
        args: Vec<(String, SemanticType, Range<usize>)>,
        body: Vec<Statement>,
        ret_type: SemanticType,
        decl_range: Range<usize>,
    },
    Use {
        lib: String,
        item: String,
    },
}

#[derive(Debug)]
pub enum Statement {
    Declare {
        var: String,
        expr: Expression,
        var_range: Range<usize>,
    },
    Assign {
        var: String,
        expr: Expression,
        var_range: Range<usize>,
    },
    If {
        guard: Expression,
        body: Vec<Statement>,
    },
    Return(Expression),
    Expr(Expression),
}

#[derive(Debug, Clone)]
pub struct Expression {
    pub expr_type: ExprType,
    pub range: Range<usize>,
}

#[derive(Debug, Clone)]
pub enum ExprType {
    Const(i64),
    Character(char),
    Bool(bool),

    Variable(String),

    Arithmetic(Box<Expression>, Box<Expression>, ArithmeticOp, Option<Sign>),
    Comparison(Box<Expression>, Box<Expression>, CompareOp, Option<Sign>),

    // Addition(Box<Expression>, Box<Expression>),
    // Subtraction(Box<Expression>, Box<Expression>),
    // Multiplication(Box<Expression>, Box<Expression>),
    // Division(Box<Expression>, Box<Expression>),
    FnCall(String, Vec<Expression>),
}

#[derive(Debug, Clone, Copy)]
pub enum ArithmeticOp {
    Add,
    Sub,
    Mult,
    Div,
}

#[derive(Debug, Clone, Copy)]
pub enum CompareOp {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}
