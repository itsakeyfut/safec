//! The value of an `#if` or an `#elif` line (C17 6.10.1).
//!
//! In order: each `defined X` and `defined ( X )` is made `1` or `0`; the
//! line is macro-replaced as an argument is; each name left is `0`; and what
//! remains is read as an integer constant expression in `intmax_t` and
//! `uintmax_t`, which are 64 bits on every target here (`__INTMAX_WIDTH__`,
//! `clang 22.1.8 -dM -E`). A name the implementation may define is refused
//! wherever it is asked about, as `#ifdef` refuses it (ADR-0055, ADR-0059).

use super::held::Sources;
use super::line::Line;
use super::{
    MALFORMED_CONDITION, Reading, UNSUPPORTED_DIRECTIVE, begin_use, replaced, undecidable,
    undecidable_report,
};
use crate::diagnostics::{Diagnostic, DiagnosticSink, Label};
use crate::token::{Punct, Token, TokenKind};
use crate::types::{Reading as Number, read_number};
use safec_ir::ir::{Rank, Sign};
use safec_ir::source::Span;

/// What the line `tokens` of the directive at `directive` (`what`, `#if` or
/// `#elif`) says, or `None` where it was refused, the report made.
pub(super) fn evaluate(
    tokens: Vec<Token>,
    (text, directive, what): (&str, Span, &'static str),
    sources: &mut Sources<'_>,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) -> Option<bool> {
    let refuse = |diagnostics: &mut DiagnosticSink, message: &str, at: Span| {
        diagnostics.report(
            Diagnostic::error(message)
                .with_code(MALFORMED_CONDITION)
                .with_label(Label::primary(at, "here"))
                .with_note(format!(
                    "none of this conditional's groups from this `{what}` on is compiled"
                )),
        );
    };
    if tokens.is_empty() {
        refuse(
            diagnostics,
            &format!("this `{what}` has no expression"),
            directive,
        );
        return None;
    }

    // `defined`, read before anything is replaced (C17 6.10.1 p4). Each
    // answer is a token of a file of its own, `0` or `1`, one per `defined`,
    // so a report at one is placed back at the `defined` it answers: placed
    // in that file, the caret showed `0 1`, text the reader never wrote.
    // Found by review.
    //
    // Whether a `defined` ends up in a function-like macro's argument, where
    // `clang` replaces it first, is asked where the arguments are gathered,
    // by `collected`, from [`Reading::answers`].
    let mut answered: Vec<(usize, bool, Span)> = Vec::new();
    let mut line = Vec::with_capacity(tokens.len());
    let mut at = 0;
    while let Some(&token) = tokens.get(at) {
        if !is_name(token) || sources.snippet(token.span) != "defined" {
            line.push(token);
            at += 1;
            continue;
        }
        let opened = tokens
            .get(at + 1)
            .is_some_and(|next| next.kind == TokenKind::Punct(Punct::LeftParen));
        let named = tokens.get(at + 1 + usize::from(opened)).copied();
        let closed = !opened
            || tokens
                .get(at + 3)
                .is_some_and(|next| next.kind == TokenKind::Punct(Punct::RightParen));
        let Some(name) = named.filter(|named| is_name(*named) && closed) else {
            refuse(diagnostics, "`defined` names no macro", token.span);
            return None;
        };
        let spelled = sources.snippet(name.span);
        if undecidable(spelled, reading) {
            diagnostics.report(undecidable_report(text, directive, what, spelled));
            return None;
        }
        let end = tokens[at + if opened { 3 } else { 1 }].span.end();
        let asked = Span::new(token.span.file(), token.span.start(), end);
        answered.push((line.len(), reading.macros.contains_key(spelled), asked));
        line.push(token);
        at += if opened { 4 } else { 2 };
    }
    let answers = if answered.is_empty() {
        None
    } else {
        let text: Vec<&str> = answered
            .iter()
            .map(|&(_, defined, _)| if defined { "1" } else { "0" })
            .collect();
        match sources.add_virtual(
            "defined",
            &text.join(" "),
            Some((directive, "evaluated here")),
            diagnostics,
        ) {
            Ok(answers) => Some(answers),
            // Reported, once, by the charge.
            Err(_) => return None,
        }
    };
    if let Some(answers) = answers {
        for (index, &(at, _, _)) in answered.iter().enumerate() {
            let start = 2 * index as u32;
            line[at] = Token::new(TokenKind::Number, Span::new(answers, start, start + 1));
        }
    }
    // Where a token is placed in a report: an answer at its `defined`.
    let placed = |span: Span| match answers {
        Some(answers) if span.file() == answers => answered[span.start() as usize / 2].2,
        _ => span,
    };

    // Replaced as an argument is, as a use of its own.
    let errors = diagnostics.error_count();
    begin_use(reading);
    reading.answers = answers.map(|file| {
        let asked = answered.iter().map(|&(_, _, asked)| asked).collect();
        (file, asked)
    });
    let line = replaced(line, false, sources, reading, diagnostics);
    reading.answers = None;
    if diagnostics.error_count() > errors {
        return None;
    }

    let character = reading.target.integer(Rank::Char, Sign::Plain);
    let mut items = Vec::with_capacity(line.len());
    for &(token, _) in &line {
        let spelled = sources.snippet(token.span);
        let token = Token::new(token.kind, placed(token.span));
        let item = match token.kind {
            _ if is_name(token) => {
                // C17 6.10.1 p4 leaves `defined` made by a macro undefined.
                if spelled == "defined" {
                    diagnostics.report(
                        Diagnostic::error("`defined` made by a macro is not read")
                            .with_code(UNSUPPORTED_DIRECTIVE)
                            .with_label(Label::primary(token.span, "made here"))
                            .with_note("C17 6.10.1 p4 leaves what it means undefined"),
                    );
                    return None;
                }
                if undecidable(spelled, reading) {
                    diagnostics.report(undecidable_report(text, directive, what, spelled));
                    return None;
                }
                Item::Value(Value::signed(0), token.span)
            }
            TokenKind::Number => match read_number(spelled) {
                Number::Integer {
                    value, unsigned, ..
                } => match u64::try_from(value) {
                    // Too large for `intmax_t`, a constant is `uintmax_t`'s,
                    // as `clang` reads it.
                    Ok(value) => Item::Value(
                        Value {
                            bits: value,
                            unsigned: unsigned || value > i64::MAX as u64,
                        },
                        token.span,
                    ),
                    Err(_) => {
                        refuse(
                            diagnostics,
                            "this constant is too large for `uintmax_t`",
                            token.span,
                        );
                        return None;
                    }
                },
                Number::TooLarge => {
                    refuse(
                        diagnostics,
                        "this constant is too large for `uintmax_t`",
                        token.span,
                    );
                    return None;
                }
                Number::Floating | Number::Malformed(_) => {
                    refuse(
                        diagnostics,
                        &format!("this `{what}` is not an integer constant expression"),
                        token.span,
                    );
                    return None;
                }
            },
            // A plain character constant of one character, valued as the
            // target's `char` holds it: `'\xff'` is -1 where `char` is
            // signed and 255 where it is not.
            // One never closed, `#if '`, is not a constant at all, and
            // sliced as one it panicked. Found by review.
            TokenKind::Character if spelled.len() < 2 || !spelled.ends_with('\'') => {
                refuse(
                    diagnostics,
                    &format!("this `{what}` is not an integer constant expression"),
                    token.span,
                );
                return None;
            }
            TokenKind::Character if spelled.starts_with('\'') => {
                let values = crate::literal::unescape(&spelled[1..spelled.len() - 1]).ok();
                let Some(&[value]) = values.as_deref() else {
                    diagnostics.report(
                        Diagnostic::error("this character constant is not read in a condition yet")
                            .with_code(UNSUPPORTED_DIRECTIVE)
                            .with_label(Label::primary(token.span, "here"))
                            .with_note("a constant of one character is read"),
                    );
                    return None;
                };
                let value = i128::from(value);
                let value = if value > character.max() {
                    value - (1i128 << character.bits())
                } else {
                    value
                };
                Item::Value(Value::signed(value as i64), token.span)
            }
            TokenKind::Character => {
                diagnostics.report(
                    Diagnostic::error("this character constant is not read in a condition yet")
                        .with_code(UNSUPPORTED_DIRECTIVE)
                        .with_label(Label::primary(token.span, "here"))
                        .with_note("a constant of one character, with no prefix, is read"),
                );
                return None;
            }
            TokenKind::Punct(punct) => Item::Operator(punct, token.span),
            _ => {
                refuse(
                    diagnostics,
                    &format!("this `{what}` is not an integer constant expression"),
                    token.span,
                );
                return None;
            }
        };
        items.push(item);
    }

    let mut parser = Parser {
        items: &items,
        at: 0,
        depth: 0,
        what,
        end: directive,
    };
    let value = parser
        .conditional(true)
        .and_then(|value| match parser.items.get(parser.at) {
            None => Ok(value),
            Some(item) => Err((
                format!("this `{what}` goes on after its expression"),
                item.span(),
            )),
        });
    match value {
        Ok(value) => Some(value.bits != 0),
        Err((message, at)) => {
            refuse(diagnostics, &message, at);
            None
        }
    }
}

/// Whether `token` is a name, which preprocessing makes of a keyword too.
fn is_name(token: Token) -> bool {
    matches!(
        token.kind,
        TokenKind::Identifier | TokenKind::Keyword(_) | TokenKind::Annotation(_)
    )
}

/// The tokens of `text[start..end]`, a directive's line at `directive`
/// with its comments made spaces, placed where they are in its file.
pub(super) fn line(line: &Line, directive: Span, (start, end): (u32, u32)) -> Vec<Token> {
    // Read from the line's own tokens rather than lexed again, so white
    // space and names are what `Line` says they are: lexed again, a no-break
    // space in `#if 1 == 1`, white space to `clang`, was a character no token
    // can begin, and the `#if` was refused. Found by architecture review 13.
    line.tokens_from(start as usize)
        .iter()
        .take_while(|token| token.end <= end as usize)
        .map(|token| {
            Token::new(
                token.kind,
                Span::new(
                    directive.file(),
                    directive.start() + token.start as u32,
                    directive.start() + token.end as u32,
                ),
            )
        })
        .collect()
}

/// A value of `intmax_t` or `uintmax_t`, by its bits.
#[derive(Clone, Copy, Debug)]
struct Value {
    bits: u64,
    unsigned: bool,
}

impl Value {
    fn signed(value: i64) -> Self {
        Self {
            bits: value as u64,
            unsigned: false,
        }
    }

    fn truth(truth: bool) -> Self {
        Self::signed(i64::from(truth))
    }
}

/// What the expression reads: a value, or an operator.
enum Item {
    Value(Value, Span),
    Operator(Punct, Span),
}

impl Item {
    fn span(&self) -> Span {
        match *self {
            Self::Value(_, span) | Self::Operator(_, span) => span,
        }
    }
}

/// Why a condition has no value: what to say and where.
type Refused = (String, Span);

/// A reader of C17 6.5's grammar from `?:` down, over values, each operand
/// evaluated only where it is `live`: `0 && 1 / 0` divides nothing, and a
/// division by zero, a signed overflow or a shift too far is refused only in
/// an operand that is evaluated (6.6 p3, p4).
struct Parser<'a> {
    items: &'a [Item],
    at: usize,
    /// How deep the reading is in parentheses, `?:` and unary operators,
    /// which [`MAX_DEPTH`] bounds.
    depth: usize,
    what: &'static str,
    /// Where a line that ends too soon is reported.
    end: Span,
}

