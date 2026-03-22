use std::{ops::Range, path::PathBuf, rc::Rc};

use crate::analyze::{
    Error, ErrorCode, ErrorContext, ErrorVec, Span,
    ast::{
        AST, ArithmeticOp, Assignable, CompareOp, ExprType, Expression, Item, SemanticType,
        Statement,
    },
    lex::{
        Lexer,
        token::{Keyword, Operator, Token},
    },
};

pub struct Parser {
    err_ctx: ErrorContext,
    src_path: Rc<PathBuf>,
    lexer: Lexer,
}

impl Parser {
    pub fn new(src_path: Rc<PathBuf>, lexer: Lexer) -> Self {
        Self {
            err_ctx: ErrorContext::new(),
            src_path,
            lexer,
        }
    }

    pub fn into_ast(mut self) -> Result<AST, ErrorVec> {
        let mut ast = AST::new();

        let result = self.parse(&mut ast);
        let mut errors = self.err_ctx.take_errors();
        if let Err(err) = result {
            errors.0.push(err);
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(ast)
    }

    fn parse(&mut self, ast: &mut AST) -> Result<(), Error> {
        while self.lexer.current().is_some() {
            let item = self.parse_item()?;
            ast.add_item(item);
        }

        Ok(())
    }

    fn find_semicolon(&mut self) -> Result<bool, Error> {
        while let Some((token, _)) = self.lexer.take_current()? {
            if matches!(token, Token::Semicolon) {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn parse_item(&mut self) -> Result<Item, Error> {
        let (token, range) = self.expect_take_current()?;
        let Token::Keyword(keyword) = token else {
            return Err(self
                .err_ctx
                .unexpected_token(self.span(range), "expected keyword")
                .finish());
        };

        match keyword {
            Keyword::Function => self.parse_function(range.start),
            Keyword::Use => unimplemented!(),
            Keyword::Extern => self.parse_extern(),
            _ => Err(self
                .err_ctx
                .unexpected_token(self.span(range), "expected function or extern import")
                .finish()),
        }
    }

    fn parse_extern(&mut self) -> Result<Item, Error> {
        match self.parse_extern_inner() {
            Ok(item) => Ok(item),
            Err(err) => {
                self.err_ctx.report(err);

                Ok(Item::ExternLib(String::new()))
            }
        }
    }

    fn parse_extern_inner(&mut self) -> Result<Item, Error> {
        let (token, range) = self.expect_take_current()?;
        let Token::Ident(lib) = token else {
            return Err(self
                .err_ctx
                .unexpected_token(self.span(range), "expected library name")
                .finish());
        };

        self.expect_semicolon()?;

        Ok(Item::ExternLib(lib))
    }

    fn parse_function(&mut self, decl_start: usize) -> Result<Item, Error> {
        let (token, range) = self.expect_take_current()?;

        let name = match token {
            Token::Ident(name) => name,
            _ => {
                self.err_ctx
                    .unexpected_token(self.span(range), "expected function name")
                    .report();

                String::from("???")
            }
        };

        self.expect_token(Token::LeftParenthesis, "expected opening parenthesis")?;

        let args = self.parse_decl_args()?;

        self.expect_token(
            Token::RightParenthesis,
            "expected argument or closing parenthesis",
        )?;

        let ret_type = match self.lexer.current() {
            Some((Token::Arrow, _)) => {
                self.lexer.lex_one()?;
                self.parse_type()?
            }
            _ => SemanticType::Unit,
        };

        let decl_end = self.lexer.last_token_end();

        let body = self.parse_block()?;

        Ok(Item::Function {
            name,
            args,
            body,
            ret_type,
            decl_span: self.span(decl_start..decl_end),
        })
    }

    fn parse_decl_args(&mut self) -> Result<Vec<(String, SemanticType, Span)>, Error> {
        let mut args = Vec::new();
        while let Some((Token::Ident(name), _)) = self.lexer.current() {
            let name = name.to_owned();

            let rstart = self.lexer.cur_token_start();

            self.lexer.lex_one()?;
            self.expect_matches(
                |t| matches!(t, Token::Colon),
                "expected colon and argument type",
            )?;

            let arg_type = self.parse_type()?;

            let rend = self.lexer.last_token_end();

            args.push((name, arg_type, self.span(rstart..rend)));

            if !matches!(self.lexer.current(), Some((Token::RightParenthesis, _))) {
                self.expect_token(Token::Comma, "expected comma")?;
            }
        }

        Ok(args)
    }

    fn parse_type(&mut self) -> Result<SemanticType, Error> {
        let (type_token, range) = self.expect_take_current()?;
        match type_token {
            Token::Reference => self
                .parse_type()
                .map(|t| SemanticType::Pointer(Box::new(t))),
            Token::Ident(type_str) => Ok(SemanticType::from(type_str)),
            _ => Err(self
                .err_ctx
                .unexpected_token(self.span(range), "expected argument type")
                .finish()),
        }
    }

    fn parse_block(&mut self) -> Result<Vec<Statement>, Error> {
        self.expect_token(Token::LeftCurlyBracket, "expected block")?;

        let mut statements = Vec::new();
        while let Some((token, _)) = self.lexer.current() {
            if matches!(token, Token::RightCurlyBracket) {
                self.lexer.take_current()?;
                return Ok(statements);
            }

            match self.parse_statement() {
                Ok(stmt) => statements.push(stmt),
                Err(err) => {
                    self.err_ctx.report(err);
                    self.find_semicolon()?;
                }
            }
        }

        Err(self.err_ctx.unexpected_eof(self.span_eof()).finish())
    }

    fn parse_statement(&mut self) -> Result<Statement, Error> {
        let (token, range) = self.lexer.current().unwrap().clone();

        if let Token::Keyword(keyword) = token {
            self.lexer.take_current()?;
            self.parse_keyword(keyword, range.clone())
        } else {
            let expr = self.parse_expr()?;

            match self.lexer.take_current()? {
                Some((Token::Semicolon, _)) => Ok(Statement::Expr(expr)),
                Some((Token::Assign, _)) => {
                    let var = match expr.expr_type {
                        ExprType::Variable(var) => Assignable::Var(var),
                        ExprType::Deref(var) => Assignable::Ptr(var),
                        _ => {
                            return Err(self
                                .err_ctx
                                .error(expr.span.clone())
                                .with_message("invalid assignment")
                                .with_label(expr.span, "only variables are allowed in assignments")
                                .finish());
                        }
                    };

                    let rvalue = self.parse_expr()?;
                    self.expect_semicolon()?;
                    Ok(Statement::Assign {
                        var,
                        expr: rvalue,
                        var_span: self.span(range),
                    })
                }
                Some((Token::Declare, _)) => {
                    let ExprType::Variable(var) = expr.expr_type else {
                        return Err(self
                            .err_ctx
                            .error(expr.span.clone())
                            .with_message("invalid assignment")
                            .with_label(expr.span, "only variables are allowed in assignments")
                            .finish());
                    };

                    let rvalue = self.parse_expr()?;
                    self.expect_semicolon()?;
                    Ok(Statement::Declare {
                        var,
                        expr: rvalue,
                        var_span: self.span(range),
                    })
                }
                Some((_, range)) => Err(self
                    .err_ctx
                    .unexpected_token(self.span(range), "expected ';', '=' or ':='")
                    .finish()),
                None => Err(self.err_ctx.unexpected_eof(self.span_eof()).finish()),
            }
        }
    }

    fn parse_keyword(&mut self, keyword: Keyword, range: Range<usize>) -> Result<Statement, Error> {
        match keyword {
            Keyword::Return => self.parse_return(),
            Keyword::If => self.parse_if(),
            _ => Err(self
                .err_ctx
                .unexpected_token(self.span(range), "unexpected keyword")
                .finish()),
        }
    }

    fn parse_return(&mut self) -> Result<Statement, Error> {
        let expr = self.parse_expr()?;
        self.expect_semicolon()?;

        Ok(Statement::Return(expr))
    }

    fn parse_if(&mut self) -> Result<Statement, Error> {
        let guard = self.parse_expr()?;
        let body = self.parse_block()?;

        Ok(Statement::If { guard, body })
    }

    fn parse_expr(&mut self) -> Result<Expression, Error> {
        let mut lhs = self.parse_single_expr()?;

        if let Some((Token::Operator(op), _)) = self.lexer.current() {
            let mut op = *op;

            let left_bind_power = op.precedence();

            self.lexer.take_current()?;

            let right_side = match self.lexer.peek() {
                Some((Token::Operator(next_op), _)) => Some((next_op.precedence(), *next_op)),
                _ => None,
            };

            let rhs = if let Some((right_bind_power, next_op)) = right_side
                && right_bind_power < left_bind_power
            {
                let rhs = self.parse_single_expr()?;
                lhs = Expression {
                    expr_type: self.bind_expr(op, lhs, rhs),
                    span: self.span(0..1),
                };

                self.lexer.lex_one()?;
                op = next_op;

                self.parse_expr()?
            } else {
                self.parse_expr()?
            };

            let span = self.span((lhs.span.1.start)..(rhs.span.1.end));
            let expr_type = self.bind_expr(op, lhs, rhs);

            return Ok(Expression { expr_type, span });
        }

        Ok(lhs)
    }

    fn bind_expr(&mut self, op: Operator, lhs: Expression, rhs: Expression) -> ExprType {
        match op {
            Operator::Plus => {
                ExprType::Arithmetic(Box::new(lhs), Box::new(rhs), ArithmeticOp::Add, None)
            }
            Operator::Minus => {
                ExprType::Arithmetic(Box::new(lhs), Box::new(rhs), ArithmeticOp::Sub, None)
            }
            Operator::Star => {
                ExprType::Arithmetic(Box::new(lhs), Box::new(rhs), ArithmeticOp::Mult, None)
            }
            Operator::Slash => {
                ExprType::Arithmetic(Box::new(lhs), Box::new(rhs), ArithmeticOp::Div, None)
            }
            Operator::Equal => {
                ExprType::Comparison(Box::new(lhs), Box::new(rhs), CompareOp::Equal, None)
            }
            Operator::NotEqual => {
                ExprType::Comparison(Box::new(lhs), Box::new(rhs), CompareOp::NotEqual, None)
            }
            Operator::Less => {
                ExprType::Comparison(Box::new(lhs), Box::new(rhs), CompareOp::Less, None)
            }
            Operator::LessOrEqual => {
                ExprType::Comparison(Box::new(lhs), Box::new(rhs), CompareOp::LessOrEqual, None)
            }
            Operator::Greater => {
                ExprType::Comparison(Box::new(lhs), Box::new(rhs), CompareOp::Greater, None)
            }
            Operator::GreaterOrEqual => ExprType::Comparison(
                Box::new(lhs),
                Box::new(rhs),
                CompareOp::GreaterOrEqual,
                None,
            ),
        }
    }

    fn parse_single_expr(&mut self) -> Result<Expression, Error> {
        let token = self.lexer.take_current()?;
        match token {
            Some((Token::Number(num), range)) => Ok(Expression {
                expr_type: ExprType::Const(num),
                span: self.span(range),
            }),
            Some((Token::Reference, ref_range)) => {
                let (token, var_range) = self.expect_take_current()?;
                let Token::Ident(var) = token else {
                    let var_span = self.span(var_range);
                    return Err(self
                        .err_ctx
                        .error(self.span(ref_range))
                        .with_message("invalid pointer")
                        .with_label(var_span, "expected variable")
                        .finish());
                };

                Ok(Expression {
                    expr_type: ExprType::Pointer(var),
                    span: self.span(ref_range.start..var_range.end),
                })
            }
            Some((Token::Operator(Operator::Star), deref_range)) => {
                let (token, var_range) = self.expect_take_current()?;
                let Token::Ident(var) = token else {
                    let var_span = self.span(var_range);
                    return Err(self
                        .err_ctx
                        .error(self.span(deref_range))
                        .with_message("invalid pointer deref")
                        .with_label(var_span, "expected variable")
                        .finish());
                };

                Ok(Expression {
                    expr_type: ExprType::Deref(var),
                    span: self.span(deref_range.start..var_range.end),
                })
            }
            Some((Token::Ident(ident), range)) => self.parse_ident_expr(ident, range),
            Some((Token::Character(c), range)) => Ok(Expression {
                expr_type: ExprType::Character(c),
                span: self.span(range),
            }),
            Some((Token::Bool(b), range)) => Ok(Expression {
                expr_type: ExprType::Bool(b),
                span: self.span(range),
            }),
            Some((_, range)) => Err(self
                .err_ctx
                .unexpected_token(self.span(range), "invalid expression")
                .finish()),
            None => Err(self.err_ctx.unexpected_eof(self.span_eof()).finish()),
        }
    }

    fn parse_ident_expr(
        &mut self,
        mut ident: String,
        range: Range<usize>,
    ) -> Result<Expression, Error> {
        while matches!(self.lexer.current(), Some((Token::PathSeparator, _))) {
            self.lexer.lex_one()?;
            let (token, range) = self.expect_take_current()?;
            let Token::Ident(sub_ident) = token else {
                return Err(self
                    .err_ctx
                    .unexpected_token(self.span(range), "expected identifier")
                    .finish());
            };

            ident.push_str("::");
            ident.push_str(&sub_ident);
        }

        if matches!(self.lexer.current(), Some((Token::LeftParenthesis, _))) {
            self.lexer.take_current()?;

            let args = self.parse_call_args()?;

            self.expect_token(Token::RightParenthesis, "expected closing parenthesis")?;

            Ok(Expression {
                expr_type: ExprType::FnCall(ident, args),
                span: self.span((range.start)..(self.lexer.last_token_end())),
            })
        } else {
            Ok(Expression {
                expr_type: ExprType::Variable(ident),
                span: self.span((range.start)..(self.lexer.last_token_end())),
            })
        }
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expression>, Error> {
        let mut args = Vec::new();
        let mut first = true;

        while !matches!(self.lexer.current(), Some((Token::RightParenthesis, _))) {
            if !first {
                self.expect_matches(|t| matches!(t, Token::Comma), "expected comma")?;
            }

            let expr = self.parse_expr()?;
            args.push(expr);

            first = false;
        }

        Ok(args)
    }

    fn expect_token(&mut self, token: Token, message: impl ToString) -> Result<(), Error> {
        self.expect_matches(|t| t == &token, message)
    }

    fn expect_matches<F>(&mut self, matches: F, message: impl ToString) -> Result<(), Error>
    where
        F: FnOnce(&Token) -> bool,
    {
        let (token, range) = self.expect_take_current()?;
        if !matches(&token) {
            return Err(self
                .err_ctx
                .unexpected_token(self.span(range), message)
                .finish());
        }

        Ok(())
    }

    fn expect_semicolon(&mut self) -> Result<(), Error> {
        let current = self.lexer.take_current()?;
        if !matches!(current, Some((Token::Semicolon, _))) {
            let pos = current
                .map(|t| t.1.start)
                .unwrap_or(self.lexer.cur_token_start());

            let insert_span = self.span((pos - 1)..pos);
            self.err_ctx
                .error(self.span(pos..(pos + 1)))
                .with_code(ErrorCode::MissingSemicolon)
                .with_message("expected semicolon")
                .with_label(insert_span, "insert the semicolon dummy")
                .report();
        }

        Ok(())
    }

    fn expect_take_current(&mut self) -> Result<(Token, Range<usize>), Error> {
        let token = self.lexer.take_current()?;
        match token {
            Some(token) => Ok(token),
            None => Err(self.err_ctx.unexpected_eof(self.span_eof()).finish()),
        }
    }

    fn span(&self, range: Range<usize>) -> (Rc<PathBuf>, Range<usize>) {
        (self.src_path.clone(), range)
    }

    fn span_eof(&self) -> (Rc<PathBuf>, Range<usize>) {
        let end = self.lexer.cur_token_start();
        self.span((end - 1)..end)
    }
}
