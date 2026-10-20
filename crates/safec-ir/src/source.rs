//! Source files and the positions inside them.
//!
//! Everything downstream attaches positions to what it produces, so this is the
//! substrate the rest of the compiler is built on. Two decisions shape it.
//!
//! Positions are byte offsets, not line and column pairs. The lexer would pay
//! for line counting on every token, and lines are only needed when something
//! is reported. Instead a file indexes its line starts once, and offsets are
//! resolved on demand.
//!
//! A [`Span`] names its file rather than indexing into one global coordinate
//! space. That costs four bytes per span next to the single-space design, and
//! buys a span that means something on its own, in a debugger or a `{:?}`.

use std::fmt;
use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A handle to a file in a [`SourceMap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(u32);

impl FileId {
    /// The index this handle refers to.
    ///
    /// For a side table keyed by file. Not for reaching into a [`SourceMap`]:
    /// use [`SourceMap::file`].
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// A handle for the file at `index`.
    ///
    /// For a caller inside the crate that already knows the position, such as
    /// a test. Ordinary code receives a handle from
    /// [`SourceMap::add_virtual`] or [`SourceMap::load`] instead of building
    /// one.
    ///
    /// Deliberately not `pub`. A handle is only meaningful against the map it
    /// came from, and [`SourceMap::file`] cannot tell a foreign one from its
    /// own, so letting callers outside the crate mint handles would make it easy
    /// to resolve a span against the wrong file and report a diagnostic about
    /// code that is fine.
    ///
    /// # Panics
    ///
    /// If `index` does not fit in a `u32`.
    #[cfg(test)]
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("more than 4 billion source files"))
    }
}

/// Where a source file came from.
///
/// Not every file has a path. Test inputs do not, and neither will the buffers
/// the Clang adapter hands over, or the expansion of a macro once there is a
/// preprocessor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileName {
    /// A file read from disk, named as the user spelled it.
    ///
    /// Kept verbatim rather than canonicalized: a diagnostic should quote the
    /// path the user typed, not its absolute form.
    Real(PathBuf),
    /// A file with no path, named for the reader.
    Virtual(String),
    /// A file read because an `#include` named it, one of these per inclusion.
    ///
    /// An instance in ADR-0054's sense: two inclusions of one header are two
    /// files, never one, so no span from the first can equal one from the
    /// second.
    Inclusion {
        /// The file as it was resolved: the including file's directory joined
        /// to what the directive spelled.
        path: PathBuf,
        /// The directive, in the including file.
        at: Span,
    },
}

impl fmt::Display for FileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Real(path) => write!(f, "{}", path.display()),
            Self::Virtual(name) => write!(f, "<{name}>"),
            // The path alone, so a diagnostic in a header names the header.
            // Where it was included from is `at`'s to say, not the name's.
            Self::Inclusion { path, .. } => write!(f, "{}", path.display()),
        }
    }
}

/// A region of a source file, as a half-open range of byte offsets.
///
/// The fields are private on purpose. A macro expansion, and later a template
/// instantiation, needs to say both where the code is written and where it
/// came from, and ADR-0054 decides how without a third field: every expansion
/// is a copy of the macro's body registered as a file of its own, its tokens
/// are located in that copy, and the source map records where the copy was
/// spelled and used. So within one expansion spans compare as they would
/// written out, and a containment test across an expansion lifts both spans to
/// a file they share first.
///
/// [`Span::new`] and the accessors below pin the shape, and the decision keeps
/// it. What changes is how spans from two files are joined, which goes through
/// the source map's lift rather than [`Span::to`] alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    file: FileId,
    start: u32,
    end: u32,
}

impl Span {
    /// A span covering `start..end` in `file`.
    ///
    /// # Panics
    ///
    /// If `end` is before `start`.
    pub fn new(file: FileId, start: u32, end: u32) -> Self {
        assert!(start <= end, "span ends at {end} but starts at {start}");
        Self { file, start, end }
    }

    /// An empty span at `offset`, for pointing between two characters.
    pub fn at(file: FileId, offset: u32) -> Self {
        Self::new(file, offset, offset)
    }

    /// The file this span belongs to.
    pub fn file(self) -> FileId {
        self.file
    }

    /// The byte offset the span starts at.
    pub fn start(self) -> u32 {
        self.start
    }

    /// The byte offset one past the end of the span.
    pub fn end(self) -> u32 {
        self.end
    }

    /// The length of the span, in bytes.
    pub fn len(self) -> u32 {
        self.end - self.start
    }

