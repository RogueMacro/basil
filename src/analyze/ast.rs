use crate::analyze::{
    Span,
    semantics::{SemanticType, Sign},
};

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

    pub fn imports(&self) -> impl Iterator<Item = &str> {
        self.items.iter().filter_map(|i| {
            if let Item::ExternLib(lib) = i {
                Some(lib.as_str())
            } else {
                None
            }
        })
    }

    pub fn mangle(&mut self, lib: &str) {
        for item in self.items.iter_mut() {
            match item {
                Item::Function { name, .. } | Item::ForwardDecl { name, .. } => {
                    *name = format!("{}::{}", lib, name)
                }
                Item::ExternLib(_) => (),
            }
        }
    }
}

#[derive(Debug)]
pub enum Item {
    Function {
        name: String,
        args: Vec<(String, SemanticType, Span)>,
        body: Vec<Statement>,
        ret_type: SemanticType,
        decl_span: Span,
    },
    ForwardDecl {
        name: String,
        args: Vec<(String, SemanticType, Span)>,
        ret_type: SemanticType,
        decl_span: Span,
    },
    ExternLib(String),
}

#[derive(Debug)]
pub enum Statement {
    Declare {
        var: String,
        expr: Expression,
        var_span: Span,
    },
    Assign {
        var: Assignable,
        expr: Expression,
        var_span: Span,
    },
    If {
        guard: Expression,
        body: Vec<Statement>,
    },
    Return(Expression),
    Expr(Expression),
}

#[derive(Debug, Clone)]
pub enum Assignable {
    Var(String),
    Ptr(String),
}

impl Assignable {
    pub fn symbol(&self) -> &str {
        match self {
            Self::Var(var) | Self::Ptr(var) => var,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Expression {
    pub inner: ExprInner,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprInner {
    Const(i64),
    Character(char),
    Bool(bool),

    Variable(String),
    Pointer(String),
    Deref(String),

    Arithmetic(Box<Expression>, Box<Expression>, ArithmeticOp, Option<Sign>),
    Comparison(Box<Expression>, Box<Expression>, CompareOp, Option<Sign>),

    Cast(Box<Expression>, SemanticType),

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
