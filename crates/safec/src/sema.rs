//! Which declaration each name refers to.
//!
//! The first half of the stage `docs/architecture.md` draws between the AST
//! and the typed AST: this says which declaration a name means, and `types.rs`
//! says what type each expression has. A binding carries the type its
//! declarator derived, which is what that module reads it for.
//!
//! The bindings are this module's own rather than nodes in the tree. A
//! `Declaration` lives in three places, and one of them, a parameter inside
//! `Type::Function`, has no id at all, so a resolution addressed by tree id
//! could not name it. Owning the bindings means every declared name is
//! addressable here whatever the tree does with it.
//!
//! Names are compared as source text. ADR-0006 says an interner is worth
//! having when identifiers are compared often, which is here, and in the same
//! record fixes the signal to add one as the *parser* asking for a
//! `&SourceMap`. This is not that moment, and nothing here measures a cost:
//! the program this phase targets has three names in it.

use std::collections::HashMap;

use crate::ast::{Ast, Declaration, Expr, ExprId, Item, Parameters, Stmt, StmtId, Type, TypeId};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use safec_ir::source::{SourceMap, Span};

/// A name used where nothing declares it.
///
/// `SC03xx` is names and types. `docs/diagnostics.md` allocates the ranges and
/// this takes the first of that one; the wording follows `clang`, which says
/// `use of undeclared identifier 'x'`, because a user arriving from a C
/// toolchain should not have to learn a second phrasing for the same thing.
const UNDECLARED: Code = Code::new("SC0301");

/// One declared name, addressed by [`BindingId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BindingId(u32);

/// A name the program declared, and where.
///
/// Not `Symbol`, which ADR-0006 has already spent on the interned identifier
/// it says to add when the parser asks for a `&SourceMap`. That is a different
/// thing: an interned name is one string however many times it is written, and
/// this is one declaration of it. `rustc` uses `Symbol` in ADR-0006's sense.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// The span of the name, which is also how it is compared.
    pub name: Span,
    /// The type the declarator derived for it.
    ///
    /// Carried here rather than looked up again, because the three places a
    /// declaration lives reach their type three different ways and one of
    /// them, a parameter, has no id to look anything up by. `types.rs` is what
    /// reads this.
    pub ty: TypeId,
}

/// What each use of a name refers to.
///
/// Keyed by the id of the identifier expression, which ADR-0008 makes unique
/// by construction. A span is not: a macro body is one piece of text, so two
/// uses expanded from one `#define` carry one span and can refer to two
/// different declarations. A table keyed on that keeps the later answer and
/// says `Some` for both, which is the shape of wrong answer this compiler
/// exists to not give. ADR-0006's addendum ruled out an index into the token
/// stream, which the tree does not hold; the tree's own id is a third thing
/// and is what this uses.
///
/// One of these belongs to one translation unit, which is what C means by a
/// scope for a file-scope name. `resolve` builds a fresh one per input and
/// there is no way to add to it from outside, deliberately: two files that
/// declare `x` declare two different things.
#[derive(Clone, Debug)]
pub struct Resolution {
    bindings: Vec<Binding>,
    resolved: HashMap<ExprId, BindingId>,
}

impl Resolution {
    /// The binding `id` names.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`Resolution`].
    pub fn binding(&self, id: BindingId) -> &Binding {
        &self.bindings[id.0 as usize]
    }

    /// What the name at `use_site` refers to, if anything did.
    ///
    /// `None` is either a name that was reported as undeclared or an id that
    /// is not an identifier at all. The two are told apart by the diagnostics,
    /// not here.
    pub fn resolved(&self, use_site: ExprId) -> Option<BindingId> {
        self.resolved.get(&use_site).copied()
    }
}

/// Resolve every name in `ast`, reporting the ones nothing declares.
///
/// The caller decides whether this runs. `driver.rs` does not call it for an
/// input that anything reported on, the scan included: the lexer drops a
/// preprocessor directive and the parser drops everything after the first
/// thing it cannot read, and in both cases the tree is a record of a program
/// this compiler did not finish reading. A name diagnostic drawn from one of
/// those is a claim about a program nobody wrote.
pub fn resolve(sources: &SourceMap, ast: &Ast, diagnostics: &mut DiagnosticSink) -> Resolution {
    let mut resolver = Resolver {
        sources,
        ast,
        resolution: Resolution {
            bindings: Vec::new(),
            resolved: HashMap::new(),
        },
        // The file scope, which is never popped.
        scopes: vec![Vec::new()],
        children: Vec::new(),
    };

    for item in ast.items() {
        resolver.item(item, diagnostics);
    }

    resolver.resolution
}

