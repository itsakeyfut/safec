//! Which declaration each name refers to.
//!
//! The stage `docs/architecture.md` draws between the AST and the typed AST,
//! or the first slice of it: this says which declaration a name means, and
//! nothing about what type it has. #58 is where a type joins a symbol.
//!
//! The symbols are this module's own rather than nodes in the tree. A
//! `Declaration` lives in three places, and one of them, a parameter inside
//! `Type::Function`, has no id at all, so a resolution addressed by tree id
//! could not name it. Owning the symbols means every declared name is
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
use crate::source::{SourceMap, Span};

/// A name used where nothing declares it.
///
/// `SC03xx` is names and types. `docs/diagnostics.md` allocates the ranges and
/// this takes the first of that one; the wording follows `clang`, which says
/// `use of undeclared identifier 'x'`, because a user arriving from a C
/// toolchain should not have to learn a second phrasing for the same thing.
const UNDECLARED: Code = Code::new("SC0301");

/// One declared name, addressed by [`SymbolId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId(u32);

/// A name the program declared, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    /// The span of the name, which is also how it is compared.
    pub name: Span,
}

/// What each use of a name refers to.
///
/// Keyed by the span of the *use*, per ADR-0006's addendum: an identifier
/// carries a `Span` and name resolution walks the tree, so a span is what it
/// has to key on. Two uses of one name are two spans, and a span names its own
/// file, so one table can hold a whole run if it is ever asked to.
#[derive(Clone, Debug)]
pub struct Resolution {
    symbols: Vec<Symbol>,
    resolved: HashMap<Span, SymbolId>,
}

impl Resolution {
    /// The symbol `id` names.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`Resolution`].
    pub fn symbol(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id.0 as usize]
    }

    /// What the name written at `use_site` refers to, if anything did.
    ///
    /// `None` is either a name that was reported as undeclared or a span that
    /// is not a use at all. The two are told apart by the diagnostics, not
    /// here.
    pub fn resolved(&self, use_site: Span) -> Option<SymbolId> {
        self.resolved.get(&use_site).copied()
    }
}

/// Resolve every name in `ast`, reporting the ones nothing declares.
///
/// The caller decides whether this runs. `driver.rs` does not call it for an
/// input whose parse reported an error: that parser stops at the first thing it
/// cannot read and drops the rest, so a name diagnostic drawn from what
/// survives would be a claim about a program this compiler did not finish
/// reading.
pub fn resolve(sources: &SourceMap, ast: &Ast, diagnostics: &mut DiagnosticSink) -> Resolution {
    let mut resolver = Resolver {
        sources,
        ast,
        resolution: Resolution {
            symbols: Vec::new(),
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
    ast: &'a Ast,
    resolution: Resolution,
    /// Innermost last. Never empty: the file scope is pushed before the walk
    /// and popped by nobody.
    scopes: Vec<Vec<SymbolId>>,
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
                self.declare(function.name);

                self.scopes.push(Vec::new());
                self.parameters(function.ty);
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
    fn declaration(&mut self, declaration: &Declaration, diagnostics: &mut DiagnosticSink) {
        self.walk_type(declaration.ty, diagnostics);
        if let Some(name) = declaration.name {
            self.declare(name);
        }
    }

    /// The names a function declarator wrote between its parentheses.
    ///
    /// Only the outermost type, which is what `driver.rs::dump_parameters`
    /// does as well: a definition whose declarator derives something other than
    /// a function is a constraint violation, and reporting it is #57's rather
    /// than this walk's to invent an answer for.
    fn parameters(&mut self, ty: TypeId) {
        let Type::Function {
            parameters: Parameters::Prototype(parameters),
            ..
        } = self.ast.ty(ty)
        else {
            return;
        };

        for parameter in parameters {
            if let Some(name) = parameter.name {
                self.declare(name);
            }
        }
    }

    /// The expressions inside a type, which is its array lengths.
    ///
    /// A recursion, bounded the way `parser.rs::apply` bounds a declarator's
    /// derivations, so a type is at most `MAX_NESTING` deep.
    ///
    /// A function's parameters are deliberately not walked. `int f(int n, int
    /// a[n])` parses today, and C17 6.7.6.2 lets a parameter's length name
    /// another parameter, whose scope is the prototype rather than anything
    /// this stage builds. Walking it in the enclosing scope would report `n`
    /// as undeclared, which is a false positive about valid C; leaving it
    /// unwalked is silence about a name nobody has yet asked us to check, and
    /// silence is the right way round for this compiler to be wrong.
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
    fn expr(&mut self, root: ExprId, diagnostics: &mut DiagnosticSink) {
        let ast = self.ast;
        let mut pending = vec![root];

        while let Some(id) = pending.pop() {
            let expr = ast.expr(id);

            if let Expr::Identifier { span } = *expr {
                match self.lookup(span) {
                    Some(symbol) => {
                        self.resolution.resolved.insert(span, symbol);
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
    fn lookup(&self, use_site: Span) -> Option<SymbolId> {
        let name = self.sources.snippet(use_site);

        self.scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.iter().rev())
            .find(|&&symbol| self.sources.snippet(self.resolution.symbol(symbol).name) == name)
            .copied()
    }

    /// Add a name to the innermost scope.
    ///
    /// The id is taken before the push, not from `len()` after it, which is the
    /// mistake ADR-0008 records for the tree's arenas and the same one here.
    fn declare(&mut self, name: Span) {
        let id = SymbolId(self.resolution.symbols.len() as u32);
        self.resolution.symbols.push(Symbol { name });
        self.scopes
            .last_mut()
            .expect("the file scope is never popped")
            .push(id);
    }
}

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
    use crate::source::FileId;

    struct Resolved {
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

        /// Where the name used at the `nth` occurrence of `text` was declared.
        fn declaration_of(&self, text: &str, nth: usize) -> Option<Span> {
            let symbol = self.resolution.resolved(self.occurrence(text, nth))?;
            Some(self.resolution.symbol(symbol).name)
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
    /// Mutation: have `Resolver::item` declare a function's name after walking
    /// its body rather than before. `add` stops resolving and this fails.
    /// Mutation: leave the scope pushed for the parameters out. The two
    /// parameters stop resolving and this fails as well.
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
    /// Mutation: drop either `rev` in `Resolver::lookup`. The outer `x`, or the
    /// first declaration in a scope, answers instead, and this fails.
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

    /// C17 6.7.6.2 lets a parameter's length name another parameter, and this
    /// stage builds no scope in which that name could be found. Reporting it
    /// would be a false positive about valid C, so nothing is reported.
    ///
    /// Mutation: walk `Parameters::Prototype` in `Resolver::walk_type`. The
    /// `n` in `a[n]` is looked up in the file scope, is not there, and this
    /// fails with one diagnostic.
    #[test]
    fn a_parameter_may_size_an_array_with_another_parameter() {
        let resolved = resolved("int f(int n, int a[n]) { return 0; }\n");

        assert_eq!(resolved.messages(), Vec::<&str>::new());
    }
}
