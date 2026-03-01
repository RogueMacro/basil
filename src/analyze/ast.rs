use std::{fmt, ops::Range};

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
    Return(Expression),
    Expr(Expression),
}

#[derive(Debug, Clone)]
pub struct Expression {
    pub expr_type: ExprType,
    // pub semantic_type: SemanticType,
    pub range: Range<usize>,
}

#[derive(Debug, Clone)]
pub enum ExprType {
    Const(i64),
    Character(char),
    Bool(bool),

    Variable(String),

    Addition(Box<Expression>, Box<Expression>),
    Subtraction(Box<Expression>, Box<Expression>),
    Multiplication(Box<Expression>, Box<Expression>),
    Division(Box<Expression>, Box<Expression>),

    FnCall(String, Vec<Expression>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SemanticType {
    Unit,
    I64,
    Char,
    Bool,
    UserType(String),
}

impl fmt::Display for SemanticType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SemanticType::Unit => write!(f, "()"),
            SemanticType::I64 => write!(f, "i64"),
            SemanticType::Char => write!(f, "char"),
            SemanticType::Bool => write!(f, "bool"),
            SemanticType::UserType(typ) => write!(f, "{}", typ),
        }
    }
}

impl<S: AsRef<str>> From<S> for SemanticType {
    fn from(string: S) -> Self {
        match string.as_ref() {
            "()" => Self::Unit,
            "i64" => Self::I64,
            "char" => Self::Char,
            "bool" => Self::Bool,
            name => Self::UserType(name.to_owned()),
        }
    }
}
