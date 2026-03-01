use std::{ops::Range, rc::Rc};

use crate::analyze::{
    Error, ErrorCode, ErrorContext, ErrorVec,
    ast::{AST, ExprType, Expression, Item, SemanticType, Statement},
    lex::{
        Lexer,
        token::{Keyword, Operator, Token},
    },
};

pub struct Parser {
    err_ctx: ErrorContext,
    lexer: Lexer,
}

impl Parser {
    pub fn new(source_name: Rc<String>, lexer: Lexer) -> Self {
        Self {
            err_ctx: ErrorContext::new(source_name),
            lexer,
        }
    }

    pub fn into_ast(mut self) -> Result<AST, ErrorVec> {
        let mut ast = AST::new();

        while self.lexer.current().is_some() {
            let item = self.parse_item()?;
            ast.add_item(item);
        }

        if !self.err_ctx.is_empty() {
            return Err(self.err_ctx.take_errors());
        }

        Ok(ast)
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
                .unexpected_token(range, "expected keyword")
                .finish());
        };

        match keyword {
            Keyword::Function => self.parse_function(range.start),
            _ => Err(self
                .err_ctx
                .unexpected_token(range, "expected function")
                .finish()),
        }
    }

    fn parse_function(&mut self, decl_start: usize) -> Result<Item, Error> {
        let (token, range) = self.expect_take_current()?;

        let name = match token {
            Token::Ident(name) => name,
            _ => {
                self.err_ctx
                    .unexpected_token(range, "expected function name")
                    .report();

                String::from("???")
            }
        };

        self.expect_next(
            |t| matches!(t, Token::Operator(Operator::LeftParenthesis)),
            "expected opening parenthesis",
        )?;

        let args = self.parse_decl_args()?;

        let decl_end = self.lexer.cur_token_start();

        self.expect_next(
            |t| matches!(t, Token::Operator(Operator::RightParenthesis)),
            "expected argument or closing parenthesis",
        )?;

        let ret_type = match self.lexer.current() {
            Some((Token::Operator(Operator::Arrow), _)) => {
                self.lexer.lex_one()?;
                let (token, range) = self.expect_take_current()?;
                if let Token::Ident(ret_typ_str) = token {
                    SemanticType::from(ret_typ_str)
                } else {
                    self.err_ctx
                        .unexpected_token(range, "expected return type")
                        .report();

                    SemanticType::Unit
                }
            }
            _ => SemanticType::Unit,
        };

        let body = self.parse_block()?;

        Ok(Item::Function {
            name,
            args,
            body,
            ret_type,
            decl_range: decl_start..decl_end,
        })
    }

    fn parse_decl_args(&mut self) -> Result<Vec<(String, SemanticType)>, Error> {
        let mut args = Vec::new();
        while let Some((Token::Ident(name), _)) = self.lexer.current() {
            let name = name.to_owned();

            self.lexer.lex_one()?;
            self.expect_next(
                |t| matches!(t, Token::Colon),
                "expected colon and argument type",
            )?;

            let (type_token, range) = self.expect_take_current()?;
            let Token::Ident(type_str) = type_token else {
                return Err(self
                    .err_ctx
                    .unexpected_token(range, "expected argument type")
                    .finish());
            };

            args.push((name, SemanticType::from(type_str)));

            if !matches!(
                self.lexer.current(),
                Some((Token::Operator(Operator::RightParenthesis), _))
            ) {
                self.expect_next(|t| matches!(t, Token::Comma), "expected comma")?;
            }
        }

        Ok(args)
    }

    fn parse_block(&mut self) -> Result<Vec<Statement>, Error> {
        self.expect_next(
            |t| matches!(t, Token::Operator(Operator::LeftCurlyBracket)),
            "expected opening curly bracket",
        )?;

        let mut statements = Vec::new();
        while let Some((token, _)) = self.lexer.current() {
            if matches!(token, Token::Operator(Operator::RightCurlyBracket)) {
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

        Err(self
            .err_ctx
            .unexpected_eof(self.lexer.cur_token_start())
            .finish())
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
                Some((Token::Operator(Operator::Assign), _)) => {
                    let ExprType::Variable(var) = expr.expr_type else {
                        return Err(self
                            .err_ctx
                            .build(expr.range)
                            .with_message("only variables are allowed in assignments")
                            .finish());
                    };

                    let rvalue = self.parse_expr()?;
                    self.expect_semicolon()?;
                    Ok(Statement::Assign {
                        var,
                        expr: rvalue,
                        var_range: range,
                    })
                }
                Some((Token::Operator(Operator::Declare), _)) => {
                    let ExprType::Variable(var) = expr.expr_type else {
                        return Err(self
                            .err_ctx
                            .build(expr.range)
                            .with_message("only variables are allowed in assignments")
                            .finish());
                    };

                    let rvalue = self.parse_expr()?;
                    self.expect_semicolon()?;
                    Ok(Statement::Declare {
                        var,
                        expr: rvalue,
                        var_range: range,
                    })
                }
                Some((_, range)) => Err(self
                    .err_ctx
                    .unexpected_token(range, "expected ';', '=' or ':='")
                    .finish()),
                None => Err(self
                    .err_ctx
                    .unexpected_eof(self.lexer.cur_token_start())
                    .finish()),
            }
        }
    }

    fn parse_keyword(&mut self, keyword: Keyword, range: Range<usize>) -> Result<Statement, Error> {
        match keyword {
            Keyword::Return => self.parse_return(),
            _ => Err(self
                .err_ctx
                .unexpected_token(range, "unexpected keyword")
                .finish()),
        }
    }

    fn parse_return(&mut self) -> Result<Statement, Error> {
        let expr = self.parse_expr()?;
        self.expect_semicolon()?;

        Ok(Statement::Return(expr))
    }

    fn parse_expr(&mut self) -> Result<Expression, Error> {
        let pos = self.lexer.cur_token_start();
        let token = self.lexer.take_current()?;

        let expr = match token {
            Some((Token::Number(num), range)) => Expression {
                expr_type: ExprType::Const(num),
                range,
            },
            Some((Token::Ident(ident), range)) => self.parse_ident_expr(ident, range)?,
            Some((Token::Character(c), range)) => Expression {
                expr_type: ExprType::Character(c),
                range,
            },
            Some((_, range)) => {
                return Err(self
                    .err_ctx
                    .unexpected_token(range, "invalid expression")
                    .finish());
            }
            None => {
                return Err(self
                    .err_ctx
                    .unexpected_token((pos - 1)..pos, "unexpected end of file")
                    .finish());
            }
        };

        if let Some((Token::Operator(op), _)) = self.lexer.current()
            && matches!(
                op,
                Operator::Plus | Operator::Minus | Operator::Star | Operator::Slash
            )
        {
            // Better way to pattern match and avoid shadowing?
            let op = *op;

            self.lexer.take_current()?;
            let sub_expr = self.parse_expr()?;

            let range = (expr.range.start)..(sub_expr.range.end);

            let expr_type = match op {
                Operator::Plus => ExprType::Addition(Box::new(expr), Box::new(sub_expr)),
                Operator::Minus => ExprType::Subtraction(Box::new(expr), Box::new(sub_expr)),
                Operator::Star => ExprType::Multiplication(Box::new(expr), Box::new(sub_expr)),
                Operator::Slash => ExprType::Division(Box::new(expr), Box::new(sub_expr)),
                _ => unreachable!(),
            };

            return Ok(Expression { expr_type, range });
        }

        Ok(expr)
    }

    fn parse_ident_expr(
        &mut self,
        ident: String,
        range: Range<usize>,
    ) -> Result<Expression, Error> {
        if matches!(
            self.lexer.current(),
            Some((Token::Operator(Operator::LeftParenthesis), _))
        ) {
            self.lexer.take_current()?;

            let args = self.parse_call_args()?;

            self.expect_next(
                |t| matches!(t, Token::Operator(Operator::RightParenthesis)),
                "expected closing parenthesis",
            )?;

            Ok(Expression {
                expr_type: ExprType::FnCall(ident, args),
                range: (range.start)..(self.lexer.last_token_end()),
            })
        } else {
            Ok(Expression {
                expr_type: ExprType::Variable(ident),
                range: (range.start)..(self.lexer.last_token_end()),
            })
        }
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expression>, Error> {
        let mut args = Vec::new();
        while !matches!(
            self.lexer.current(),
            Some((Token::Operator(Operator::RightParenthesis), _))
        ) {
            let expr = self.parse_expr()?;
            args.push(expr);
        }

        Ok(args)
    }

    fn expect_next<F>(&mut self, matches: F, message: impl ToString) -> Result<(), Error>
    where
        F: FnOnce(&Token) -> bool,
    {
        let (token, range) = self.expect_take_current()?;
        if !matches(&token) {
            return Err(self.err_ctx.unexpected_token(range, message).finish());
        }

        Ok(())
    }

    fn expect_semicolon(&mut self) -> Result<(), Error> {
        let current = self.lexer.take_current()?;
        if !matches!(current, Some((Token::Semicolon, _))) {
            let pos = current
                .map(|t| t.1.start)
                .unwrap_or(self.lexer.cur_token_start());
            self.err_ctx
                .build(pos..(pos + 1))
                .with_code(ErrorCode::MissingSemicolon)
                .with_message("expected semicolon")
                .with_label((pos - 1)..pos, "insert the semicolon dummy")
                .report();
        }

        Ok(())
    }

    fn expect_take_current(&mut self) -> Result<(Token, Range<usize>), Error> {
        let token = self.lexer.take_current()?;
        match token {
            Some(token) => Ok(token),
            None => Err(self
                .err_ctx
                .unexpected_token(
                    (self.lexer.cur_token_start() - 1)..self.lexer.cur_token_start(),
                    "unexpected end of file",
                )
                .finish()),
        }
    }
}