struct Resolver<'a> {
    sources: &'a SourceMap,
    /// Shared, and held in a field rather than passed per method.
    ///
    /// That is what lets `let ast = self.ast;` free the walk below to borrow
    /// `self` mutably. It works only while the reference is shared: the day
    /// this stage has to *create* a type, the field has to become a `&mut Ast`
    /// parameter on each method and three comments here stop being true.
    /// `types.rs` is written that way already and says why.
    ast: &'a Ast,
    resolution: Resolution,
    /// Innermost last. Never empty: the file scope is pushed before the walk
    /// and popped by nobody.
    scopes: Vec<Vec<BindingId>>,
    /// Reused across every expression walk, so that a run allocates once.
    children: Vec<ExprId>,
}

impl Resolver<'_> {
    fn item(&mut self, item: &Item, diagnostics: &mut DiagnosticSink) {
        match item {
            Item::Function(function) => {
                self.walk_type(function.ty, diagnostics);
                // Declared before the body is walked, so that a function can
                // call itself. C17 6.2.1 p7 puts the start of a file-scope
                // name at the end of its declarator, which is before the body.
                self.declare(function.name, function.ty);

                self.scopes.push(Vec::new());
                self.parameters(function.ty, diagnostics);
                // The body is a compound statement and pushes a scope of its
                // own, so a parameter sits one scope outside the block rather
                // than in it, where C17 6.2.1 p4 puts it. The difference is
                // invisible to everything here: it decides only whether
                // `int f(int a) { int a; }` is a redeclaration or a shadowing,
                // and this stage reports neither. #57 is where that lands.
                self.stmt(function.body, diagnostics);
                self.scopes.pop();
            }
            Item::Declaration(declaration) => self.declaration(declaration, diagnostics),
            Item::Error { .. } => {}
        }
    }

    /// A declaration, wherever it was written.
    ///
    /// The type is walked before the name is declared, because an array length
    /// is an expression and C17 6.2.1 p7 starts a name's scope at the end of
    /// its declarator: in `int n[n];` the length is the outer `n`, if there is
    /// one, and not the array being declared.
    ///
    /// The parameters get a scope that opens and closes here, which is what
    /// C17 6.2.1 p4 gives a declarator that is not part of a definition: their
    /// names are visible to each other and to nothing else. A definition's
    /// parameters are the same names in a scope [`Resolver::item`] keeps open
    /// for the body instead.
    fn declaration(&mut self, declaration: &Declaration, diagnostics: &mut DiagnosticSink) {
        self.walk_type(declaration.ty, diagnostics);

        self.scopes.push(Vec::new());
        self.parameters(declaration.ty, diagnostics);
        self.scopes.pop();

        if let Some(name) = declaration.name {
            self.declare(name, declaration.ty);
        }
    }

    /// The names a function declarator wrote between its parentheses, and the
    /// expressions inside their types, in the scope the caller has open.
    ///
    /// Every name goes in before any length is walked. C17 6.7.6.2 lets a
    /// parameter's array length name another parameter, `int f(int n, int
    /// a[n])`, which `clang` accepts and which the parser here reads, so a
    /// length looked up before its siblings are declared would be reported as
    /// undeclared: a false positive about valid C. Declaring first is looser
    /// than C, which starts each name at the end of its own declarator, and
    /// the direction to be loose in is the one that stays quiet.
    ///
    /// Only the outermost type, which is what `driver.rs::dump_parameters`
    /// does as well: a definition whose declarator derives something other than
    /// a function is a constraint violation, and reporting it is #57's rather
    /// than this walk's to invent an answer for.
    fn parameters(&mut self, ty: TypeId, diagnostics: &mut DiagnosticSink) {
        let ast = self.ast;
        let Type::Function {
            parameters: Parameters::Prototype(parameters),
            ..
        } = ast.ty(ty)
        else {
            return;
        };

        for parameter in parameters {
            if let Some(name) = parameter.name {
                self.declare(name, parameter.ty);
            }
        }

        for parameter in parameters {
            self.walk_type(parameter.ty, diagnostics);
        }
    }

    /// The expressions inside a type, which is its array lengths.
    ///
    /// A recursion, bounded the way `parser.rs::apply` bounds a declarator's
    /// derivations, so a type is at most `MAX_NESTING` deep.
    ///
    /// A function's parameters are not walked here, because they need a scope
    /// of their own and this has none to give: [`Resolver::parameters`] is
    /// where they are declared and their lengths looked up. Doing it here
    /// instead reports `n` in `int f(int n, int a[n])` as undeclared, which is
    /// a false positive about valid C.
    fn walk_type(&mut self, ty: TypeId, diagnostics: &mut DiagnosticSink) {
        match self.ast.ty(ty) {
            Type::Int | Type::Char | Type::Void => {}
            Type::Pointer(pointee) => self.walk_type(*pointee, diagnostics),
            Type::Array { element, length } => {
                if let Some(length) = *length {
                    self.expr(length, diagnostics);
                }
                self.walk_type(*element, diagnostics);
            }
            Type::Function { returns, .. } => self.walk_type(*returns, diagnostics),
        }
    }

    /// One statement and everything under it.
    ///
    /// A recursion, for the reason `driver.rs::dump_stmt` gives: every place a
    /// statement nests inside another is a recursion in the parser too, and
    /// `MAX_NESTING` bounds those.
    fn stmt(&mut self, id: StmtId, diagnostics: &mut DiagnosticSink) {
        // Taken out of `self` so that the walk below can borrow `self` mutably.
        // The tree outlives this resolver and is not what is being changed.
        let ast = self.ast;

        match ast.stmt(id) {
            Stmt::Compound { body, .. } => {
                self.scopes.push(Vec::new());
                for &statement in body {
                    self.stmt(statement, diagnostics);
                }
                self.scopes.pop();
            }
            Stmt::Declaration(declaration) => self.declaration(declaration, diagnostics),
            Stmt::Return { value, .. } | Stmt::Expression { value, .. } => {
                if let Some(value) = *value {
                    self.expr(value, diagnostics);
                }
            }
            Stmt::If {
                condition,
                then,
                otherwise,
                ..
            } => {
                self.expr(*condition, diagnostics);
                self.stmt(*then, diagnostics);
                if let Some(otherwise) = *otherwise {
                    self.stmt(otherwise, diagnostics);
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.expr(*condition, diagnostics);
                self.stmt(*body, diagnostics);
            }
            Stmt::For {
                initialiser,
                condition,
                step,
                body,
                ..
            } => {
                for clause in [*initialiser, *condition, *step].into_iter().flatten() {
                    self.expr(clause, diagnostics);
                }
                self.stmt(*body, diagnostics);
            }
            Stmt::Error { .. } => {}
        }
    }

    /// Every name under one expression.
    ///
    /// An explicit stack rather than a recursion, because an expression tree
    /// has no bound on its depth: two of the parser's rules fold with a loop,
    /// which is why `driver.rs::dump_expr` carries its own stack as well.
    ///
    /// Mutation: walk the children by calling this on each of them instead.
    /// `the_artifact_grows_with_the_source_rather_than_with_its_square` and
    /// `a_tree_deeper_than_a_format_width_does_not_end_the_process` in
    /// `tests/exit_code.rs` both fail, because the process is killed rather
    /// than reporting anything. A thousand terms is not enough to do it and
    /// the twenty five hundred one of those tests writes is, which is why the
    /// bound is worth having rather than arguing about.
    fn expr(&mut self, root: ExprId, diagnostics: &mut DiagnosticSink) {
        let ast = self.ast;
        let mut pending = vec![root];

        while let Some(id) = pending.pop() {
            let expr = ast.expr(id);

            if let Expr::Identifier { span } = *expr {
                match self.lookup(span) {
                    Some(binding) => {
                        self.resolution.resolved.insert(id, binding);
                    }
                    None => diagnostics.report(undeclared(self.sources.snippet(span), span)),
                }
            }

            // Cleared per node, because `extend_children` appends. Pushed in
            // reverse so that they come back off in the order they were
            // written, which is the order the diagnostics are reported in.
            self.children.clear();
            expr.extend_children(&mut self.children);
            pending.extend(self.children.iter().rev().copied());
        }
    }

    /// The innermost declaration of the name written at `use_site`.
    fn lookup(&self, use_site: Span) -> Option<BindingId> {
        let name = self.sources.snippet(use_site);

        self.scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.iter().rev())
            .find(|&&binding| self.sources.snippet(self.resolution.binding(binding).name) == name)
            .copied()
    }

    /// Add a name to the innermost scope.
    ///
    /// The id is taken before the push, not from `len()` after it, which is the
    /// mistake ADR-0008 records for the tree's arenas and the same one here.
    fn declare(&mut self, name: Span, ty: TypeId) {
        let id = BindingId(self.resolution.bindings.len() as u32);
        self.resolution.bindings.push(Binding { name, ty });
        self.scopes
            .last_mut()
            .expect("the file scope is never popped")
            .push(id);
    }
}

