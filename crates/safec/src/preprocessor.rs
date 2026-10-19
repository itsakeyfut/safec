//! Directives, read between the lexer and the parser.
//!
//! What it reads is `#include "file"`: the tokens of the file it names, itself
//! preprocessed, take the directive's place. Every other directive is refused
//! as `SC0104`, `#include <file>` among them, which needs a search path and a
//! system header this compiler cannot read yet anyway.
//!
//! Each inclusion is a file of its own in the source map, so a diagnostic in a
//! header names the header and two inclusions of one header never share a
//! span. See ADR-0054. The translation unit is preprocessed whole before it is
//! parsed, for the reason that record gives.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::driver::{Access, reason};
use crate::lexer::lex;
use crate::token::{Token, TokenKind};
use safec_ir::source::{FileId, FileName, SourceMap, Span};

// `SC01xx` with the lexer's, because the topic is the same: what a line of the
// source is before the grammar reads it. `SC0104` was the lexer's until the
// preprocessor took the line over, and kept its number when it moved.
const UNSUPPORTED_DIRECTIVE: Code = Code::new("SC0104");
/// A file an `#include` names that cannot be read.
const UNREADABLE_INCLUSION: Code = Code::new("SC0109");
/// An `#include` of a file that is already being read.
const RECURSIVE_INCLUSION: Code = Code::new("SC0110");

/// Splice every `#include "file"` in `tokens`, the scan of `file`, and report
/// every directive that is not one.
///
/// The end-of-file token is the input's own: an included file's is dropped,
/// since the parser reads the first one it meets as the end.
pub fn preprocess(
    file: FileId,
    tokens: &[Token],
    sources: &mut SourceMap,
    diagnostics: &mut DiagnosticSink,
) -> Vec<Token> {
    let mut reading = Vec::new();
    if let Some(path) = path_of(sources.file(file).name()) {
        reading.push((canonical(path), file));
    }

    let mut spliced = Vec::with_capacity(tokens.len());
    splice(tokens, sources, &mut reading, diagnostics, &mut spliced);
    spliced.extend(tokens.iter().copied().filter(|token| token.is_eof()));
    spliced
}

/// Append `tokens` to `spliced`, an `#include "file"` replaced by that file's,
/// and no end of file.
///
/// `reading` is the files being read, outermost first, each by its canonical
/// path, so a cycle is found whatever spelling reached it. Recursion ends
/// because a file on it is never read again, and a chain of different files is
/// as deep as the files on disk make it.
fn splice(
    tokens: &[Token],
    sources: &mut SourceMap,
    reading: &mut Vec<(PathBuf, FileId)>,
    diagnostics: &mut DiagnosticSink,
    spliced: &mut Vec<Token>,
) {
    for &token in tokens {
        match token.kind {
            TokenKind::Eof => {}
            TokenKind::Directive => {
                // Owned, because reading the file it names adds to the map.
                let text = sources.snippet(token.span).to_owned();
                match form(&text) {
                    Form::Quoted { name, at } => {
                        let quoted = Span::new(
                            token.span.file(),
                            token.span.start() + at,
                            token.span.start() + at + name.len() as u32 + 2,
                        );
                        include(
                            token.span,
                            quoted,
                            name,
                            sources,
                            reading,
                            diagnostics,
                            spliced,
                        );
                    }
                    Form::Angled => diagnostics.report(
                        refused(&text, token.span, "`#include <...>` is not read yet")
                            .with_note("only `#include \"...\"` is read yet"),
                    ),
                    Form::Other => diagnostics.report(
                        refused(
                            &text,
                            token.span,
                            "preprocessor directives are not supported yet",
                        )
                        // Which directives, rather than which stage.
                        // `docs/frontend.md` puts the preprocessor in its
                        // Stage 3 and `docs/roadmap.md` pulls four of them
                        // forward, so a stage number is both a number the
                        // reader does not have and one that stopped being
                        // the whole answer.
                        .with_note(
                            "`#include \"...\"` is read, and `#define`, `#ifdef` and \
                                 `#ifndef` come next",
                        )
                        .with_note("the line is one token and is not compiled"),
                    ),
                }
            }
            _ => spliced.push(token),
        }
    }
}

