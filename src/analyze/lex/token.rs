#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Keyword(Keyword),

    Number(i64),
    Character(char),
    String(String),
    Bool(bool),

    Ident(String),

    Semicolon,
    Colon,
    Comma,
    LeftParenthesis,
    RightParenthesis,
    LeftCurlyBracket,
    RightCurlyBracket,

    Reference,

    Declare,
    Assign,
    Arrow,
    PathSeparator,

    Operator(Operator),
}

impl Token {
    pub fn parse_atom(current: char, lookahead: Option<char>) -> Option<(Self, bool)> {
        let token = match (current, lookahead) {
            (':', Some('=')) => (Self::Declare, true),
            (':', Some(':')) => (Self::PathSeparator, true),
            ('-', Some('>')) => (Self::Arrow, true),

            ('=', _) => (Self::Assign, false),
            (';', _) => (Self::Semicolon, false),
            (':', _) => (Self::Colon, false),
            (',', _) => (Self::Comma, false),
            ('(', _) => (Self::LeftParenthesis, false),
            (')', _) => (Self::RightParenthesis, false),
            ('{', _) => (Self::LeftCurlyBracket, false),
            ('}', _) => (Self::RightCurlyBracket, false),

            ('&', _) => (Self::Reference, false),

            _ => return None,
        };

        Some(token)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Operator {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,

    Plus,
    Minus,
    Star,
    Slash,
}

impl Operator {
    pub fn parse(current: char, lookahead: Option<char>) -> Option<(Self, bool)> {
        let op = match (current, lookahead) {
            ('=', Some('=')) => (Self::Equal, true),
            ('!', Some('=')) => (Self::NotEqual, true),
            ('<', Some('=')) => (Self::LessOrEqual, true),
            ('>', Some('=')) => (Self::GreaterOrEqual, true),
            ('<', _) => (Self::Less, false),
            ('>', _) => (Self::Greater, false),

            ('+', _) => (Self::Plus, false),
            ('-', _) => (Self::Minus, false),
            ('*', _) => (Self::Star, false),
            ('/', _) => (Self::Slash, false),

            _ => return None,
        };

        Some(op)
    }

    pub fn precedence(&self) -> i32 {
        use Operator::*;

        match self {
            Equal | NotEqual | Less | LessOrEqual | Greater | GreaterOrEqual => 0,
            Plus | Minus => 1,
            Star | Slash => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Keyword {
    Function,
    Return,
    If,
    Use,
    Extern,
    As,
    While,
    For,
    In,
}

impl Keyword {
    pub fn parse(value: impl AsRef<str>) -> Option<Self> {
        let keyword = match value.as_ref() {
            "fn" => Keyword::Function,
            "return" => Keyword::Return,
            "if" => Keyword::If,
            "use" => Keyword::Use,
            "extern" => Keyword::Extern,
            "as" => Keyword::As,
            "while" => Keyword::While,
            "for" => Keyword::For,
            "in" => Keyword::In,
            _ => return None,
        };

        Some(keyword)
    }
}
