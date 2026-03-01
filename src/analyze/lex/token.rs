#[derive(Debug, Clone)]
pub enum Token {
    Keyword(Keyword),

    Number(i64),
    Character(char),
    Bool(bool),

    Ident(String),

    Semicolon,
    Colon,
    Comma,
    Operator(Operator),
}

#[derive(Debug, Clone, Copy)]
pub enum Operator {
    Declare,
    Equality,
    Arrow,

    LeftParenthesis,
    RightParenthesis,
    LeftCurlyBracket,
    RightCurlyBracket,
    Assign,
    Plus,
    Minus,
    Star,
    Slash,
}

impl Operator {
    pub fn parse(current: char, lookahead: Option<char>) -> Option<(Self, bool)> {
        let op = match (current, lookahead) {
            (':', Some('=')) => (Self::Declare, true),
            ('=', Some('=')) => (Self::Equality, true),
            ('-', Some('>')) => (Self::Arrow, true),

            ('(', _) => (Self::LeftParenthesis, false),
            (')', _) => (Self::RightParenthesis, false),
            ('{', _) => (Self::LeftCurlyBracket, false),
            ('}', _) => (Self::RightCurlyBracket, false),
            ('=', _) => (Self::Assign, false),
            ('+', _) => (Self::Plus, false),
            ('-', _) => (Self::Minus, false),
            ('*', _) => (Self::Star, false),
            ('/', _) => (Self::Slash, false),

            _ => return None,
        };

        Some(op)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Keyword {
    Function,
    Return,
}

impl Keyword {
    pub fn parse(value: impl AsRef<str>) -> Option<Self> {
        let keyword = match value.as_ref() {
            "fn" => Keyword::Function,
            "return" => Keyword::Return,
            _ => return None,
        };

        Some(keyword)
    }
}
