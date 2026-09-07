//! What the lexer produces.
//!
//! A token is a kind and a position, and nothing else. The text it covers is
//! recovered from the source map when something needs it, which is the same
//! arrangement [`crate::source`] already uses for position: one authority, and
//! everything else derived from it on demand. See ADR-0006.
//!
//! That is a decision about where work happens as much as about size. A
//! `TokenKind::Number` names a span and stops there, so the lexer settles where
//! a numeric constant ends and nothing more. What value it has, and what type,
//! follows from its base, its suffix, and the first type it fits in, which are
//! questions for a stage that knows the target's type sizes. A token carrying a
//! value would force the scanner to answer them.

use std::mem;

use crate::source::Span;

/// One token: what it is, and where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token {
    /// What kind of token this is.
    pub kind: TokenKind,
    /// The text is covers.
    pub span: Span,
}

impl Token {
    /// A token of `kind` covering `span`.
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Whether this is the end of the token stream.
    pub fn is_eof(self) -> bool {
        self.kind == TokenKind::Eof
    }
}

/// What a token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// A word reserved by the language.
    Keyword(Keyword),
    /// A word that is not.
    Identifier,
    /// A numeric constant, as C delimits one.
    ///
    /// This is a *preprocessing number*: a digit, or a `.` and a digit,
    /// followed by digits, identifier characters, `.`, and a sign after `e`,
    /// `E`, `p` or `P`. It is deliberately wider than the set of valid
    /// constants, so `123abc` is one token rather than two, and is rejected
    /// later with something to point at rather than here with a caret in the
    /// middle of it.
    Number,
    /// A string literal, quotes included.
    String,
    /// A character constant, quptes included.
    Character,
    /// An operator or a separator
    Punct(Punct),
    /// A run of characters that can begin no token at all.
    ///
    /// Reported where it is found, and kept, so that the stream still accounts
    /// for the text is came from.
    Unknown,
    /// The end of the file.
    ///
    /// A real token with an empty span at the end of the text, so that a parser
    /// asking for the next token always gets one. Running out of input is then
    /// an unexpected token like any other, rather than a second error path
    /// every caller has to remember.
    Eof,
}

