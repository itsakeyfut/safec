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
/// An `#include` nested deeper than [`MAX_DEPTH`].
const TOO_DEEP: Code = Code::new("SC0111");
/// An `#include` past [`MAX_INCLUSIONS`].
const TOO_MANY: Code = Code::new("SC0112");
/// The lexer's code for a splice or a trigraph it does not perform, which an
/// `#include` line is refused with for the same reason.
const UNSUPPORTED_SPLICE: Code = Code::new("SC0107");

/// How many files may be being read at once, the input among them.
///
/// The recursion takes a native stack frame per file, so a chain of different
/// files, each including the next, ended the process with nothing to read
/// once it was long enough: 300 overflowed a debug build on Windows, measured,
/// and 250 did not. Found by review. C17 5.2.4.1 asks for 15 levels, so this
/// leaves a program written for any compiler room, and a refusal past it says
/// where.
const MAX_DEPTH: usize = 200;

/// How many inclusions one input may make in all.
///
/// [`MAX_DEPTH`] bounds a chain, not a tree: a header that includes the next
/// one twice, fifteen deep, is 65,536 inclusions, each a file of its own
/// (ADR-0054), and every two levels more took four times as long, measured,
/// with nothing printed until it ended. An include guard would stop that in a
/// real header, and `#ifndef` is not read yet. Found by review.
const MAX_INCLUSIONS: usize = 4096;

/// The files being read, and how many have been included so far.
struct Reading {
    /// Outermost first, each by its canonical path, so a cycle is found
    /// whatever spelling reached it.
    open: Vec<(PathBuf, FileId)>,
    included: usize,
}

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
    let mut reading = Reading {
        open: Vec::new(),
        included: 0,
    };
    if let Some(path) = path_of(sources.file(file).name()) {
        reading.open.push((canonical(path), file));
    }

    let mut spliced = Vec::with_capacity(tokens.len());
    splice(tokens, sources, &mut reading, diagnostics, &mut spliced);
    spliced.extend(tokens.iter().copied().filter(|token| token.is_eof()));
    spliced
}