    /// Whether the span covers no text.
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// The span as a range, for slicing the file contents.
    pub fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    /// The smallest span covering both this one and `other`.
    ///
    /// This is how a parser builds the span of a node from the spans of its
    /// parts, so it also covers whatever sits between them.
    ///
    /// # Panics
    ///
    /// If the spans are in different files. Joining those would produce a span
    /// that cannot be shown, so it is treated as a bug rather than resolved to
    /// one file or the other.
    pub fn to(self, other: Self) -> Self {
        assert_eq!(
            self.file, other.file,
            "cannot join spans from different files"
        );
        Self {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// Which end of a span a lift moves, and checks against its instance's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// The start, which has to be where the instance's first token is.
    Start,
    /// The end, which has to be where the instance's last token is.
    End,
}

/// Where a join would hold some of an instance's tokens and not others.
///
/// What [`SourceMap::join`] answers instead of a span. ADR-0054 refuses such
/// a node rather than joining it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cut {
    /// The end that does not reach its instance's edge, at the level of the
    /// chain where it does not.
    pub span: Span,
    /// The instance it cuts.
    pub instance: FileId,
    /// Which end.
    pub edge: Edge,
}

impl fmt::Display for Cut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let edge = match self.edge {
            Edge::Start => "start",
            Edge::End => "end",
        };
        write!(
            f,
            "{:?} cuts {:?}: its {edge} is not its instance's",
            self.span, self.instance
        )
    }
}

/// A position as a reader would give it: 1-based line and column.
///
/// The column counts characters rather than bytes, so that a line holding
/// non-ASCII text in a comment or a string literal still points where the
/// reader expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    /// The line, counting from 1.
    pub line: u32,
    /// The column, counting characters from 1.
    pub column: u32,
}

impl fmt::Display for LineCol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// One source file, and the index needed to resolve offsets inside it.
#[derive(Debug)]
pub struct SourceFile {
    name: FileName,
    contents: String,
    /// The byte offset of the first character of each line.
    ///
    /// Always begins with 0, so a file always has at least one line, and is
    /// strictly increasing, which is what lets an offset be found by binary
    /// search.
    line_starts: Vec<u32>,
}

impl SourceFile {
    fn new(name: FileName, mut contents: String) -> Self {
        // A leading byte order mark is not part of the program. Stripping it
        // here keeps every offset in the file relative to what is stored, so
        // nothing downstream has to know the mark was ever there.
        const BOM: &str = "\u{feff}";
        if contents.starts_with(BOM) {
            contents.drain(..BOM.len());
        }

        assert!(
            u32::try_from(contents.len()).is_ok(),
            "source files larger than 4 GiB are not supported: {name}"
        );

        let line_starts = std::iter::once(0)
            .chain(
                contents
                    .bytes()
                    .enumerate()
                    .filter(|(_, byte)| *byte == b'\n')
                    .map(|(offset, _)| offset as u32 + 1),
            )
            .collect();

        Self {
            name,
            contents,
            line_starts,
        }
    }

    /// What to call this file in a diagnostic.
    pub fn name(&self) -> &FileName {
        &self.name
    }

    /// The text of the file.
    pub fn contents(&self) -> &str {
        &self.contents
    }

    /// The length of the file, in bytes.
    pub fn len(&self) -> u32 {
        self.contents.len() as u32
    }

    /// Whether the file holds no text.
    pub fn is_empty(&self) -> bool {
        self.contents.is_empty()
    }

    /// The number of lines in the file.
    ///
    /// A file ending in a newline has a final empty line, matching how an
    /// editor numbers the same file.
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// The line `offset` falls on, counting from 0.
    ///
    /// # Panics
    ///
    /// If `offset` is past the end of the file.
    pub fn line_index(&self, offset: u32) -> usize {
        assert!(
            offset <= self.len(),
            "offset {offset} is past the end of {}",
            self.name
        );
        // Every line start at or before the offset is on an earlier line or on
        // this one, so the count of them is the 1-based line number.
        self.line_starts.partition_point(|&start| start <= offset) - 1
    }

    /// The byte range of a line, including its terminator.
    ///
    /// `line_index` counts from 0, the way [`SourceFile::line_index`] returns
    /// it, not from 1 the way [`LineCol::line`] reports it.
    ///
    /// # Panics
    ///
    /// If `line_index` is past the end of the file.
    pub fn line_range(&self, line_index: usize) -> Range<usize> {
        assert!(
            line_index < self.line_count(),
            "line {line_index} is past the end of {}",
            self.name
        );
        let start = self.line_starts[line_index] as usize;
        let end = self
            .line_starts
            .get(line_index + 1)
            .map_or(self.contents.len(), |&start| start as usize);
        start..end
    }

    /// The text of a line, without its terminator.
    ///
    /// Exactly one terminator comes off. A `\r` with no `\n` after it is
    /// content and stays, the same way a `\r` in the middle of a line does.
    ///
    /// `line_index` counts from 0, the way [`SourceFile::line_index`] returns
    /// it, not from 1 the way [`LineCol::line`] reports it.
    ///
    /// # Panics
    ///
    /// If `line_index` is past the end of the file.
    pub fn line_text(&self, line_index: usize) -> &str {
        let text = &self.contents[self.line_range(line_index)];
        match text.strip_suffix('\n') {
            Some(without_newline) => without_newline
                .strip_suffix('\r')
                .unwrap_or(without_newline),
            None => text,
        }
    }

    /// Where `offset` is, as a reader would give it.
    ///
    /// # Panics
    ///
    /// If `offset` is past the end of the file, or falls inside a character.
    pub fn line_col(&self, offset: u32) -> LineCol {
        let line = self.line_index(offset);
        let start = self.line_starts[line] as usize;
        let column = self.contents[start..offset as usize].chars().count();
        LineCol {
            line: line as u32 + 1,
            column: column as u32 + 1,
        }
    }
}