/// Read the file `name` names, relative to the file the directive is in, and
/// splice its tokens.
///
/// `directive` is the whole line and `quoted` the name with its quotes, which
/// is where a report about the file is put.
fn include(
    directive: Span,
    quoted: Span,
    name: &str,
    sources: &mut SourceMap,
    reading: &mut Vec<(PathBuf, FileId)>,
    diagnostics: &mut DiagnosticSink,
    spliced: &mut Vec<Token>,
) {
    let unreadable = |note: String| {
        Diagnostic::error("cannot read the included file")
            .with_code(UNREADABLE_INCLUSION)
            .with_label(Label::primary(quoted, "included here"))
            .with_note(note)
    };

    let Some(includer) = path_of(sources.file(directive.file()).name()) else {
        diagnostics.report(unreadable(format!(
            "`{name}` is found from the directory of the file that includes it, and this \
             file has no directory"
        )));
        return;
    };
    let path = resolved(includer, name);

    let canonical = canonical(&path);
    if let Some(&(_, open)) = reading.iter().find(|(being, _)| *being == canonical) {
        let refused = Diagnostic::error("this file includes itself")
            .with_code(RECURSIVE_INCLUSION)
            .with_label(Label::primary(quoted, "already being read"));
        // Where the open one began to be read: the directive that included
        // it, or nowhere, if it is the input. Asked of the file rather than
        // of its place on the stack, which says the same only while the input
        // is the first thing pushed.
        let refused = match sources.file(open).name() {
            FileName::Inclusion { at, .. } => refused
                .with_label(Label::secondary(*at, "where it began to be read"))
                .with_note(format!(
                    "`{}` has not finished, so reading it here would never end",
                    path.display()
                )),
            FileName::Real(_) | FileName::Virtual(_) => refused.with_note(format!(
                "`{}` is the input, so reading it here would never end",
                path.display()
            )),
        };
        diagnostics.report(refused);
        return;
    }

    let file = match sources.include(&path, directive) {
        Ok(file) => file,
        Err(error) => {
            // The name as written, which is what the reader can find in the
            // directive above. Where it was looked for follows it.
            let why = if error.kind() == std::io::ErrorKind::InvalidData {
                "it is not valid UTF-8, and safec reads source files as UTF-8".to_owned()
            } else {
                reason(&path, &error, Access::Read)
            };
            diagnostics.report(unreadable(format!(
                "`{name}` was looked for at `{}`: {why}",
                path.display()
            )));
            return;
        }
    };

    let tokens = lex(file, &sources.file_owned(file), diagnostics);
    reading.push((canonical, file));
    splice(&tokens, sources, reading, diagnostics, spliced);
    reading.pop();
}

/// What an `#include` is spelled as, which decides what is done with it.
#[derive(Debug, PartialEq, Eq)]
enum Form<'a> {
    /// `#include "name"`, with `at` the offset of the opening quote in the
    /// directive's text.
    Quoted { name: &'a str, at: u32 },
    /// `#include <name>`.
    Angled,
    /// Any other directive, and an `#include` that is neither of those, such
    /// as one naming a macro.
    Other,
}

/// Read a directive's text, from its `#` to the end of its line.
///
/// Whitespace may stand between the `#`, the name and the file (C17 6.10 p5),
/// and a comment may follow the file. A `\r` is whitespace here, because the
/// line of a file saved with CRLF ends in one, and a header that did not read
/// on Windows would be a header that did not read for most of the people who
/// write one.
fn form(text: &str) -> Form<'_> {
    let blank = [' ', '\t', '\u{b}', '\u{c}', '\r'];

    let Some(rest) = text.strip_prefix('#').or_else(|| text.strip_prefix("%:")) else {
        return Form::Other;
    };
    let Some(rest) = rest.trim_start_matches(blank).strip_prefix("include") else {
        return Form::Other;
    };
    // `#included` is a directive of another name.
    if rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
        return Form::Other;
    }
    let rest = rest.trim_start_matches(blank);
    if rest.starts_with('<') {
        return Form::Angled;
    }
    let Some(quoted) = rest.strip_prefix('"') else {
        return Form::Other;
    };
    let Some((name, mut tail)) = quoted.split_once('"') else {
        return Form::Other;
    };

    loop {
        tail = tail.trim_start_matches(blank);
        if tail.is_empty() || tail.starts_with("//") {
            break;
        }
        match tail
            .strip_prefix("/*")
            .and_then(|comment| comment.split_once("*/"))
        {
            Some((_, after)) => tail = after,
            None => return Form::Other,
        }
    }

    Form::Quoted {
        name,
        at: (text.len() - rest.len()) as u32,
    }
}

/// The report for a directive this does not read, at its `#`.
///
/// The `#`, not the line. A caret under a long macro definition is a wall, and
/// the line is quoted above it anyway.
fn refused(text: &str, directive: Span, message: &str) -> Diagnostic {
    // Two bytes where the `#` is written `%:`.
    let hash = if text.starts_with("%:") { 2 } else { 1 };
    Diagnostic::error(message)
        .with_code(UNSUPPORTED_DIRECTIVE)
        .with_label(Label::primary(
            Span::new(
                directive.file(),
                directive.start(),
                directive.start() + hash,
            ),
            "directive begins here",
        ))
}