/// How deep a condition may nest before it is refused rather than read on
/// the native stack: a thousand `(` overflowed it with nothing said. Found by
/// review. The parser of C refuses past the same depth.
const MAX_DEPTH: usize = 256;

impl Parser<'_> {
    /// One level deeper, or the refusal past [`MAX_DEPTH`].
    fn deeper(&mut self) -> Result<(), Refused> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err((
                format!("this `{}` nests deeper than {MAX_DEPTH}", self.what),
                self.here(),
            ));
        }
        Ok(())
    }

    /// An expression with its commas (C17 6.5.17), which an integer
    /// constant expression may hold only where it is not evaluated (6.6 p3):
    /// `0 && (1, 2)` is 0, and `(1, 2)` evaluated is refused.
    fn expression(&mut self, live: bool) -> Result<Value, Refused> {
        let mut value = self.conditional(live)?;
        while self.peek() == Some(Punct::Comma) {
            if live {
                return Err(self.not_constant(self.here()));
            }
            self.at += 1;
            value = self.conditional(live)?;
        }
        Ok(value)
    }

    fn not_constant(&self, at: Span) -> Refused {
        (
            format!("this `{}` is not an integer constant expression", self.what),
            at,
        )
    }

    fn peek(&self) -> Option<Punct> {
        match self.items.get(self.at) {
            Some(Item::Operator(punct, _)) => Some(*punct),
            _ => None,
        }
    }

    fn eat(&mut self, punct: Punct) -> bool {
        let found = self.peek() == Some(punct);
        if found {
            self.at += 1;
        }
        found
    }

    fn here(&self) -> Span {
        self.items.get(self.at).map_or(self.end, Item::span)
    }

    fn conditional(&mut self, live: bool) -> Result<Value, Refused> {
        let condition = self.binary(0, live)?;
        if !self.eat(Punct::Question) {
            return Ok(condition);
        }
        self.deeper()?;
        let taken = condition.bits != 0;
        let then = self.expression(live && taken)?;
        if !self.eat(Punct::Colon) {
            return Err(self.not_constant(self.here()));
        }
        let otherwise = self.conditional(live && !taken)?;
        self.depth -= 1;
        // The usual arithmetic conversions bring both to one type
        // (C17 6.5.15 p5).
        let unsigned = then.unsigned || otherwise.unsigned;
        let chosen = if taken { then } else { otherwise };
        Ok(Value {
            bits: chosen.bits,
            unsigned,
        })
    }

    /// The binary operators by precedence, `level` 0 the loosest, `||`.
    fn binary(&mut self, level: usize, live: bool) -> Result<Value, Refused> {
        const LEVELS: &[&[Punct]] = &[
            &[Punct::PipePipe],
            &[Punct::AmpersandAmpersand],
            &[Punct::Pipe],
            &[Punct::Caret],
            &[Punct::Ampersand],
            &[Punct::EqualEqual, Punct::BangEqual],
            &[
                Punct::Less,
                Punct::Greater,
                Punct::LessEqual,
                Punct::GreaterEqual,
            ],
            &[Punct::LessLess, Punct::GreaterGreater],
            &[Punct::Plus, Punct::Minus],
            &[Punct::Star, Punct::Slash, Punct::Percent],
        ];
        let Some(operators) = LEVELS.get(level) else {
            return self.unary(live);
        };
        let mut left = self.binary(level + 1, live)?;
        while let Some(operator) = self.peek().filter(|punct| operators.contains(punct)) {
            let at = self.here();
            self.at += 1;
            let right_live = match operator {
                Punct::AmpersandAmpersand => live && left.bits != 0,
                Punct::PipePipe => live && left.bits == 0,
                _ => live,
            };
            let right = self.binary(level + 1, right_live)?;
            left = self.apply(operator, left, right, (live, right_live), at)?;
        }
        Ok(left)
    }

    fn apply(
        &self,
        operator: Punct,
        left: Value,
        right: Value,
        (live, right_live): (bool, bool),
        at: Span,
    ) -> Result<Value, Refused> {
        let unsigned = left.unsigned || right.unsigned;
        let (l, r) = (left.bits, right.bits);
        let (sl, sr) = (l as i64, r as i64);
        let refuse = |message: &str| Err((message.to_owned(), at));
        let checked = |value: Option<i64>| match value {
            Some(value) => Ok(Value::signed(value)),
            None if live => refuse("this overflows `intmax_t`"),
            None => Ok(Value::signed(0)),
        };
        let compared = |less: bool, equal: bool| Ok(Value::truth(less || equal));
        match operator {
            Punct::PipePipe => Ok(Value::truth(l != 0 || (right_live && r != 0))),
            Punct::AmpersandAmpersand => Ok(Value::truth(l != 0 && r != 0)),
            Punct::Pipe => Ok(Value {
                bits: l | r,
                unsigned,
            }),
            Punct::Caret => Ok(Value {
                bits: l ^ r,
                unsigned,
            }),
            Punct::Ampersand => Ok(Value {
                bits: l & r,
                unsigned,
            }),
            Punct::EqualEqual => Ok(Value::truth(l == r)),
            Punct::BangEqual => Ok(Value::truth(l != r)),
            Punct::Less | Punct::Greater | Punct::LessEqual | Punct::GreaterEqual => {
                let order = if unsigned { l.cmp(&r) } else { sl.cmp(&sr) };
                let (less, equal) = match operator {
                    Punct::Less => (order.is_lt(), false),
                    Punct::Greater => (order.is_gt(), false),
                    Punct::LessEqual => (order.is_lt(), order.is_eq()),
                    _ => (order.is_gt(), order.is_eq()),
                };
                compared(less, equal)
            }
            // The type of a shift is its left operand's (C17 6.5.7 p3), and a
            // count below zero or past the width is undefined.
            Punct::LessLess | Punct::GreaterGreater => {
                // A count below zero is past 63 as `uintmax_t` bits.
                if r >= 64 {
                    return if live {
                        refuse("this shifts by a count below zero or past 63")
                    } else {
                        // Not evaluated, it still has its type, which `?:`
                        // converts the other arm to (C17 6.5.15 p5): made
                        // `intmax_t`, `(1 ? -1 : 1u << 64) < 0` took the
                        // group C does not. Found by review.
                        Ok(Value {
                            bits: 0,
                            unsigned: left.unsigned,
                        })
                    };
                }
                let count = r as u32;
                match (operator, left.unsigned) {
                    (Punct::LessLess, true) => Ok(Value {
                        bits: l << count,
                        unsigned: true,
                    }),
                    (Punct::LessLess, false) => checked(
                        (sl >= 0)
                            .then(|| i128::from(sl) << count)
                            .and_then(|shifted| i64::try_from(shifted).ok()),
                    ),
                    (_, true) => Ok(Value {
                        bits: l >> count,
                        unsigned: true,
                    }),
                    (_, false) => Ok(Value::signed(sl >> count)),
                }
            }
            Punct::Plus | Punct::Minus | Punct::Star if unsigned => Ok(Value {
                bits: match operator {
                    Punct::Plus => l.wrapping_add(r),
                    Punct::Minus => l.wrapping_sub(r),
                    _ => l.wrapping_mul(r),
                },
                unsigned: true,
            }),
            Punct::Plus => checked(sl.checked_add(sr)),
            Punct::Minus => checked(sl.checked_sub(sr)),
            Punct::Star => checked(sl.checked_mul(sr)),
            Punct::Slash | Punct::Percent => {
                if r == 0 {
                    return if live {
                        refuse("this divides by zero")
                    } else {
                        // Its type all the same, as a shift's above.
                        Ok(Value { bits: 0, unsigned })
                    };
                }
                if unsigned {
                    let bits = if operator == Punct::Slash {
                        l / r
                    } else {
                        l % r
                    };
                    return Ok(Value { bits, unsigned });
                }
                checked(if operator == Punct::Slash {
                    sl.checked_div(sr)
                } else {
                    sl.checked_rem(sr)
                })
            }
            _ => Err(self.not_constant(at)),
        }
    }

    fn unary(&mut self, live: bool) -> Result<Value, Refused> {
        let at = self.here();
        match self.items.get(self.at) {
            Some(Item::Value(value, _)) => {
                self.at += 1;
                Ok(*value)
            }
            Some(Item::Operator(Punct::LeftParen, _)) => {
                self.at += 1;
                self.deeper()?;
                let value = self.expression(live)?;
                if !self.eat(Punct::RightParen) {
                    return Err(self.not_constant(self.here()));
                }
                self.depth -= 1;
                Ok(value)
            }
            Some(Item::Operator(
                operator @ (Punct::Plus | Punct::Minus | Punct::Tilde | Punct::Bang),
                _,
            )) => {
                let operator = *operator;
                self.at += 1;
                self.deeper()?;
                let value = self.unary(live)?;
                self.depth -= 1;
                match operator {
                    Punct::Plus => Ok(value),
                    Punct::Tilde => Ok(Value {
                        bits: !value.bits,
                        unsigned: value.unsigned,
                    }),
                    Punct::Bang => Ok(Value::truth(value.bits == 0)),
                    _ if value.unsigned => Ok(Value {
                        bits: value.bits.wrapping_neg(),
                        unsigned: true,
                    }),
                    _ => match (value.bits as i64).checked_neg() {
                        Some(negated) => Ok(Value::signed(negated)),
                        None if live => Err(("this overflows `intmax_t`".to_owned(), at)),
                        None => Ok(Value::signed(0)),
                    },
                }
            }
            _ => Err(self.not_constant(at)),
        }
    }
}