/// Every source file the compiler has been given.
///
/// Files are added once and never removed, which is what makes a [`FileId`] a
/// plain index and a [`Span`] cheap to copy.
///
/// Each file is behind an [`Arc`] so that one can be read while another is
/// added. `#include` is that case: a lexer holding the text of `a.c` meets one
/// and has to call [`SourceMap::load`], which takes `&mut self` and cannot have
/// it while the text is borrowed. [`SourceMap::file_owned`] hands out a handle
/// that outlives the borrow. `Arc` rather than `Rc` because the roadmap has
/// thread safety analysis in it, and the difference costs nothing until
/// something is actually shared. See ADR-0005.
#[derive(Debug, Default)]
pub struct SourceMap {
    files: Vec<Arc<SourceFile>>,
    /// Per file, the first and last thing of an instance the parser is
    /// handed, once the preprocessor has said. See [`SourceMap::set_extent`].
    extents: Vec<Option<(Span, Span)>>,
}

impl SourceMap {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a file from disk and add it.
    pub fn load(&mut self, path: impl AsRef<Path>) -> io::Result<FileId> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)?;
        Ok(self.add(FileName::Real(path.to_path_buf()), contents))
    }

    /// Read a file an `#include` at `at` names, and add it as an inclusion.
    ///
    /// A new file on every call, a second inclusion of the same path
    /// included. See [`FileName::Inclusion`].
    ///
    /// # Panics
    ///
    /// If `at` is not in a file already in the map. Every walk along a chain
    /// of uses, [`SourceMap::locate`] and the lifts among them, ends because
    /// each use is in a file registered before the instance it is the use
    /// of; a use in the file being added, or a later one, would be a chain
    /// with no root, and the walk would not end. Found by review.
    pub fn include(&mut self, path: impl AsRef<Path>, at: Span) -> io::Result<FileId> {
        assert!(
            at.file().index() < self.files.len(),
            "an inclusion's use {at:?} is in no file already registered, so its chain would have no root"
        );
        let path = path.as_ref();
        let contents = fs::read_to_string(path)?;
        Ok(self.add(
            FileName::Inclusion {
                path: path.to_path_buf(),
                at,
            },
            contents,
        ))
    }

    /// Add a file that is not on disk.
    ///
    /// For test inputs today. Later for the buffers the Clang adapter hands
    /// over. A macro expansion and an inclusion get a kind of their own, since
    /// each records where it was used (ADR-0054).
    pub fn add_virtual(&mut self, name: impl Into<String>, contents: impl Into<String>) -> FileId {
        self.add(FileName::Virtual(name.into()), contents.into())
    }

    fn add(&mut self, name: FileName, contents: String) -> FileId {
        let id = FileId(self.files.len() as u32);
        self.files.push(Arc::new(SourceFile::new(name, contents)));
        self.extents.push(None);
        id
    }

    /// Record the first and last thing of an instance the parser is handed.
    ///
    /// Recorded by the preprocessor, which hands it over, since an instance
    /// is registered before it is read and its text need not begin or end
    /// with what reaches the parser: a header can open with a comment, or a
    /// line that hands nothing over. Asked rather than worked out from the
    /// text, because working it out would be a second copy of the lexer's and
    /// the preprocessor's rules, and a copy that disagreed would let a lift
    /// take a cut for a whole instance. An instance nested in this one counts
    /// as its use, so an included file whose first line is an `#include`
    /// begins at that directive, and what it includes lifts through it.
    ///
    /// # Panics
    ///
    /// If `file` is not an instance, or either span is not in it.
    pub fn set_extent(&mut self, file: FileId, first: Span, last: Span) {
        assert!(
            self.at(file).is_some(),
            "only an instance has an extent, and {file:?} is not one"
        );
        assert!(
            first.file() == file && last.file() == file,
            "the extent of {file:?} is in another file: {first:?}, {last:?}"
        );
        self.extents[file.index()] = Some((first, last));
    }

    /// Where an instance was used, or `None` for a file that is not one.
    ///
    /// The one place a kind of instance answers for its use, so a second
    /// kind is an arm here and nothing else. See ADR-0054.
    pub fn at(&self, file: FileId) -> Option<Span> {
        match self.file(file).name() {
            FileName::Inclusion { at, .. } => Some(*at),
            FileName::Real(_) | FileName::Virtual(_) => None,
        }
    }

    /// `span` lifted to its use in `into`, checking at every level of the
    /// chain that it starts where its instance's first token does.
    ///
    /// For a join: the start of what this returns is where a node holding
    /// `span` at its start begins in `into`. On a contiguous token stream the
    /// check is exactly that the node holds the whole instance at its start,
    /// so a node that cuts one cannot be joined. See ADR-0054.
    ///
    /// # Panics
    ///
    /// Where `span` does not start at its instance's first token at some
    /// level, naming the cut, or where the chain ends before `into`.
    pub fn lift_start(&self, span: Span, into: FileId) -> Span {
        self.lifted(span, into, Edge::Start)
            .unwrap_or_else(|cut| panic!("{cut}"))
    }

    /// `span` lifted to its use in `into`, checking at every level of the
    /// chain that it ends where its instance's last token does. The end's
    /// counterpart of [`SourceMap::lift_start`].
    ///
    /// # Panics
    ///
    /// Where `span` does not end at its instance's last token at some level,
    /// naming the cut, or where the chain ends before `into`.
    pub fn lift_end(&self, span: Span, into: FileId) -> Span {
        self.lifted(span, into, Edge::End)
            .unwrap_or_else(|cut| panic!("{cut}"))
    }

    /// The span from `from`'s start to `to`'s end, in the nearest file both
    /// reach, or where it cuts an instance.
    ///
    /// The one join across instances, for every node a parser builds and
    /// every span built after it: `from` is lifted by its start and `to` by
    /// its end, each checked at every level as [`SourceMap::lift_start`] and
    /// [`SourceMap::lift_end`] check, so a join holds every instance it
    /// touches whole or is not made. A cut is answered rather than panicked
    /// on, because a construct that cuts an instance is C, and refusing it is
    /// the caller's to say. See ADR-0054.
    ///
    /// Two spans with no file in common, which no translation unit holds,
    /// are a cut of `from`'s file.
    pub fn join(&self, from: Span, to: Span) -> Result<Span, Cut> {
        let Some(file) = self.shared(from, to) else {
            return Err(Cut {
                span: from,
                instance: from.file(),
                edge: Edge::Start,
            });
        };
        let start = self.lifted(from, file, Edge::Start)?;
        let end = self.lifted(to, file, Edge::End)?;
        // `from`'s start and `to`'s end, not the smallest span covering both:
        // a declarator's store runs from its name to the end of a span that
        // starts earlier, at the specifiers.
        Ok(Span::new(file, start.start(), end.end()))
    }

    /// `span` lifted into `into` by one edge, or the first level it cuts.
    fn lifted(&self, mut span: Span, into: FileId, edge: Edge) -> Result<Span, Cut> {
        while span.file() != into {
            let file = span.file();
            let Some(at) = self.at(file) else {
                panic!("{span:?} cannot be lifted into {into:?}: {file:?} is not an instance");
            };
            // An instance with nothing the parser read has no extent, and no
            // span in it can be joined, so this is a defect in the caller.
            let Some((first, last)) = self.extents[file.index()] else {
                panic!("{file:?} has no recorded extent, so {span:?} cannot be lifted out of it");
            };
            let whole = match edge {
                Edge::Start => span.start() == first.start(),
                Edge::End => span.end() == last.end(),
            };
            if !whole {
                return Err(Cut {
                    span,
                    instance: file,
                    edge,
                });
            }
            span = at;
        }
        Ok(span)
    }

    /// `span` moved along its chain to its use in `into`, any span at all, or
    /// `None` where the chain never reaches `into`.
    ///
    /// For a comparison, which asks where something is and not whether a node
    /// may hold it, so an interior span is located as well as an edge.
    pub fn locate(&self, mut span: Span, into: FileId) -> Option<Span> {
        while span.file() != into {
            span = self.at(span.file())?;
        }
        Some(span)
    }

    /// The nearest file both spans' chains reach, or `None`.
    pub fn shared(&self, a: Span, b: Span) -> Option<FileId> {
        let mut here = Some(a);
        while let Some(span) = here {
            if self.locate(b, span.file()).is_some() {
                return Some(span.file());
            }
            here = self.at(span.file());
        }
        None
    }

    /// Whether `inner` lies within `outer`, `outer` itself included, where
    /// both are located in the nearest file they share; `false` where they
    /// share none.
    pub fn within(&self, outer: Span, inner: Span) -> bool {
        let Some(file) = self.shared(outer, inner) else {
            return false;
        };
        match (self.locate(outer, file), self.locate(inner, file)) {
            (Some(outer), Some(inner)) => {
                outer.start() <= inner.start() && inner.end() <= outer.end()
            }
            _ => false,
        }
    }

    /// `span`'s chain from its root down, `(file index, start)` at each level,
    /// the outermost first: the key a total order by where code is reads.
    ///
    /// Two spans compare at the nearest file they share, a tie there is
    /// settled one level in, and two with no file in common compare by their
    /// roots, which is the order by file index there was before instances.
    pub fn position(&self, span: Span) -> Vec<(usize, u32)> {
        let mut chain = vec![(span.file().index(), span.start())];
        let mut here = span;
        while let Some(at) = self.at(here.file()) {
            chain.push((at.file().index(), at.start()));
            here = at;
        }
        chain.reverse();
        chain
    }

    /// The file a handle refers to.
    ///
    /// A [`FileId`] is a plain index, so a handle belonging to another map is
    /// not detected: it panics only when the index happens to be out of range,
    /// and otherwise returns whichever file sits at that position. A span and
    /// the map it came from have to be kept together.
    ///
    /// # Panics
    ///
    /// If the index is past the end of this map.
    pub fn file(&self, id: FileId) -> &SourceFile {
        &self.files[id.index()]
    }

    /// The file a handle refers to, as a handle of its own.
    ///
    /// The same file as [`SourceMap::file`], borrowing nothing from the map, so
    /// a caller can keep reading it while the map is added to. That is what a
    /// lexer needs when it meets an `#include`: the text it is scanning has to
    /// survive the [`SourceMap::load`] the directive causes.
    ///
    /// Prefer [`SourceMap::file`] where the borrow is not in the way. It is the
    /// same file either way; this one costs an atomic increment and says, at
    /// the call site, that the handle is meant to outlive the borrow.
    ///
    /// # Panics
    ///
    /// If the index is past the end of this map.
    pub fn file_owned(&self, id: FileId) -> Arc<SourceFile> {
        Arc::clone(&self.files[id.index()])
    }

    /// The number of files in the map.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether the map holds no files.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Every file in the map, in the order they were added.
    pub fn files(&self) -> impl Iterator<Item = (FileId, &SourceFile)> {
        self.files
            .iter()
            .enumerate()
            .map(|(index, file)| (FileId(index as u32), &**file))
    }

    /// The text a span covers.
    ///
    /// # Panics
    ///
    /// If the span reaches past the end of its file, or does not fall on
    /// character boundaries.
    pub fn snippet(&self, span: Span) -> &str {
        &self.file(span.file()).contents()[span.range()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(contents: &str) -> SourceFile {
        SourceFile::new(FileName::Virtual("test".to_owned()), contents.to_owned())
    }

    #[test]
    fn an_empty_file_still_has_one_line() {
        let file = file("");
        assert_eq!(file.line_count(), 1);
        assert_eq!(file.line_text(0), "");
        assert_eq!(file.line_col(0), LineCol { line: 1, column: 1 });
    }

    #[test]
    fn a_trailing_newline_opens_a_final_empty_line() {
        // What an editor shows: "a\n" puts the cursor on line 2.
        let file = file("a\n");
        assert_eq!(file.line_count(), 2);
        assert_eq!(file.line_text(1), "");
    }

    #[test]
    fn resolves_offsets_across_line_boundaries() {
        let file = file("ab\ncd\n");
        let at = |offset| file.line_col(offset);

        assert_eq!(at(0), LineCol { line: 1, column: 1 });
        assert_eq!(at(2), LineCol { line: 1, column: 3 }); // the newline itself
        assert_eq!(at(3), LineCol { line: 2, column: 1 });
        assert_eq!(at(6), LineCol { line: 3, column: 1 }); // end of file
    }

    #[test]
    fn columns_count_characters_rather_than_bytes() {
        // A diagnostic pointing into a line with a non-ASCII comment has to
        // land where the reader sees the code, not where the bytes are.
        // "int x; /* " is 10 characters, the comment text 13 and " */ " another
        // 4, so "int y" begins at the 28th character but the 54th byte.
        let file = file("int x; /* シンプルで安全なコンパイラ */ int y;");
        let byte_offset = file.contents().find("int y").unwrap() as u32;
        let column = file.line_col(byte_offset).column;

        assert_eq!(column, 28);
        assert_ne!(column, byte_offset + 1, "the column is counting bytes");
    }

    #[test]
    fn a_line_excludes_its_terminator_in_either_convention() {
        assert_eq!(file("a\nb").line_text(0), "a");
        assert_eq!(file("a\r\nb").line_text(0), "a");
    }

    #[test]
    fn a_byte_order_mark_does_not_shift_the_first_line() {
        let file = file("\u{feff}int main(void) {}");
        assert!(file.contents().starts_with("int"));
        assert_eq!(file.line_col(0), LineCol { line: 1, column: 1 });
    }

    #[test]
    fn joining_spans_covers_what_lies_between_them() {
        let id = FileId(0);
        let left = Span::new(id, 4, 6);
        let right = Span::new(id, 12, 15);

        assert_eq!(left.to(right), Span::new(id, 4, 15));
        assert_eq!(right.to(left), Span::new(id, 4, 15), "join is symmetric");
    }

    #[test]
    #[should_panic(expected = "different files")]
    fn joining_spans_from_different_files_is_a_bug() {
        Span::new(FileId(0), 0, 1).to(Span::new(FileId(1), 0, 1));
    }

    #[test]
    fn a_span_names_the_text_it_covers() {
        let mut map = SourceMap::new();
        let id = map.add_virtual("test", "int x = 42;");
        let span = Span::new(id, 8, 10);

        assert_eq!(map.snippet(span), "42");
        assert_eq!(map.file(id).line_col(span.start()).column, 9);
    }

    #[test]
    fn a_handle_built_from_an_index_refers_to_that_index() {
        assert_eq!(FileId::from_index(0).index(), 0);
        assert_eq!(FileId::from_index(3).index(), 3);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    #[should_panic(expected = "more than 4 billion source files")]
    fn a_handle_beyond_the_range_of_a_file_id_is_a_bug() {
        FileId::from_index(usize::MAX);
    }

    #[test]
    fn files_keep_the_order_they_were_added_in() {
        let mut map = SourceMap::new();
        let first = map.add_virtual("a.c", "");
        let second = map.add_virtual("b.c", "");

        assert_ne!(first, second);
        assert_eq!(map.len(), 2);
        // `index()` is documented as the key for side tables keyed by file, so
        // it has to be the position the file was added at.
        assert_eq!(first.index(), 0);
        assert_eq!(second.index(), 1);
        assert_eq!(
            map.files().map(|(id, _)| id).collect::<Vec<_>>(),
            [first, second]
        );
    }

    #[test]
    fn a_carriage_return_that_is_not_a_terminator_is_content() {
        // Only one terminator comes off. A `\r` with no `\n` after it is text,
        // exactly as it is in the middle of a line.
        assert_eq!(file("a\r\r\n").line_text(0), "a\r");
        assert_eq!(file("a\r").line_text(0), "a\r");
        assert_eq!(file("a\rb").line_text(0), "a\rb");
    }

    #[test]
    fn a_line_range_keeps_the_terminator_that_line_text_drops() {
        let file = file("ab\ncd");

        assert_eq!(file.line_range(0), 0..3);
        assert_eq!(file.line_range(1), 3..5);
        assert_eq!(file.line_text(0), "ab");
    }

    #[test]
    fn a_span_reports_its_own_extent() {
        let id = FileId(0);
        let span = Span::new(id, 4, 9);

        assert_eq!(span.file(), id);
        assert_eq!(span.start(), 4);
        assert_eq!(span.end(), 9);
        assert_eq!(span.len(), 5);
        assert!(!span.is_empty());
        assert_eq!(span.range(), 4..9);
    }

    #[test]
    fn an_empty_span_points_between_two_characters() {
        let span = Span::at(FileId(0), 7);

        assert_eq!(span.start(), 7);
        assert_eq!(span.end(), 7);
        assert_eq!(span.len(), 0);
        assert!(span.is_empty());
        assert_eq!(span.range(), 7..7);
    }

    /// The `main.c` half of a diagnostic header. Editors and test harnesses
    /// read this, so the bracketing and the separator are part of the interface.
    #[test]
    fn file_names_render_for_a_diagnostic_header() {
        assert_eq!(
            FileName::Real(PathBuf::from("src/main.c")).to_string(),
            "src/main.c"
        );
        assert_eq!(
            FileName::Virtual("expansion".to_owned()).to_string(),
            "<expansion>"
        );
        assert_eq!(
            FileName::Inclusion {
                path: PathBuf::from("src/list.h"),
                at: Span::at(FileId(0), 0),
            }
            .to_string(),
            "src/list.h"
        );
    }

    /// The `12:5` half of the same header.
    #[test]
    fn positions_render_as_a_line_and_a_column() {
        assert_eq!(
            LineCol {
                line: 12,
                column: 5
            }
            .to_string(),
            "12:5"
        );
    }

    #[test]
    fn loading_a_file_keeps_the_path_as_it_was_given() {
        let path = std::env::temp_dir().join("safec_keeps_the_path_as_given.c");
        fs::write(&path, "int main(void) { return 0; }").unwrap();

        let mut map = SourceMap::new();
        let id = map.load(&path).unwrap();

        assert_eq!(map.file(id).name(), &FileName::Real(path.clone()));
        assert_eq!(map.file(id).contents(), "int main(void) { return 0; }");

        fs::remove_file(&path).unwrap();
    }

    /// `main.c`, written in memory, includes `outer`, which includes `b.h`:
    /// a chain of two instances with each extent recorded, as the
    /// preprocessor records it. `outer` is written to a directory of its own
    /// named by `tag`, so tests can run at once.
    ///
    /// `outer`'s text holds `#include "b.h"` and the `at` of `b.h` is that
    /// directive; `b.h` is ` int z;`, whose tokens are at 1, 5 and 6.
    fn chain(tag: &str, outer: &str) -> (SourceMap, FileId, FileId, FileId) {
        let directory =
            std::env::temp_dir().join(format!("safec_chain_{tag}_{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("a.h"), outer).unwrap();
        fs::write(directory.join("b.h"), " int z;").unwrap();

        let mut map = SourceMap::new();
        let main = map.add_virtual("main.c", "x = 0;\n#include \"a.h\"\ny = 1;\n");
        let a = map
            .include(directory.join("a.h"), Span::new(main, 7, 21))
            .unwrap();
        let directive = outer.find('#').unwrap() as u32;
        let b = map
            .include(
                directory.join("b.h"),
                Span::new(a, directive, directive + 14),
            )
            .unwrap();
        fs::remove_dir_all(&directory).unwrap();

        // The first and last token of each, as a scan would find them.
        let tokens: Vec<(u32, u32)> = if outer.starts_with('#') {
            vec![(0, 14), (15, 18), (19, 20), (20, 21)]
        } else {
            vec![(0, 3), (4, 5), (5, 6), (7, 21)]
        };
        let (first, last) = (tokens[0], tokens[tokens.len() - 1]);
        map.set_extent(
            a,
            Span::new(a, first.0, first.1),
            Span::new(a, last.0, last.1),
        );
        map.set_extent(b, Span::new(b, 1, 4), Span::new(b, 6, 7));
        (map, main, a, b)
    }

    /// `int z;`'s `int` starts `b.h`, whose directive starts `a.h`, so the
    /// start of a node holding `b.h` whole is the `#include` of `a.h`.
    #[test]
    fn a_span_at_the_first_token_of_every_level_lifts_its_start() {
        let (map, main, _, b) = chain("start", "#include \"b.h\"\nint y;");
        assert_eq!(
            map.lift_start(Span::new(b, 1, 4), main),
            Span::new(main, 7, 21)
        );
    }

    /// `b.h` is whole at its own level, and its `#include` is not where
    /// `a.h` starts, so a node starting at `int` cuts `a.h`.
    ///
    /// Mutation: check only the innermost level. The start lifts.
    #[test]
    #[should_panic(expected = "cuts FileId(1): its start is not its instance's")]
    fn a_span_whole_in_its_own_instance_but_not_in_the_next_does_not_lift_its_start() {
        let (map, main, _, b) = chain("cut_start", "int w;\n#include \"b.h\"");
        map.lift_start(Span::new(b, 1, 4), main);
    }

    /// The end's counterpart: `b.h`'s `;` ends it, and its `#include` ends
    /// `a.h`.
    #[test]
    fn a_span_at_the_last_token_of_every_level_lifts_its_end() {
        let (map, main, _, b) = chain("end", "int w;\n#include \"b.h\"");
        assert_eq!(
            map.lift_end(Span::new(b, 6, 7), main),
            Span::new(main, 7, 21)
        );
    }

    /// Mutation: check only the innermost level. The end lifts.
    #[test]
    #[should_panic(expected = "cuts FileId(1): its end is not its instance's")]
    fn a_span_whole_in_its_own_instance_but_not_in_the_next_does_not_lift_its_end() {
        let (map, main, _, b) = chain("cut_end", "#include \"b.h\"\nint y;");
        map.lift_end(Span::new(b, 6, 7), main);
    }

    /// A join holding an inclusion whole is made where its `#include` is,
    /// from `from`'s start to `to`'s end: `b.h` starts `a.h` in the first
    /// chain and ends it in the second.
    #[test]
    fn a_join_holding_an_instance_whole_is_made_at_its_use() {
        let (map, main, _, b) = chain("join", "#include \"b.h\"\nint y;");
        assert_eq!(
            map.join(Span::new(b, 1, 4), Span::new(main, 22, 23)),
            Ok(Span::new(main, 7, 23))
        );

        let (map, main, _, b) = chain("join_last", "int w;\n#include \"b.h\"");
        assert_eq!(
            map.join(Span::new(main, 0, 1), Span::new(b, 6, 7)),
            Ok(Span::new(main, 0, 21))
        );
    }

    /// A join whose start is partway through an instance names the cut,
    /// rather than panicking or joining.
    ///
    /// Mutation: have `join` lift without checking the edge. It answers a
    /// span.
    #[test]
    fn a_join_starting_partway_through_an_instance_is_a_cut() {
        let (map, main, _, b) = chain("join_start", "#include \"b.h\"\nint y;");

        assert_eq!(
            map.join(Span::new(b, 5, 6), Span::new(main, 22, 23)),
            Err(Cut {
                span: Span::new(b, 5, 6),
                instance: b,
                edge: Edge::Start,
            })
        );
    }

    /// The end's counterpart, at the outer level: `b.h` ends at its `;`, and
    /// its `#include` is not where `a.h` ends, which is `int y;`.
    #[test]
    fn a_join_ending_partway_through_an_instance_is_a_cut() {
        let (map, main, a, b) = chain("join_end", "#include \"b.h\"\nint y;");

        assert_eq!(
            map.join(Span::new(main, 0, 1), Span::new(b, 6, 7)),
            Err(Cut {
                span: Span::new(a, 0, 14),
                instance: a,
                edge: Edge::End,
            })
        );
    }

    /// Two spans no chain relates are a cut, never a span made of offsets
    /// from two files.
    #[test]
    fn a_join_of_spans_that_share_no_file_is_a_cut() {
        let (mut map, main, _, _) = chain("join_none", "#include \"b.h\"\nint y;");
        let elsewhere = map.add_virtual("other.c", "int q;\n");

        assert_eq!(
            map.join(Span::new(main, 0, 1), Span::new(elsewhere, 0, 3)),
            Err(Cut {
                span: Span::new(main, 0, 1),
                instance: main,
                edge: Edge::Start,
            })
        );
    }

    /// An interior span is located as an edge is, and a span already in the
    /// file asked for is itself.
    ///
    /// Mutation: have `locate` return the span unchanged. The first two
    /// assertions fail.
    #[test]
    fn locating_moves_any_span_to_its_use_and_leaves_one_already_there() {
        let (mut map, main, a, b) = chain("locate", "#include \"b.h\"\nint y;");
        let z = Span::new(b, 5, 6);

        assert_eq!(map.locate(z, main), Some(Span::new(main, 7, 21)));
        assert_eq!(map.locate(z, a), Some(Span::new(a, 0, 14)));
        assert_eq!(
            map.locate(Span::new(main, 0, 1), main),
            Some(Span::new(main, 0, 1))
        );
        let elsewhere = map.add_virtual("other.c", "int q;\n");
        assert_eq!(map.locate(z, elsewhere), None);
    }

    /// A span in `b.h` is within a span of `main.c` that holds the
    /// `#include` of `a.h`, and not within one that does not; a span in a
    /// file the chain never reaches is within nothing.
    ///
    /// Mutation: compare without locating. The first assertion fails.
    #[test]
    fn a_span_in_an_instance_is_within_what_holds_its_use() {
        let (mut map, main, _, b) = chain("within", "#include \"b.h\"\nint y;");
        let z = Span::new(b, 5, 6);

        assert!(map.within(Span::new(main, 0, 28), z));
        assert!(!map.within(Span::new(main, 0, 6), z));
        let elsewhere = map.add_virtual("other.c", "int q;\n");
        assert!(!map.within(Span::new(elsewhere, 0, 6), z));
    }

    /// The order a span's chain gives is the order of the code: `b.h`'s span
    /// is after `x = 0;` and before `y = 1;`, though `b.h` was registered
    /// after both.
    ///
    /// Mutation: read the chain innermost first rather than from its root.
    /// The second assertion fails.
    #[test]
    fn a_span_in_an_instance_is_ordered_where_its_use_is() {
        let (map, main, _, b) = chain("order", "#include \"b.h\"\nint y;");
        let z = map.position(Span::new(b, 5, 6));

        assert!(map.position(Span::new(main, 0, 1)) < z);
        assert!(z < map.position(Span::new(main, 22, 23)));
    }

    /// A use in a file not yet registered would give a chain with no root,
    /// which every walk along uses would follow for ever.
    ///
    /// Mutation: drop the assertion in `include`. The inclusion is added, and
    /// this does not panic.
    #[test]
    #[should_panic(expected = "is in no file already registered")]
    fn an_inclusion_whose_use_is_in_no_registered_file_is_refused() {
        let mut map = SourceMap::new();
        let main = map.add_virtual("main.c", "#include \"a.h\"\n");
        let later = FileId(main.0 + 1);
        let _ = map.include(std::env::temp_dir(), Span::new(later, 0, 1));
    }

    /// Two inclusions of one header are two files (ADR-0054), so a span in
    /// one never equals a span in the other, and each remembers its own
    /// directive.
    ///
    /// Mutation: have `include` return the id of a file already holding the
    /// same path. Both assertions fail.
    #[test]
    fn including_one_file_twice_adds_two_files() {
        let path = std::env::temp_dir().join("safec_including_one_file_twice.h");
        fs::write(&path, "int f(void);").unwrap();

        let mut map = SourceMap::new();
        let main = map.add_virtual(
            "main.c",
            "#include \"a.h\"
#include \"a.h\"
",
        );
        let first = map.include(&path, Span::new(main, 0, 14)).unwrap();
        let second = map.include(&path, Span::new(main, 15, 29)).unwrap();

        assert_ne!(first, second);
        assert_eq!(
            map.file(second).name(),
            &FileName::Inclusion {
                path: path.clone(),
                at: Span::new(main, 15, 29)
            }
        );

        fs::remove_file(&path).unwrap();
    }

    /// The shape a lexer needs. `#include` means reading one file and adding
    /// another at the same time, and a borrow of the map cannot survive the
    /// `&mut` that adding takes.
    ///
    /// The guard is the compiler rather than the assertions: this does not
    /// build against a map that hands out only borrows, which is what
    /// [`SourceMap::file`] alone would do. Replacing `file_owned` with `file`
    /// here is the reversal, and it fails to compile.
    #[test]
    fn a_file_can_be_read_while_another_is_added() {
        let mut sources = SourceMap::new();
        let first = sources.add_virtual(
            "a.c",
            "#include \"a.h\"
",
        );

        let held = sources.file_owned(first);
        let text = held.contents();

        // What a lexer does on reaching the directive, while still scanning.
        let second = sources.add_virtual(
            "a.h", "int x;
",
        );

        assert_eq!(
            text,
            "#include \"a.h\"
"
        );
        assert_eq!(
            sources.file(second).contents(),
            "int x;
"
        );
    }

    #[test]
    fn loading_a_missing_file_reports_it_as_not_found() {
        let mut map = SourceMap::new();
        let error = map
            .load(std::env::temp_dir().join("safec_no_such_file.c"))
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(map.is_empty(), "a failed load must not add a file");
    }

    #[test]
    #[should_panic(expected = "span ends at 3 but starts at 5")]
    fn a_span_cannot_end_before_it_starts() {
        Span::new(FileId(0), 5, 3);
    }

    #[test]
    #[should_panic(expected = "is past the end of")]
    fn an_offset_past_the_end_of_the_file_is_a_bug() {
        file("ab").line_index(3);
    }

    #[test]
    #[should_panic(expected = "line 2 is past the end of")]
    fn a_line_past_the_end_of_the_file_is_a_bug() {
        file("a\n").line_range(2);
    }

    #[test]
    #[should_panic(expected = "char boundary")]
    fn an_offset_inside_a_character_is_a_bug() {
        file("日").line_col(1);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn a_span_reaching_past_the_end_of_its_file_is_a_bug() {
        let mut map = SourceMap::new();
        let id = map.add_virtual("test", "ab");

        map.snippet(Span::new(id, 0, 99));
    }
}