/// Turn one list of spellings into the enum, the roster and the spelling.
///
/// Three things that must agree, generated from one place so that they cannot
/// disagree: a variant missing from the roster or from `as_str` is impossible
/// rather than merely tested for.
macro_rules! spellings {
    (
        $(#[$enum_meta:meta])*
        $name:ident { $($variant:ident => $spelling:literal,)+ }
    ) => {
        $(#[$enum_meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $(
                #[doc = concat!("`", $spelling, "`")]
                $variant,
            )+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            /// How this is spelled in a C source file.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling,)+
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

spellings! {
    /// A word C reserves.
    ///
    /// The C17 set. C23 makes keywords of `bool`, `true`, `false`, `nullptr`,
    /// `constexpr`, `typeof` and others that C17 leaves as ordinary
    /// identifiers, so the two cannot both be recognised without knowing which
    /// language version is being compiled. There is no `-std` yet; when there
    /// is, that is what chooses between them, and until then the older set is
    /// the one that misreads no valid program.
    Keyword {
        Auto => "auto",
        Break => "break",
        Case => "case",
        Char => "char",
        Const => "const",
        Continue => "continue",
        Default => "default",
        Do => "do",
        Double => "double",
        Else => "else",
        Enum => "enum",
        Extern => "extern",
        Float => "float",
        For => "for",
        Goto => "goto",
        If => "if",
        Inline => "inline",
        Int => "int",
        Long => "long",
        Register => "register",
        Restrict => "restrict",
        Return => "return",
        Short => "short",
        Signed => "signed",
        Sizeof => "sizeof",
        Static => "static",
        Struct => "struct",
        Switch => "switch",
        Typedef => "typedef",
        Union => "union",
        Unsigned => "unsigned",
        Void => "void",
        Volatile => "volatile",
        While => "while",
        Alignas => "_Alignas",
        Alignof => "_Alignof",
        Atomic => "_Atomic",
        Bool => "_Bool",
        Complex => "_Complex",
        Generic => "_Generic",
        Imaginary => "_Imaginary",
        Noreturn => "_Noreturn",
        StaticAssert => "_Static_assert",
        ThreadLocal => "_Thread_local",
    }
}

impl Keyword {
    /// The keyword spelled `word`, if any.
    ///
    /// A linear scan. It runs once per identifier, over a list of forty-odd
    /// short strings that almost always differ in their first character, and
    /// replacing it is a change to make against a measurement rather than
    /// against an intuition.
    pub fn from_spelling(word: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kw| kw.as_str() == word)
    }
}

spellings! {
    /// An operator or a separator.
    ///
    /// Declared longest first, which is what makes [`Punct::starting`] a
    /// maximal munch: the first spelling that matches is the longest one that
    /// could. Reordering this list makes `>>=` scan as `>>` and then `=`.
    ///
    /// The digraphs (`<:`, `%>`, `%:%:` and the rest) are left out. They are
    /// still in the standard and nothing writes them; adding them is a table
    /// entry each, whenever something does.
    Punct {
        Ellipsis => "...",
        LessLessEqual => "<<=",
        GreaterGreaterEqual => ">>=",

        Arrow => "->",
        PlusPlus => "++",
        MinusMinus => "--",
        LessLess => "<<",
        GreaterGreater => ">>",
        LessEqual => "<=",
        GreaterEqual => ">=",
        EqualEqual => "==",
        BangEqual => "!=",
        AmpersandAmpersand => "&&",
        PipePipe => "||",
        StarEqual => "*=",
        SlashEqual => "/=",
        PercentEqual => "%=",
        PlusEqual => "+=",
        MinusEqual => "-=",
        AmpersandEqual => "&=",
        CaretEqual => "^=",
        PipeEqual => "|=",
        HashHash => "##",

        LeftBracket => "[",
        RightBracket => "]",
        LeftParen => "(",
        RightParen => ")",
        LeftBrace => "{",
        RightBrace => "}",
        Dot => ".",
        Ampersand => "&",
        Star => "*",
        Plus => "+",
        Minus => "-",
        Tilde => "~",
        Bang => "!",
        Slash => "/",
        Percent => "%",
        Less => "<",
        Greater => ">",
        Caret => "^",
        Pipe => "|",
        Question => "?",
        Colon => ":",
        Semicolon => ";",
        Equal => "=",
        Comma => ",",
        Hash => "#",
    }
}

impl Punct {
    /// The longest punctuator `text` begins with, if any.
    ///
    /// C's rule: a scanner takes the longest sequence of characters that could
    /// make up a token, even when a shorter one would let the rest parse. `a+++b`
    /// is `a` `++` `+` `b` and does not compile, rather than `a` `+` `++` `b`
    /// and doing so.
    pub fn starting(text: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|punct| text.starts_with(punct.as_str()))
    }

    /// Whether a punctuator can begin with `c`.
    ///
    /// What the lexer asks to decide where a run of unrecognised characters
    /// ends.
    pub fn can_start_with(c: char) -> bool {
        Self::ALL.iter().any(|punct| punct.as_str().starts_with(c))
    }
}

/// A token is a kind and a position.
///
/// The guard for ADR-0006, and the reason it is written as a size: the decision
/// is that a token carries no text, and a `String` field is what breaking it
/// would look like. Adding one takes this to 40.
const _: () = assert!(mem::size_of::<Token>() == 16);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::FileId;

    fn span(start: u32, end: u32) -> Span {
        Span::new(FileId::from_index(0), start, end)
    }

    #[test]
    fn a_token_is_a_kind_and_a_position() {
        let token = Token::new(TokenKind::Identifier, span(4, 8));

        assert_eq!(token.kind, TokenKind::Identifier);
        assert_eq!(token.span, span(4, 8));
        assert!(!token.is_eof());
        assert!(Token::new(TokenKind::Eof, span(8, 8)).is_eof());
    }

    /// Every spelling in the table is the one C uses, and no two variants share
    /// one. A duplicate would make `from_spelling` return whichever came first
    /// and leave the other unreachable.
    #[test]
    fn every_keyword_has_its_own_spelling() {
        let mut spellings: Vec<_> = Keyword::ALL.iter().map(|kw| kw.as_str()).collect();
        let count = spellings.len();
        spellings.sort_unstable();
        spellings.dedup();

        assert_eq!(spellings.len(), count, "two keywords share a spelling");
        assert_eq!(count, 44, "the C17 keyword set has 44 members");
    }

    #[test]
    fn every_keyword_is_found_by_its_spelling() {
        for &keyword in Keyword::ALL {
            assert_eq!(
                Keyword::from_spelling(keyword.as_str()),
                Some(keyword),
                "{keyword} did not round trip"
            );
        }
    }

    /// The C23 additions are ordinary identifiers here, which is what C17 says
    /// and what `-std` will later choose between.
    #[test]
    fn a_word_that_is_not_a_c17_keyword_is_not_one() {
        for word in ["owner", "borrow", "bool", "true", "nullptr", "constexpr"] {
            assert_eq!(Keyword::from_spelling(word), None, "{word}");
        }
    }

    #[test]
    fn every_punctuator_has_its_own_spelling() {
        let mut spellings: Vec<_> = Punct::ALL.iter().map(|p| p.as_str()).collect();
        let count = spellings.len();
        spellings.sort_unstable();
        spellings.dedup();

        assert_eq!(spellings.len(), count, "two punctuators share a spelling");
    }

    /// The ordering is the maximal munch. Sorting this table any other way
    /// makes the scan return a prefix of the punctuator that is there.
    #[test]
    fn punctuators_are_declared_longest_first() {
        let lengths: Vec<_> = Punct::ALL.iter().map(|p| p.as_str().len()).collect();

        assert!(
            lengths.windows(2).all(|pair| pair[0] >= pair[1]),
            "{:?}",
            Punct::ALL
        );
    }

    #[test]
    fn a_scan_takes_the_longest_punctuator_that_fits() {
        assert_eq!(Punct::starting(">>=x"), Some(Punct::GreaterGreaterEqual));
        assert_eq!(Punct::starting(">>x"), Some(Punct::GreaterGreater));
        assert_eq!(Punct::starting(">x"), Some(Punct::Greater));
        assert_eq!(Punct::starting("...)"), Some(Punct::Ellipsis));
        assert_eq!(Punct::starting("..)"), Some(Punct::Dot));
    }

    /// `a+++b` is `a` `++` `+` `b`, which does not compile, and not
    /// `a` `+` `++` `b`, which would. The rule is about the scan, not about
    /// what parses.
    #[test]
    fn the_longest_scan_wins_even_when_a_shorter_one_would_parse() {
        assert_eq!(Punct::starting("+++b"), Some(Punct::PlusPlus));
    }

    #[test]
    fn a_character_that_begins_no_punctuator_is_not_one() {
        for c in ['<', '>', '#', '.', '='] {
            assert!(Punct::can_start_with(c), "{c}");
        }
        for c in ['a', '0', '"', '\'', '@', '$', '`', '\\', '日'] {
            assert!(!Punct::can_start_with(c), "{c}");
        }
    }

    #[test]
    fn a_punctuator_and_a_keyword_say_how_they_are_spelled() {
        assert_eq!(Keyword::Int.to_string(), "int");
        assert_eq!(Keyword::StaticAssert.to_string(), "_Static_assert");
        assert_eq!(Punct::Semicolon.to_string(), ";");
        assert_eq!(Punct::GreaterGreaterEqual.to_string(), ">>=");
    }
}
