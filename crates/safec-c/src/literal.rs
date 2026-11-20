//! What the characters between a literal's quotes stand for: C17 6.4.4.4's
//! escape sequences, read once here for the parser, which refuses what this
//! compiler does not read, and for the type checker, which gives a constant
//! its value.

/// Why a literal's characters could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// Valid C this compiler does not read yet, and what it is.
    Unread(&'static str),
    /// Not C: what C17 does not define.
    Malformed(&'static str),
}

/// The values the characters between the quotes stand for, one per
/// character, escapes read: `\n` is 10, `\101` and `\x41` are 65.
///
/// Read as C17 6.4.4.4 has them: the simple escapes of p1, an octal escape of
/// one to three digits, and a hexadecimal one of as many digits as follow.
/// Refused rather than given a value:
///
/// - a universal character name, `\u` or `\U`, and a character outside ASCII:
///   valid, and read through a member of the execution character set this
///   compiler does not choose yet;
/// - an escape whose value does not fit a `char`, which p9 makes a
///   constraint violation;
/// - any other character after a `\`, which the grammar has no escape for.
pub fn unescape(body: &str) -> Result<Vec<u32>, Unreadable> {
    let mut values = Vec::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if !c.is_ascii() {
            return Err(Unreadable::Unread("a character outside ASCII in a literal"));
        }
        if c != '\\' {
            values.push(u32::from(c));
            continue;
        }
        let Some(escape) = chars.next() else {
            return Err(Unreadable::Malformed("a `\\` with nothing after it"));
        };
        let value = match escape {
            '\'' => 39,
            '"' => 34,
            '?' => 63,
            '\\' => 92,
            'a' => 7,
            'b' => 8,
            'f' => 12,
            'n' => 10,
            'r' => 13,
            't' => 9,
            'v' => 11,
            '0'..='7' => {
                let mut value = escape.to_digit(8).expect("an octal digit");
                for _ in 0..2 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(digit) => {
                            value = value * 8 + digit;
                            chars.next();
                        }
                        None => break,
                    }
                }
                value
            }
            'x' => {
                let mut value: u32 = 0;
                let mut any = false;
                while let Some(digit) = chars.peek().and_then(|c| c.to_digit(16)) {
                    value = value.saturating_mul(16).saturating_add(digit);
                    chars.next();
                    any = true;
                }
                if !any {
                    return Err(Unreadable::Malformed(
                        "a `\\x` with no hexadecimal digit after it",
                    ));
                }
                value
            }
            'u' | 'U' => return Err(Unreadable::Unread("a universal character name")),
            _ => {
                return Err(Unreadable::Malformed(
                    "an escape sequence C does not define",
                ));
            }
        };
        if value > 0xff {
            return Err(Unreadable::Malformed(
                "an escape whose value does not fit a `char`",
            ));
        }
        values.push(value);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every escape of C17 6.4.4.4 p1, written out with the value ASCII gives
    /// it rather than walked from a table this module also reads.
    ///
    /// Mutation: give any one escape another value; this fails.
    #[test]
    fn each_escape_stands_for_its_value() {
        let cases: &[(&str, &[u32])] = &[
            ("a", &[97]),
            ("\\'", &[39]),
            ("\\\"", &[34]),
            ("\\?", &[63]),
            ("\\\\", &[92]),
            ("\\a", &[7]),
            ("\\b", &[8]),
            ("\\f", &[12]),
            ("\\n", &[10]),
            ("\\r", &[13]),
            ("\\t", &[9]),
            ("\\v", &[11]),
            ("\\0", &[0]),
            ("\\101", &[65]),
            ("\\1012", &[65, 50]),
            ("\\x41", &[65]),
            ("\\x041", &[65]),
            ("\\xff", &[255]),
        ];
        for &(body, values) in cases {
            assert_eq!(unescape(body), Ok(values.to_vec()), "{body:?}");
        }
    }

    /// What is refused, and as which.
    ///
    /// Mutation: accept `\q` as `q`; this fails.
    #[test]
    fn what_is_not_read_is_refused_as_what_it_is() {
        assert!(matches!(unescape("\\q"), Err(Unreadable::Malformed(_))));
        assert!(matches!(unescape("\\x"), Err(Unreadable::Malformed(_))));
        assert!(matches!(unescape("\\x100"), Err(Unreadable::Malformed(_))));
        assert!(matches!(unescape("\\400"), Err(Unreadable::Malformed(_))));
        assert!(matches!(unescape("\\u0041"), Err(Unreadable::Unread(_))));
        assert!(matches!(unescape("é"), Err(Unreadable::Unread(_))));
    }
}