/// Append `tokens` to `spliced`, an `#include "file"` replaced by that file's,
/// and no end of file.
///
/// Recursion ends because a file being read is never read again, a chain of
/// different files stops at [`MAX_DEPTH`], and a tree of them at
/// [`MAX_INCLUSIONS`].
fn splice(
    tokens: &[Token],
    sources: &mut SourceMap,
    reading: &mut Reading,
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
                    Form::Spliced => diagnostics.report(
                        refused(
                            &text,
                            token.span,
                            UNSUPPORTED_SPLICE,
                            "a line splice or a trigraph in an `#include` is not read yet",
                        )
                        .with_note(
                            "C can join the line below to this one, and it would be \
                             compiled as code instead",
                        ),
                    ),
                    Form::Angled => diagnostics.report(
                        refused(
                            &text,
                            token.span,
                            UNSUPPORTED_DIRECTIVE,
                            "`#include <...>` is not read yet",
                        )
                        .with_note("only `#include \"...\"` is read yet"),
                    ),
                    Form::Other => diagnostics.report(
                        refused(
                            &text,
                            token.span,
                            UNSUPPORTED_DIRECTIVE,
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
    reading: &mut Reading,
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

    if reading.open.len() == MAX_DEPTH {
        diagnostics.report(
            Diagnostic::error("`#include`s are nested too deeply")
                .with_code(TOO_DEEP)
                .with_label(Label::primary(quoted, "included here"))
                .with_note(format!("{MAX_DEPTH} files are already being read here")),
        );
        return;
    }
    // Once, at the first one past it, rather than at each of the thousands
    // after it, which would bury the report it is.
    if reading.included >= MAX_INCLUSIONS {
        if reading.included > MAX_INCLUSIONS {
            return;
        }
        reading.included += 1;
        diagnostics.report(
            Diagnostic::error("this input includes too many files")
                .with_code(TOO_MANY)
                .with_label(Label::primary(quoted, "included here"))
                .with_note(format!(
                    "{MAX_INCLUSIONS} inclusions have already been read for it, and \
                     include guards are not read yet"
                )),
        );
        return;
    }

    let canonical = canonical(&path);
    if let Some(&(_, open)) = reading.open.iter().find(|(being, _)| *being == canonical) {
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

    // Asked before the file is opened, because opening is what never returns
    // for a console or a pipe: `#include "CON"` waited on the terminal until
    // it was killed. Found by review. A device that cannot even be asked, as
    // Windows answers for `CON`, is refused with what it said.
    let not_a_file = match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() || metadata.is_dir() => None,
        Ok(_) => Some("this is not a regular file, and safec reads a source file".to_owned()),
        Err(error) => Some(reason(&path, &error, Access::Read)),
    };
    if let Some(why) = not_a_file {
        diagnostics.report(unreadable(format!(
            "`{name}` was looked for at `{}`: {why}",
            path.display()
        )));
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

    reading.included += 1;
    let tokens = lex(file, &sources.file_owned(file), diagnostics);
    reading.open.push((canonical, file));
    splice(&tokens, sources, reading, diagnostics, spliced);
    reading.open.pop();
}

/// What an `#include` is spelled as, which decides what is done with it.
#[derive(Debug, PartialEq, Eq)]
enum Form<'a> {
    /// `#include "name"`, with `at` the offset of the opening quote in the
    /// directive's text.
    Quoted { name: &'a str, at: u32 },
    /// `#include <name>`.
    Angled,
    /// `#include "name"` on a line that ends in a splice or holds a trigraph.
    ///
    /// Both change what the line is before any directive is found (C17
    /// 5.1.1.2 p1, phases 1 and 2), and the lexer ends a directive at its
    /// newline without performing either. Read as it stands, `// \` after the
    /// name left the line below to be compiled where C makes it part of the
    /// comment, and a null test there went missing from the safety check.
    /// Found by review.
    Spliced,
    /// Any other directive, and an `#include` that is neither of those, such
    /// as one naming a macro.
    Other,
}

/// Read a directive's text, from its `#` to the end of its line.
///
/// Space and horizontal tab may stand between the `#`, the name and the file
/// (C17 6.10 p5). A vertical tab and a form feed are taken as spaces, which
/// the same paragraph allows translation phase 3 to have done, and a comment
/// may follow the file. A `\r` is whitespace here too, because the
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

    let trigraph = text
        .as_bytes()
        .windows(3)
        .any(|three| three[..2] == *b"??" && b"=()/'<>!-".contains(&three[2]));
    if text.trim_end_matches([' ', '\t', '\r']).ends_with('\\') || trigraph {
        return Form::Spliced;
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
fn refused(text: &str, directive: Span, code: Code, message: &str) -> Diagnostic {
    // Two bytes where the `#` is written `%:`.
    let hash = if text.starts_with("%:") { 2 } else { 1 };
    Diagnostic::error(message)
        .with_code(code)
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

    /// Every spelling C allows of the one form read.
    ///
    /// Mutation: take `\r` out of `blank`. The line a CRLF file ends with is
    /// not read.
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

    /// And the ones beside it that are not it. `#included` needs no test of
    /// its own in `form`: what follows `include` has to be `<` or `"`.
    ///
    /// Mutation: accept anything after the closing quote. `"a.h" x` is read.
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
            ("#include \"a.h\" // \\", Form::Spliced),
            ("#include \"a.h\" // \\ \r", Form::Spliced),
            ("#include \"a.h\" // ??/", Form::Spliced),
            ("#include \"a??=.h\"", Form::Spliced),
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

    /// The codes reported for `main.c` in a directory of its own holding
    /// `files`, each a path under it and what it holds.
    fn reported(name: &str, files: &[(String, String)]) -> Vec<String> {
        let directory = std::env::temp_dir().join(format!("safec_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        for (path, contents) in files {
            let path = directory.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }

        let mut sources = SourceMap::new();
        let file = sources.load(directory.join("main.c")).unwrap();
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        preprocess(file, &tokens, &mut sources, &mut diagnostics);
        let _ = fs::remove_dir_all(&directory);

        diagnostics
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code())
            .map(|code| code.to_string())
            .collect()
    }

    /// A chain of different files, each including the next, is refused past
    /// the bound rather than read until the native stack runs out, and read
    /// up to it: `MAX_DEPTH` files at once, the input among them.
    ///
    /// Mutation: drop the check of `MAX_DEPTH`. The longer chain is read to
    /// its end and nothing is reported. Not an overflow here: a test thread's
    /// stack is larger than the main thread's, which is where the bound is
    /// needed. Mutation: refuse one file sooner or one later. One of the two
    /// chains answers wrongly.
    #[test]
    fn includes_nested_past_the_bound_are_refused() {
        let chain = |headers: usize| {
            let mut files = vec![("main.c".to_owned(), "#include \"0.h\"\n".to_owned())];
            for at in 0..headers - 1 {
                files.push((format!("{at}.h"), format!("#include \"{}.h\"\n", at + 1)));
            }
            files.push((format!("{}.h", headers - 1), "int z;\n".to_owned()));
            files
        };

        assert_eq!(
            reported("deep_enough", &chain(MAX_DEPTH - 1)),
            Vec::<String>::new()
        );
        assert_eq!(reported("too_deep", &chain(MAX_DEPTH)), ["SC0111"]);
    }

    /// A header that includes the next one twice, thirteen deep, is refused
    /// once its inclusions pass the bound, and said once.
    ///
    /// Mutation: drop the bound. Every inclusion is read and nothing is
    /// reported. Mutation: report every inclusion past it. One is reported
    /// for each file still open, since a refused one reads nothing more.
    #[test]
    fn a_tree_of_inclusions_past_the_bound_is_refused_once() {
        let levels = 13;
        let mut files = vec![("main.c".to_owned(), "#include \"0.h\"\n".to_owned())];
        for at in 0..levels {
            let next = format!("#include \"{}.h\"\n", at + 1);
            files.push((format!("{at}.h"), next.repeat(2)));
        }
        files.push((format!("{levels}.h"), "int z;\n".to_owned()));

        assert_eq!(reported("wide", &files), ["SC0112"]);
    }

    /// A device is refused before it is opened, since opening one can wait
    /// for ever: `CON` waited on the terminal.
    ///
    /// Mutation: drop the question asked before opening. The device reads as
    /// an empty file and nothing is reported.
    #[test]
    fn an_include_of_a_device_is_refused() {
        let device = if cfg!(windows) {
            "//./NUL"
        } else {
            "/dev/null"
        };
        let files = [("main.c".to_owned(), format!("#include \"{device}\"\n"))];

        assert_eq!(reported("device", &files), ["SC0109"]);
    }

    /// An included file's own includes are found beside it, not beside the
    /// input: `sub/b.h` writes `c.h` and means `sub/c.h`.
    ///
    /// Mutation: resolve every include against the input's directory. `c.h`
    /// is looked for beside `main.c`, where there is none.
    #[test]
    fn an_include_in_an_included_file_is_found_beside_that_file() {
        let files = [
            ("main.c", "#include \"sub/b.h\"\n"),
            ("sub/b.h", "#include \"c.h\"\n"),
            ("sub/c.h", "int c;\n"),
        ]
        .map(|(path, contents)| (path.to_owned(), contents.to_owned()));

        assert_eq!(reported("nested", &files), Vec::<String>::new());
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
