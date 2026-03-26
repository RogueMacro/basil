use std::{ops::Range, path::PathBuf, rc::Rc};

use crate::analyze::{
    Error, ErrorContext,
    lex::token::{Keyword, Operator, Token},
};

pub mod token;

pub struct Lexer {
    code: Vec<char>,
    index: usize,
    last: Option<(Token, Range<usize>)>,
    current: Option<(Token, Range<usize>)>,
    next: Option<(Token, Range<usize>)>,
    err_ctx: ErrorContext,
    src_path: Rc<PathBuf>,
}

impl Lexer {
    pub fn new(src_path: Rc<PathBuf>, code: impl AsRef<str>) -> Result<Self, Error> {
        let code: Vec<char> = code.as_ref().chars().collect();

        let mut lexer = Self {
            code,
            index: 0,
            last: None,
            current: None,
            next: None,
            err_ctx: ErrorContext::new(),
            src_path,
        };

        lexer.lex_two()?;

        Ok(lexer)
    }

    pub fn cur_token_start(&self) -> usize {
        self.current
            .as_ref()
            .map(|(_, r)| r.start)
            .unwrap_or(self.index)
    }

    pub fn last_token_end(&self) -> usize {
        self.last.as_ref().map(|(_, r)| r.end).unwrap_or(self.index)
    }

    /// Get current token
    pub fn current(&self) -> Option<&(Token, Range<usize>)> {
        self.current.as_ref()
    }

    pub fn take_current(&mut self) -> Result<Option<(Token, Range<usize>)>, Error> {
        self.lex_one()?;
        Ok(self.last.clone())
    }

    /// Lookahead to next token
    pub fn peek(&self) -> Option<&(Token, Range<usize>)> {
        self.next.as_ref()
    }

    /// Move on from current token to the next
    pub fn lex_one(&mut self) -> Result<(), Error> {
        self.last = self.current.take();
        self.current = self.next.take();
        self.next = self.lex_next()?;

        // if let Some(token) = self.next.as_ref() {
        //     println!("[lex] {:?}", token);
        // }

        Ok(())
    }

    /// Move on and skip the next token
    pub fn lex_two(&mut self) -> Result<(), Error> {
        self.lex_one()?;
        self.lex_one()?;
        Ok(())
    }
}

/// Internals
impl Lexer {
    fn peek_char(&self) -> Option<char> {
        self.code.get(self.index + 1).copied()
    }

    fn cur_char(&self) -> Option<char> {
        self.code.get(self.index).copied()
    }

    fn find_next_lexable(&mut self) {
        while let Some(c) = self.cur_char() {
            if c == '/' && self.peek_char() == Some('/') {
                self.lex_comment();
            } else if c.is_whitespace() {
                self.index += 1;
            } else {
                break;
            }
        }
    }

    fn lex_next(&mut self) -> Result<Option<(Token, Range<usize>)>, Error> {
        self.find_next_lexable();

        let Some(c) = self.cur_char() else {
            return Ok(None);
        };

        let token_atom = Token::parse_atom(c, self.peek_char());
        let op = Operator::parse(c, self.peek_char());

        match (token_atom, op) {
            (Some((token, true)), _) => {
                self.index += 2;
                return Ok(Some((token, (self.index - 2)..self.index)));
            }
            (_, Some((op, true))) => {
                self.index += 2;
                return Ok(Some((Token::Operator(op), (self.index - 2)..self.index)));
            }
            (Some((token, false)), _) => {
                self.index += 1;
                return Ok(Some((token, (self.index - 1)..self.index)));
            }
            (_, Some((op, false))) => {
                self.index += 1;
                return Ok(Some((Token::Operator(op), (self.index - 1)..self.index)));
            }
            _ => (),
        }

        if c.is_ascii_alphabetic() || c == '_' {
            return Ok(Some(self.lex_ascii()));
        }

        if c.is_ascii_digit() {
            return Ok(Some(self.lex_number()));
        }

        if c == '\'' {
            self.index += 1;
            let Some(character) = self.cur_char() else {
                return Err(self
                    .err_ctx
                    .unexpected_eof(self.span((self.index - 1)..self.index))
                    .finish());
            };

            self.index += 1;
            if !matches!(self.cur_char(), Some('\'')) {
                return Err(self
                    .err_ctx
                    .unexpected_token(
                        self.span(self.index..(self.index + 1)),
                        format!("expected ' (quote), got '{:?}'", self.cur_char()),
                    )
                    .finish());
            }

            self.index += 1;
            return Ok(Some((
                Token::Character(character),
                (self.index - 3)..self.index,
            )));
        }

        if c == '"' {
            return self.lex_string().map(Some);
        }

        Err(self
            .err_ctx
            .unexpected_token(
                self.span(self.index..(self.index + 1)),
                "unexpected character",
            )
            .finish())
    }

    fn lex_string(&mut self) -> Result<(Token, Range<usize>), Error> {
        assert!(self.cur_char() == Some('\"'));

        let start = self.index;
        self.index += 1;
        let mut string = String::new();
        while let Some(c) = self.cur_char() {
            if c == '"' {
                self.index += 1;
                return Ok((Token::String(string), start..self.index));
            }

            if c == '\\' {
                self.index += 1;
                let Some(next) = self.cur_char() else {
                    return Err(self
                        .err_ctx
                        .unexpected_eof(self.span(start..self.index))
                        .finish());
                };

                let escaped = match next {
                    '\\' => '\\',
                    '"' => '"',
                    '\'' => '\'',
                    'n' => '\n',
                    _ => {
                        let span = self.span((self.index - 1)..self.index);
                        return Err(self
                            .err_ctx
                            .error(span.clone())
                            .with_message("invalid escape character")
                            .with_label(span, "this is not a valid escape character")
                            .finish());
                    }
                };

                string.push(escaped);
            } else if c.is_ascii() {
                string.push(c);
            } else {
                let span = self.span((self.index - 1)..self.index);
                return Err(self
                    .err_ctx
                    .error(span.clone())
                    .with_message("invalid string")
                    .with_label(span, "not a valid character")
                    .finish());
            }

            self.index += 1;
        }

        Err(self
            .err_ctx
            .unexpected_eof(self.span(start..self.index))
            .finish())
    }

    fn lex_ascii(&mut self) -> (Token, Range<usize>) {
        let start = self.index;
        let mut string = String::new();
        while let Some(c) = self.cur_char()
            && (c.is_ascii_alphanumeric() || c == '_')
        {
            string.push(c);
            self.index += 1;
        }

        let token = if let Some(keyword) = Keyword::parse(&string) {
            Token::Keyword(keyword)
        } else if let Ok(b) = string.parse::<bool>() {
            Token::Bool(b)
        } else {
            Token::Ident(string)
        };

        (token, start..self.index)
    }

    fn lex_number(&mut self) -> (Token, Range<usize>) {
        let start = self.index;
        let mut string = String::new();
        while let Some(c) = self.cur_char()
            && c.is_ascii_digit()
        {
            string.push(c);
            self.index += 1;
        }

        let num: i64 = string.parse().unwrap();
        (Token::Number(num), start..self.index)
    }

    fn lex_comment(&mut self) {
        while let Some(c) = self.peek_char() {
            self.index += 1;
            if c == '\n' {
                break;
            }
        }
    }

    fn span(&self, range: Range<usize>) -> (Rc<PathBuf>, Range<usize>) {
        (self.src_path.clone(), range)
    }
}