/// The path a file was read from, if it was read from one.
fn path_of(name: &FileName) -> Option<&Path> {
    match name {
        FileName::Real(path) | FileName::Inclusion { path, .. } => Some(path),
        FileName::Virtual(_) => None,
    }
}

/// `name` found from the directory `includer` is in.
///
/// Joined with a `/` rather than by [`Path::join`], which writes `\` on
/// Windows: the path reaches a diagnostic, and a diagnostic says the same thing
/// on every host. Windows reads a `/` as a separator, so nothing it opens
/// changes.
fn resolved(includer: &Path, name: &str) -> PathBuf {
    let directory = includer.parent().unwrap_or(Path::new(""));
    if Path::new(name).has_root() || directory.as_os_str().is_empty() {
        return PathBuf::from(name);
    }
    let mut path = OsString::from(directory.as_os_str());
    path.push("/");
    path.push(name);
    PathBuf::from(path)
}

/// What a cycle is found by: the file itself, whatever path reached it.
///
/// A file that cannot be canonicalized cannot be read either, so its own
/// spelling is enough to stand for it until the read fails.
fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every spelling C allows of the one form read, and the ones beside it
    /// that are not it.
    #[test]
    fn an_include_is_read_however_it_is_spaced() {
        for (text, name) in [
            ("#include \"a.h\"", "a.h"),
            ("#  include\t\"a.h\"", "a.h"),
            ("#include\"a.h\"", "a.h"),
            ("%:include \"a.h\"", "a.h"),
            ("#include \"a.h\"\r", "a.h"),
            ("#include \"a.h\" // why", "a.h"),
            ("#include \"a.h\" /* why */ ", "a.h"),
            ("#include \"dir/a b.h\"", "dir/a b.h"),
        ] {
            let Form::Quoted { name: read, at } = form(text) else {
                panic!("{text:?} is not read as an include: {:?}", form(text));
            };
            assert_eq!(read, name, "{text:?}");
            assert_eq!(&text[at as usize..at as usize + 1], "\"", "{text:?}");
        }
    }

    #[test]
    fn what_is_not_a_quoted_include_is_not_read_as_one() {
        for (text, expected) in [
            ("#include <stdio.h>", Form::Angled),
            ("#  include<stdio.h>", Form::Angled),
            ("#define X 1", Form::Other),
            ("#included \"a.h\"", Form::Other),
            ("#include HEADER", Form::Other),
            ("#include \"a.h", Form::Other),
            ("#include \"a.h\" x", Form::Other),
            ("#include \"a.h\" /* open", Form::Other),
            ("#", Form::Other),
        ] {
            assert_eq!(form(text), expected, "{text:?}");
        }
    }

    /// The directory is joined with `/` on every host, so a diagnostic naming
    /// the path is the same on Windows as elsewhere.
    ///
    /// Mutation: resolve with `Path::join`. Fails on Windows, which is where
    /// the separator differs.
    #[test]
    fn an_include_is_found_beside_the_file_that_includes_it() {
        assert_eq!(resolved(Path::new("a.c"), "a.h"), PathBuf::from("a.h"));
        assert_eq!(resolved(Path::new("src/a.c"), "a.h").as_os_str(), "src/a.h");
        assert_eq!(
            resolved(Path::new("src/a.c"), "/usr/a.h"),
            PathBuf::from("/usr/a.h")
        );
    }

    /// A file with no path has no directory to find a name from, and is not
    /// taken to be in the one the compiler happens to run in.
    ///
    /// Mutation: resolve against an empty directory for a file with no path.
    /// The note says the name was looked for, at a path nobody wrote.
    #[test]
    fn an_include_in_a_file_with_no_path_is_refused() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "#include \"a.h\"\n");
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        preprocess(file, &tokens, &mut sources, &mut diagnostics);

        let [refused] = diagnostics.diagnostics() else {
            panic!("{:?}", diagnostics.diagnostics());
        };
        assert_eq!(
            refused.code().map(|code| code.to_string()).as_deref(),
            Some("SC0109")
        );
        assert_eq!(
            refused.notes(),
            [
                "`a.h` is found from the directory of the file that includes it, and this file \
              has no directory"
            ]
        );
    }

    /// The code moved from the lexer with the report, and is the handle a
    /// reader kept, so it is asserted by itself here as the lexer's are there.
    #[test]
    fn a_directive_that_is_not_read_keeps_the_code_it_was_assigned() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "#define X 1\n");
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        preprocess(file, &tokens, &mut sources, &mut diagnostics);

        let codes: Vec<String> = diagnostics
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code())
            .map(|code| code.to_string())
            .collect();
        assert_eq!(codes, ["SC0104"]);
    }
}
