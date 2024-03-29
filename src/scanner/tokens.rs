// enable pretty-printing if needed
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum TokenType {
    // Token types (names)
    // When we say `an Empty token` we mean a Token object
    // whose `name` field is set to `TokenName::Empty`

    // ANCHORS
    StartAnchor,     // \A
    EndAnchor,       // \Z
    WordBoundary,    // \b
    NonWordBoundary, // \B

    // SPECIAL
    // indicator of places like:
    // "" (an empty string)
    // "|..." before the leading |
    // "...|" after the trailing |
    // "...||..." between | and |
    // "...(|...)..." between ( and |
    // "...(...|)..." between | and )
    // "...()..." between ( and )
    Empty,
    Character { value: char },

    // METACHARACTERS
    LeftParen,  // (
    RightParen, // )
    Pipe,       // |, alternation operator (E1|E2|...|E_n)
    Mark,       // ?, match zero or one occurrence of previous expression
    Star,       // *, match zero or more occurrences of previous expression
    Plus,       // +, match zero or more occurrences of previous expression
    Dot,        // ., match any single character even newline `\n`
}

// Scanner generates `Tokens` which are a atoms of regular expressions
// Token is identified by two properties:
// name    : a variant of TokenName
// position: usize integer indicating where this Token begins inside source string given to the
// scanner
// The scanner just splits the pattern string for the parser

// enable pretty-printing if needed
#[derive(Debug, Clone, Copy)]
pub struct Token {
    // What kind this token is?
    pub type_name: TokenType,
    // index in source string
    pub position: usize,
}