/// `name` is source text, and it reaches a terminal through this message.
///
/// Interpolated raw rather than through `{:?}`, which is safe because the
/// lexer's `is_identifier_continue` admits only `is_ascii_alphanumeric` and
/// `_`, so an identifier cannot carry a control character. That is a property
/// of the lexer rather than of this function, and `lexer.rs` lists non-ASCII
/// identifiers as a gap it may close, so this is the line to revisit when it
/// does.
fn undeclared(name: &str, span: Span) -> Diagnostic {
    Diagnostic::error(format!("use of undeclared identifier `{name}`"))
        .with_code(UNDECLARED)
        .with_label(Label::primary(
            span,
            "no declaration for this name is in scope",
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;
    use safec_ir::source::FileId;

    struct Resolved {
        ast: Ast,
        resolution: Resolution,
        diagnostics: DiagnosticSink,
        sources: SourceMap,
        file: FileId,
    }

    /// Scan, parse and resolve, the way the driver does.
    fn resolved(text: &str) -> Resolved {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("test.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, sources.file(file), &mut diagnostics);
        let ast = parse(file, &tokens, &mut diagnostics);
        assert!(
            !diagnostics.has_errors(),
            "the input did not parse: {:?}",
            diagnostics.diagnostics()
        );
        let resolution = resolve(&sources, &ast, &mut diagnostics);

        Resolved {
            ast,
            resolution,
            diagnostics,
            sources,
            file,
        }
    }

    impl Resolved {
        /// The span of the `nth` place `name` is written as a whole word,
        /// counting from zero.
        ///
        /// A test names a position the way a reader finds one, rather than by
        /// counting bytes, so that editing the input above does not silently
        /// move what the assertion is about. Whole words, because `a` is
        /// otherwise found inside `add` and `main`, which is how the first
        /// version of this helper made two of these tests ask the wrong
        /// question.
        fn occurrence(&self, name: &str, nth: usize) -> Span {
            let is_part_of_a_name = |c: char| c.is_ascii_alphanumeric() || c == '_';
            let text = self.sources.file(self.file).contents();
            let start = text
                .match_indices(name)
                .filter(|(at, _)| {
                    !text[..*at].ends_with(is_part_of_a_name)
                        && !text[at + name.len()..].starts_with(is_part_of_a_name)
                })
                .nth(nth)
                .unwrap_or_else(|| panic!("{name:?} is not written {} times", nth + 1))
                .0 as u32;

            Span::new(self.file, start, start + name.len() as u32)
        }

        /// The identifier expression written at `span`.
        ///
        /// A test says where a name is by pointing at the text, and the table
        /// is keyed by the tree's id, so something has to cross between them.
        /// `Ast::expr_ids` is what makes that possible from outside the module
        /// that owns the arena.
        fn used_at(&self, span: Span) -> ExprId {
            self.ast
                .expr_ids()
                .find(
                    |&id| matches!(self.ast.expr(id), Expr::Identifier { span: at } if *at == span),
                )
                .unwrap_or_else(|| panic!("no identifier is written at {span:?}"))
        }

        /// Where the name used at the `nth` occurrence of `text` was declared.
        fn declaration_of(&self, text: &str, nth: usize) -> Option<Span> {
            let use_site = self.used_at(self.occurrence(text, nth));
            let binding = self.resolution.resolved(use_site)?;
            Some(self.resolution.binding(binding).name)
        }

        fn messages(&self) -> Vec<&str> {
            self.diagnostics
                .diagnostics()
                .iter()
                .map(Diagnostic::message)
                .collect()
        }
    }

    /// The program `docs/roadmap.md` sets as the target, resolved.
    ///
    /// Three names are used and each has to find the right one of the four
    /// declared: the parameters `a` and `b` from inside the body, which is the
    /// acceptance criterion about a parameter, and `add` from the function
    /// after it.
    ///
    /// Mutation: stop declaring a parameter in `Resolver::parameters`. The two
    /// parameters stop resolving and this fails.
    ///
    /// What it does *not* hold is the order `item` declares a function in:
    /// `add` is written above `main`, so declaring a function's name after its
    /// body rather than before still resolves this program. That is what
    /// `a_function_can_call_itself` is for, and it was measured rather than
    /// assumed, because this comment said otherwise first.
    #[test]
    fn the_mvp_program_resolves_every_name_it_uses() {
        let resolved = resolved(
            "int add(int a, int b) {\n    return a + b;\n}\n\nint main(void) {\n    return add(1, 2);\n}\n",
        );

        assert_eq!(resolved.messages(), Vec::<&str>::new());

        // The second `a` is the use in `a + b`, and the first is the parameter
        // it has to find.
        assert_eq!(
            resolved.declaration_of("a", 1),
            Some(resolved.occurrence("a", 0))
        );
        assert_eq!(
            resolved.declaration_of("b", 1),
            Some(resolved.occurrence("b", 0))
        );
        assert_eq!(
            resolved.declaration_of("add", 1),
            Some(resolved.occurrence("add", 0))
        );
    }

    /// A function's own name is in scope inside its body.
    ///
    /// The only thing that turns on `item` declaring the name before it walks
    /// the body rather than after: every other program here calls a function
    /// written above it, which resolves either way.
    ///
    /// Mutation: move the `declare` in `Resolver::item` below the
    /// `self.stmt(function.body, ...)` call. `f` stops resolving inside itself
    /// and this fails. Nothing else in the suite notices.
    #[test]
    fn a_function_can_call_itself() {
        let resolved = resolved("int f(int n) {\n    return f(n);\n}\n");

        assert_eq!(resolved.messages(), Vec::<&str>::new());
        assert_eq!(
            resolved.declaration_of("f", 1),
            Some(resolved.occurrence("f", 0))
        );
    }

    /// Mutation: have `Resolver::expr` ignore a lookup that found nothing.
    /// Nothing is reported and this fails.
    #[test]
    fn a_name_nothing_declares_is_reported_where_it_is_used() {
        let resolved = resolved("int main(void) { return undeclared; }\n");

        assert_eq!(
            resolved.messages(),
            ["use of undeclared identifier `undeclared`"]
        );

        let [reported] = resolved.diagnostics.diagnostics() else {
            panic!("{:?}", resolved.diagnostics.diagnostics());
        };
        assert_eq!(
            reported.code().map(|code| code.to_string()).as_deref(),
            Some("SC0301")
        );
        assert_eq!(
            reported.primary_label().map(Label::span),
            Some(resolved.occurrence("undeclared", 0))
        );
    }

    /// Mutation: never pop the scope a compound statement pushes. The name
    /// resolves outside its block, nothing is reported, and this fails.
    #[test]
    fn a_name_declared_in_a_block_does_not_leave_it() {
        let resolved = resolved("int main(void) { { int i; } return i; }\n");

        assert_eq!(resolved.messages(), ["use of undeclared identifier `i`"]);
    }

    /// The innermost declaration wins, and each use is answered on its own.
    ///
    /// Mutation: drop the `rev` over `self.scopes` in `Resolver::lookup`. The
    /// outer `x` answers instead and this fails.
    ///
    /// The other `rev`, over the bindings within one scope, is held by nothing
    /// and cannot be until #57: it decides which of two declarations of one
    /// name in one scope answers, and C makes that either an error, inside a
    /// block, or two spellings of one object, at file scope. There is no
    /// program whose meaning this compiler can state today that tells the two
    /// orders apart.
    #[test]
    fn a_name_finds_the_innermost_declaration_of_it() {
        let resolved = resolved("int main(void) { int x; { int x; return x; } return x; }\n");

        assert_eq!(resolved.messages(), Vec::<&str>::new());
        // The use inside the block finds the inner declaration, and the one
        // after it finds the outer, from the same table.
        assert_eq!(
            resolved.declaration_of("x", 2),
            Some(resolved.occurrence("x", 1))
        );
        assert_eq!(
            resolved.declaration_of("x", 3),
            Some(resolved.occurrence("x", 0))
        );
    }

    /// An array's length is source the program wrote, so a name in it is a use.
    ///
    /// Mutation: have `Resolver::declaration` skip `walk_type`. The `n` is
    /// never looked up, nothing is reported for the second program, and this
    /// fails.
    #[test]
    fn a_name_in_an_array_length_is_used_like_any_other() {
        let declared = resolved("int main(void) { int n; int a[n]; return 0; }\n");
        assert_eq!(declared.messages(), Vec::<&str>::new());
        assert_eq!(
            declared.declaration_of("n", 1),
            Some(declared.occurrence("n", 0))
        );

        let undeclared = resolved("int main(void) { int a[n]; return 0; }\n");
        assert_eq!(undeclared.messages(), ["use of undeclared identifier `n`"]);
    }

    /// Every place a name can be written, and the order they come back in.
    ///
    /// One undeclared name per position, so the list below says which of them
    /// the walk reached: a missing entry is a place a use goes unchecked, and
    /// this compiler being silent about a name it never looked at is worse
    /// than being wrong about one it did.
    ///
    /// The order is the order they are written, and it is asserted because it
    /// is what a reader sees.
    ///
    /// `p` is the control: it is a parameter, it is used, and it is absent
    /// from the list.
    ///
    /// Mutation: delete any one of the `self.expr` or `self.stmt` calls in
    /// `Resolver::stmt`, or the `walk_type` call in `Resolver::declaration`.
    /// One name leaves the list and this fails. Before it existed, taking the
    /// condition out of `Stmt::If` broke nothing in the whole suite.
    #[test]
    fn every_place_a_name_can_be_written_is_looked_at() {
        let resolved = resolved(
            "int outer[ol];\n\nint main(int p) {\n    int inner[il];\n    if (c1) e1; else e2;\n    while (c2) e3;\n    for (i1; c3; s1) e4;\n    p;\n    return r1 + r2;\n}\n",
        );

        assert_eq!(
            resolved.messages(),
            [
                "use of undeclared identifier `ol`",
                "use of undeclared identifier `il`",
                "use of undeclared identifier `c1`",
                "use of undeclared identifier `e1`",
                "use of undeclared identifier `e2`",
                "use of undeclared identifier `c2`",
                "use of undeclared identifier `e3`",
                "use of undeclared identifier `i1`",
                "use of undeclared identifier `c3`",
                "use of undeclared identifier `s1`",
                "use of undeclared identifier `e4`",
                "use of undeclared identifier `r1`",
                "use of undeclared identifier `r2`",
            ]
        );
    }

    /// C17 6.7.6.2 lets a parameter's array length name another parameter, and
    /// `clang` accepts it, so it is not a name to report. A length that names
    /// nothing at all is, in a definition and in a declaration alike: `clang`
    /// rejects both, verified with `--target=x86_64-unknown-linux-gnu`.
    ///
    /// The two halves are one test because the first version of this code had
    /// only the first half of the rule and skipped every parameter's type to
    /// get it, which made the second half silent. A test for either alone
    /// passes against that.
    ///
    /// Mutation: walk the parameters' types before declaring their names in
    /// `Resolver::parameters`. The sibling case starts reporting `n` and this
    /// fails. Mutation: skip the second loop there. The two undeclared cases
    /// stop reporting and this fails from the other side.
    #[test]
    fn a_parameter_may_size_an_array_with_another_parameter_and_nothing_else() {
        let sibling = resolved("int f(int n, int a[n]) { return 0; }\n");
        assert_eq!(sibling.messages(), Vec::<&str>::new());

        let definition = resolved("int f(int a[nowhere]) { return 0; }\n");
        assert_eq!(
            definition.messages(),
            ["use of undeclared identifier `nowhere`"]
        );

        let declaration = resolved("int f(int a[nowhere]);\n");
        assert_eq!(
            declaration.messages(),
            ["use of undeclared identifier `nowhere`"]
        );
    }

    /// A parameter is gone when the function it belongs to is.
    ///
    /// Mutation: remove the `self.scopes.pop()` that closes the parameter
    /// scope in `Resolver::item`. `a` is still in scope inside `g`, nothing is
    /// reported, and this fails. Before it existed, that `pop` was held by
    /// nothing in the suite: every other program declares one function.
    #[test]
    fn a_parameter_does_not_reach_the_function_after_it() {
        let resolved =
            resolved("int f(int a) {\n    return a;\n}\n\nint g(void) {\n    return a;\n}\n");

        assert_eq!(resolved.messages(), ["use of undeclared identifier `a`"]);
    }
}
