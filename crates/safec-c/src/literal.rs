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

/// An encoding prefix, written before a character constant's or a string
/// literal's opening quote (C17 6.4.4.4 p1, 6.4.5 p1), which the lexer
/// scans into the token. It decides the type of the elements and how a
/// character is encoded into them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prefix {
    /// None: `char`, and the characters this compiler reads are ASCII.
    None,
    /// `u8`, a string literal's only: `char`, encoded as UTF-8 (6.4.5 p6).
    Utf8,
    /// `L`: `wchar_t`, whose width is the target's.
    Wide,
    /// `u`: `char16_t`, encoded as UTF-16.
    Char16,
    /// `U`: `char32_t`, encoded as UTF-32.
    Char32,
}

impl Prefix {
    /// The prefix `text` begins with, and what follows it, which begins
    /// with the opening quote. `u8` is asked before `u`, which begins it.
    pub fn split(text: &str) -> (Self, &str) {
        for (spelling, prefix) in [
            ("u8", Self::Utf8),
            ("L", Self::Wide),
            ("u", Self::Char16),
            ("U", Self::Char32),
        ] {
            if let Some(rest) = text.strip_prefix(spelling) {
                if rest.starts_with(['"', '\'']) {
                    return (prefix, rest);
                }
            }
        }
        (Self::None, text)
    }

    /// How many bits each element has on the widest target there is: what
    /// a reader that does not know the target reads with, so that it
    /// refuses nothing some target holds. Only `L` differs by target, and
    /// the type check reads it again with the target's width.
    pub fn widest_bits(self) -> u32 {
        match self {
            Self::None | Self::Utf8 => 8,
            Self::Char16 => 16,
            Self::Wide | Self::Char32 => 32,
        }
    }
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
    read(body, Prefix::None, 8)
}

/// The code units the characters between the quotes of a literal with
/// `prefix` stand for, each of `bits` bits: [`unescape`]'s reading, with a
/// character outside ASCII encoded as the prefix says, UTF-8, UTF-16 or
/// UTF-32, where [`Prefix::None`] refuses it, and an escape held to what
/// `bits` holds rather than to a `char` (C17 6.4.4.4 p9, "the unsigned
/// type corresponding to `wchar_t`" and the like). `L` takes UTF-16 where
/// `wchar_t` has 16 bits and UTF-32 where it has 32, as `clang` encodes it.
///
/// Mutation: encode every character as one unit; the run of
/// `a_prefixed_literal_is_its_encoding_s_elements` counts one where UTF-16
/// has two.
pub fn read(body: &str, prefix: Prefix, bits: u32) -> Result<Vec<u32>, Unreadable> {
    let limit = (1u64 << bits) - 1;
    let mut values = Vec::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if !c.is_ascii() {
            match (prefix, bits) {
                (Prefix::None, _) => {
                    return Err(Unreadable::Unread("a character outside ASCII in a literal"));
                }
                (_, 8) => {
                    let mut buffer = [0; 4];
                    values.extend(c.encode_utf8(&mut buffer).bytes().map(u32::from));
                }
                (_, 16) => {
                    let mut buffer = [0; 2];
                    values.extend(
                        c.encode_utf16(&mut buffer)
                            .iter()
                            .map(|&unit| u32::from(unit)),
                    );
                }
                _ => values.push(u32::from(c)),
            }
            continue;
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
                let mut value = u64::from(escape.to_digit(8).expect("an octal digit"));
                for _ in 0..2 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(digit) => {
                            value = value * 8 + u64::from(digit);
                            chars.next();
                        }
                        None => break,
                    }
                }
                value
            }
            'x' => {
                // Wider than any unit, so a value past the widest still
                // reads as past it rather than saturating at it.
                let mut value: u64 = 0;
                let mut any = false;
                while let Some(digit) = chars.peek().and_then(|c| c.to_digit(16)) {
                    value = value.saturating_mul(16).saturating_add(u64::from(digit));
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
        if value > limit {
            return Err(Unreadable::Malformed(if bits == 8 {
                "an escape whose value does not fit a `char`"
            } else {
                "an escape whose value does not fit the literal's element type"
            }));
        }
        values.push(u32::try_from(value).expect("held to at most 32 bits above"));
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

    /// A character outside ASCII is its prefix's encoding, written out here
    /// as the code units UTF-8, UTF-16 and UTF-32 give it, and an escape is
    /// held to the element's width, a hexadecimal one too however many
    /// digits it has. Mutations: encode every character as its code point,
    /// and the UTF-8 and UTF-16 rows fail; accumulate a hexadecimal escape
    /// in 32 bits saturating, and `\x100000000` is read as `u32::MAX`.
    #[test]
    fn a_prefixed_literal_is_encoded_and_held_to_its_element() {
        assert_eq!(read("é", Prefix::Utf8, 8), Ok(vec![0xc3, 0xa9]));
        assert_eq!(read("😀", Prefix::Char16, 16), Ok(vec![0xd83d, 0xde00]));
        assert_eq!(read("😀", Prefix::Wide, 16), Ok(vec![0xd83d, 0xde00]));
        assert_eq!(read("😀", Prefix::Wide, 32), Ok(vec![0x1f600]));
        assert_eq!(read("😀", Prefix::Char32, 32), Ok(vec![0x1f600]));
        assert_eq!(read("\\xffff", Prefix::Char16, 16), Ok(vec![0xffff]));
        assert!(matches!(
            read("\\x10000", Prefix::Char16, 16),
            Err(Unreadable::Malformed(_))
        ));
        assert_eq!(
            read("\\xffffffff", Prefix::Char32, 32),
            Ok(vec![0xffff_ffff])
        );
        assert!(matches!(
            read("\\x100000000", Prefix::Char32, 32),
            Err(Unreadable::Malformed(_))
        ));
        assert!(matches!(
            read("\\u00e9", Prefix::Char32, 32),
            Err(Unreadable::Unread(_))
        ));
    }

    /// The prefix is read off the front, `u8` before the `u` it begins with,
    /// and a word that only begins like one is no prefix.
    #[test]
    fn a_prefix_is_split_off_its_quote() {
        assert_eq!(Prefix::split("u8\"a\""), (Prefix::Utf8, "\"a\""));
        assert_eq!(Prefix::split("u'a'"), (Prefix::Char16, "'a'"));
        assert_eq!(Prefix::split("U\"a\""), (Prefix::Char32, "\"a\""));
        assert_eq!(Prefix::split("L'a'"), (Prefix::Wide, "'a'"));
        assert_eq!(Prefix::split("\"a\""), (Prefix::None, "\"a\""));
    }
}
