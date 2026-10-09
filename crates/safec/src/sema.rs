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

use std::collections::{HashMap, HashSet};

use crate::ast::{
    Ast, Attribute, Declaration, Expr, ExprId, ForStart, InitDeclarator, Item, Parameters, Stmt,
    StmtId, Type, TypeId,
};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::parser::{UNREAD_ANNOTATION, UNREAD_ATTRIBUTE_LABEL};
use safec_ir::source::{SourceMap, Span};

/// A name used where nothing declares it.
///
/// `SC03xx` is names and types. `docs/diagnostics.md` allocates the ranges and
/// this takes the first of that one; the wording follows `clang`, which says
/// `use of undeclared identifier 'x'`, because a user arriving from a C
/// toolchain should not have to learn a second phrasing for the same thing.
const UNDECLARED: Code = Code::new("SC0301");

/// Something given a definition twice: a tag given its content twice in one
/// scope, which C17 6.7.2.3 p1 forbids ("A specific type shall have its
/// content defined at most once"), or a function given two bodies, which 6.9
/// p5 forbids ("there shall be no more than one" external definition).
///
/// One code for both, because it is one class of fault, and the note names
/// the clause that differs. The wording follows `clang`'s `redefinition of
/// 'S'`, for the reason [`UNDECLARED`] gives, but keeps `struct` for a tag: a
/// tag and an ordinary name of one spelling are two names, and a message that
/// dropped the keyword would read the same for either.
const REDEFINED: Code = Code::new("SC0312");

/// One declared name, addressed by [`BindingId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BindingId(u32);

/// One tag, addressed by [`TagId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TagId(u32);

/// A tag the program declared, which is one structure type: C17 6.2.3 puts
/// tags in a name space of their own, and 6.7.2.3 says when two uses of one
/// name are one type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    /// The name as first written, or `None` for a struct with no tag, which
    /// is a type of its own wherever it is written.
    pub name: Option<Span>,
    /// The name as written where the content was given, or `None` if nothing
    /// in the translation unit gives it.
    ///
    /// This is the state after the whole walk, not at any one use: the tag a
    /// use before the definition is bound to, or a member inside it, has this
    /// set too, though C17 6.7.2.1 p8 leaves the type incomplete there until
    /// the closing brace. Whether a type is complete where it is used is not
    /// recorded, and #27, which is the first reader to need it, has to.
    pub defined: Option<Span>,
}

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
    /// Every attribute this stage accepted as a hatch, by where it was written.
    hatches: Vec<Span>,
    tags: Vec<Tag>,
    /// The tag every struct written in the tree is, by the type's id.
    tag_of: HashMap<TypeId, TagId>,
}

impl Resolution {
    /// Whether `attribute` is one this stage accepted as a hatch.
    ///
    /// Asked by the lowering, so that what makes a function a hatch is this
    /// stage's verdict rather than an attribute being present. A refused one
    /// is an error, and the driver lowers nothing after names and types
    /// report one, so today no lowering meets it. What asking still does is
    /// keep a spelling this stage accepts for something else, such as the
    /// effects #249 adds, from making a hatch. See ADR-0038.
    pub fn is_hatch(&self, attribute: Attribute) -> bool {
        self.hatches.contains(&attribute.span)
    }

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

    /// The tag the struct at `ty` is.
    ///
    /// Two struct types are one type exactly when this answers one tag for
    /// both. Nothing asks it yet: the type checker refuses every struct, and
    /// #27, which gives a struct its meaning, is the reader.
    ///
    /// # Panics
    ///
    /// If `ty` is not a struct this stage bound. That includes a struct in
    /// the parameters of a function declarator nested inside another type,
    /// `void f(void (*cb)(struct A *))`, which the walk does not reach yet:
    /// #409. A panic rather than an `Option`, because two answers of `None`
    /// compare equal, and a comparison that called two unrelated structs one
    /// type would be believed.
    pub fn tag(&self, ty: TypeId) -> TagId {
        self.tag_of
            .get(&ty)
            .copied()
            .unwrap_or_else(|| panic!("the struct at {ty:?} was bound to no tag"))
    }

    /// The tag `id` names.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`Resolution`].
    pub fn tag_declaration(&self, id: TagId) -> &Tag {
        &self.tags[id.0 as usize]
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
            hatches: Vec::new(),
            tags: Vec::new(),
            tag_of: HashMap::new(),
        },
        // The file scope, which is never popped.
        scopes: vec![Vec::new()],
        tag_scopes: vec![Vec::new()],
        children: Vec::new(),
        walked: HashSet::new(),
        definitions: Vec::new(),
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
    /// The tags each scope declared, in step with `scopes`: C17 6.2.3 gives
    /// tags a name space of their own and 6.2.1 p4 the same scopes, so the
    /// two are opened and closed together, by [`Resolver::open_scope`].
    tag_scopes: Vec<Vec<TagId>>,
    /// Reused across every expression walk, so that a run allocates once.
    children: Vec<ExprId>,
    /// The struct definitions [`Resolver::walk_type`] has walked, so that each
    /// is walked once: every declarator of `struct S { ... } a, b;` shares the
    /// one definition, and walking it per declarator reported a member's
    /// undeclared length once per declarator and took time exponential in how
    /// deep such declarations nest.
    walked: HashSet<TypeId>,
    /// The name of every function definition so far, in source order, which
    /// is what a second definition of one is compared with. A declaration,
    /// `int f(void);`, is never here: 6.9 p5 counts definitions, and a
    /// prototype before or after its definition is the same function.
    definitions: Vec<Span>,
}

impl Resolver<'_> {
    fn item(&mut self, item: &Item, diagnostics: &mut DiagnosticSink) {
        match item {
            Item::Function(function) => {
                if let Some(attribute) = function.attribute {
                    self.attribute(attribute, diagnostics);
                }
                self.walk_type(function.ty, diagnostics);
                self.define(function.name, diagnostics);
                // Declared before the body is walked, so that a function can
                // call itself. C17 6.2.1 p7 puts the start of a file-scope
                // name at the end of its declarator, which is before the body.
                self.declare(function.name, function.ty);

                self.open_scope();
                self.parameters(function.ty, diagnostics);
                // C17 6.2.1 p4 puts a parameter in the body's outermost block,
                // so the body's statements are walked here, in the parameters'
                // scope, rather than through `stmt`, which would open a second
                // one. It decides whether `int f(struct S *p) { struct S {
                // int a; } s; }` is one tag or two, and whether a struct the
                // parameter list defines and the body defines again is a
                // redefinition. The same holds for ordinary names, where it
                // makes `int f(int a) { int a; }` a redeclaration rather than
                // a shadowing, which this stage does not report: #57 does.
                let ast = self.ast;
                match ast.stmt(function.body) {
                    Stmt::Compound { body, .. } => {
                        for &statement in body {
                            self.stmt(statement, diagnostics);
                        }
                    }
                    _ => self.stmt(function.body, diagnostics),
                }
                self.close_scope();
            }
            Item::Declaration {
                declarators,
                specified,
                ..
            } => {
                self.specified(declarators, *specified, diagnostics);
                self.declarators(declarators, diagnostics);
            }
            Item::Error { .. } => {}
        }
    }

    /// Refuse an attribute that is not `annotate("safec_unchecked")`.
    ///
    /// Here rather than in the parser, which reads the shape and not the text:
    /// [`Attribute`] says why. The string is compared as written, quotes
    /// included, so an escape that spells the same bytes is refused, which is
    /// a refusal the reader can see and not a spelling anybody writes.
    ///
    /// What it accepts is recorded, and [`Resolution::is_hatch`] is the only
    /// thing that makes a function a hatch.
    ///
    /// [`Attribute`]: crate::ast::Attribute
    fn attribute(&mut self, attribute: Attribute, diagnostics: &mut DiagnosticSink) {
        let refused = if self.sources.snippet(attribute.name) != "annotate" {
            attribute.name
        } else if self.sources.snippet(attribute.argument) != "\"safec_unchecked\"" {
            attribute.argument
        } else {
            self.resolution.hatches.push(attribute.span);
            return;
        };

        diagnostics.report(
            Diagnostic::error("safec does not read this attribute")
                .with_code(UNREAD_ANNOTATION)
                .with_label(Label::primary(refused, UNREAD_ATTRIBUTE_LABEL)),
        );
    }

    /// Every declarator of one declaration, in the order they were written.
    ///
    /// One at a time, and each one's name goes in before its own initializer is
    /// walked. That is C17 6.2.1 p7 from the other side of the declarator to
    /// the one [`Resolver::declaration`] cites: a name's scope begins at the
    /// end of its declarator, an array length is inside the declarator and an
    /// initializer is after it. So `int a = a;` resolves to the `a` being
    /// declared, which `clang -std=c17` accepts with a warning rather than an
    /// error, and `int b = a, a = 1;` does not resolve at all, which `clang`
    /// reports as an undeclared identifier. Both were measured.
    ///
    /// Declaring every name first and walking the initializers afterwards would
    /// resolve the second, which is the direction that stays quiet and is the
    /// wrong one here: nothing about it is valid C, and accepting it would make
    /// this compiler the only reader that thinks so.
    fn declarators(&mut self, declarators: &[InitDeclarator], diagnostics: &mut DiagnosticSink) {
        for declarator in declarators {
            self.declaration(&declarator.declaration, diagnostics);
            if let Some(init) = declarator.init {
                self.expr(init, diagnostics);
            }
        }
    }

    /// The type a declaration with no declarator names, which nothing else
    /// walks: `struct S { int a[n]; };` has a length to resolve and no
    /// declarator to reach it through.
    fn specified(
        &mut self,
        declarators: &[InitDeclarator],
        specified: TypeId,
        diagnostics: &mut DiagnosticSink,
    ) {
        if declarators.is_empty() {
            // C17 6.7.2.3 p7: `struct S;` declares `S` in this scope, hiding
            // an outer one, rather than using whatever `S` is visible, which
            // is the only thing that tells it from `struct S *p;`.
            if let Type::Struct {
                tag: Some(tag),
                members: None,
            } = self.ast.ty(specified)
            {
                let found = self.tag_here(*tag);
                let id = found.unwrap_or_else(|| self.declare_tag(Some(*tag), None));
                self.resolution.tag_of.insert(specified, id);
            }
            self.walk_type(specified, diagnostics);
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

        self.open_scope();
        self.parameters(declaration.ty, diagnostics);
        self.close_scope();

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
    /// Only the outermost type, which is what
    /// `driver/dumps.rs::dump_parameters` does as well: a definition whose
    /// declarator derives something other than a function is a constraint
    /// violation, and reporting it is #57's rather than this walk's to invent
    /// an answer for.
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

        // As written, because an array's length is in the type the
        // declarator derived and the adjusted pointer has none: `int a[n]`
        // is `int *` to every other reader, and `n` still has to resolve.
        for parameter in parameters {
            self.walk_type(parameter.written, diagnostics);
        }
    }

    /// The expressions inside a type, which is its array lengths, a struct
    /// member's included.
    ///
    /// An explicit stack rather than a recursion. A type is a declarator's
    /// derivations, which `parser.rs::apply` bounds, and a struct's members,
    /// each a type of its own, which `Parser::deeper` bounds separately, so
    /// the two multiply: `MAX_NESTING` structs each reached through a long
    /// pointer chain overflowed the stack. In the order the recursion visited
    /// them, so the names are resolved, and reported, in source order.
    ///
    /// A function's parameters are not walked here, because they need a scope
    /// of their own and this has none to give: [`Resolver::parameters`] is
    /// where they are declared and their lengths looked up. Doing it here
    /// instead reports `n` in `int f(int n, int a[n])` as undeclared, which is
    /// a false positive about valid C.
    fn walk_type(&mut self, ty: TypeId, diagnostics: &mut DiagnosticSink) {
        let ast = self.ast;
        let mut pending = vec![ty];
        while let Some(ty) = pending.pop() {
            match ast.ty(ty) {
                Type::Int | Type::Char | Type::Void => {}
                Type::Pointer(pointee) => pending.push(*pointee),
                Type::Array {
                    element,
                    length,
                    written: _,
                } => {
                    if let Some(length) = *length {
                        self.expr(length, diagnostics);
                    }
                    pending.push(*element);
                }
                Type::Function { returns, .. } => pending.push(*returns),
                // A member's array lengths are expressions too, walked once
                // per definition, for the reason `walked` gives. Its name is
                // not looked up: what it means is #27's. The tag is, and is
                // bound before its members are walked, because C17 6.2.1 p7
                // starts a tag's scope just after it appears: `struct L {
                // struct L *next; }` is one type.
                Type::Struct { tag, members } => {
                    if !self.resolution.tag_of.contains_key(&ty) {
                        self.bind_tag(ty, *tag, members.is_some(), diagnostics);
                    }
                    if let Some(members) = members {
                        if self.walked.insert(ty) {
                            pending.extend(members.iter().rev().map(|member| member.written));
                        }
                    }
                }
            }
        }
    }

    /// One statement and everything under it.
    ///
    /// A recursion, for the reason `driver/dumps.rs::dump_stmt` gives: every
    /// place a statement nests inside another is a recursion in the parser too,
    /// and `MAX_NESTING` bounds those.
    fn stmt(&mut self, id: StmtId, diagnostics: &mut DiagnosticSink) {
        // Taken out of `self` so that the walk below can borrow `self` mutably.
        // The tree outlives this resolver and is not what is being changed.
        let ast = self.ast;

        match ast.stmt(id) {
            Stmt::Compound { body, .. } => {
                self.open_scope();
                for &statement in body {
                    self.stmt(statement, diagnostics);
                }
                self.close_scope();
            }
            Stmt::Declaration {
                declarators,
                specified,
                ..
            } => {
                self.specified(declarators, *specified, diagnostics);
                self.declarators(declarators, diagnostics);
            }
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
                start,
                condition,
                step,
                body,
                ..
            } => {
                // C17 6.8.5 p5: a declaration here is in scope for the rest of
                // the `for`, the other two clauses and the body included, and
                // nowhere after it. So the scope opens before the declaration
                // and closes after the body, and an expression start needs none.
                let declares = match *start {
                    Some(ForStart::Declaration(declaration)) => {
                        self.open_scope();
                        self.stmt(declaration, diagnostics);
                        true
                    }
                    Some(ForStart::Expression(initialiser)) => {
                        self.expr(initialiser, diagnostics);
                        false
                    }
                    None => false,
                };
                for clause in [*condition, *step].into_iter().flatten() {
                    self.expr(clause, diagnostics);
                }
                self.stmt(*body, diagnostics);
                if declares {
                    self.close_scope();
                }
            }
            Stmt::Error { .. } => {}
        }
    }

    /// Every name under one expression.
    ///
    /// An explicit stack rather than a recursion, because an expression tree
    /// has no bound on its depth: two of the parser's rules fold with a loop,
    /// which is why `driver/dumps.rs::dump_expr` carries its own stack as well.
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

    /// Open a scope for ordinary names and tags alike, which C17 6.2.1 p4
    /// gives the same extent.
    fn open_scope(&mut self) {
        self.scopes.push(Vec::new());
        self.tag_scopes.push(Vec::new());
    }

    /// Close the scope [`Resolver::open_scope`] opened last.
    fn close_scope(&mut self) {
        self.scopes.pop();
        self.tag_scopes.pop();
    }

    /// Bind the struct at `ty` to a tag, by C17 6.7.2.3.
    ///
    /// A definition, `struct S { ... }`, looks in this scope only: a tag
    /// defined here already is a redefinition (p1), one only declared here is
    /// completed, and none here declares a new one, hiding an outer `S`. Any
    /// other use, `struct S *p`, is the innermost visible `S`, or if none is
    /// visible declares a new incomplete one here (p8), which in a parameter
    /// list is the prototype's scope. A struct with no tag is a type of its
    /// own. `struct S;` alone is [`Resolver::specified`]'s.
    fn bind_tag(
        &mut self,
        ty: TypeId,
        tag: Option<Span>,
        defines: bool,
        diagnostics: &mut DiagnosticSink,
    ) {
        let Some(tag) = tag else {
            let id = self.declare_tag(None, None);
            self.resolution.tag_of.insert(ty, id);
            return;
        };

        let id = if defines {
            match self.tag_here(tag) {
                Some(id) => {
                    if let Some(first) = self.resolution.tags[id.0 as usize].defined {
                        let what = format!("struct {}", self.sources.snippet(tag));
                        diagnostics.report(redefined(&what, tag, first, "C17 6.7.2.3 p1"));
                    } else {
                        self.resolution.tags[id.0 as usize].defined = Some(tag);
                    }
                    id
                }
                None => self.declare_tag(Some(tag), Some(tag)),
            }
        } else {
            match self.tag_visible(tag) {
                Some(id) => id,
                None => self.declare_tag(Some(tag), None),
            }
        };
        self.resolution.tag_of.insert(ty, id);
    }

    /// The tag named as `name` is, declared in the innermost scope only.
    fn tag_here(&self, name: Span) -> Option<TagId> {
        let scope = self
            .tag_scopes
            .last()
            .expect("the file scope is never popped");
        self.tag_in(scope, name)
    }

    /// The innermost visible tag named as `name` is.
    fn tag_visible(&self, name: Span) -> Option<TagId> {
        self.tag_scopes
            .iter()
            .rev()
            .find_map(|scope| self.tag_in(scope, name))
    }

    fn tag_in(&self, scope: &[TagId], name: Span) -> Option<TagId> {
        let spelled = self.sources.snippet(name);
        scope
            .iter()
            .rev()
            .find(|&&id| {
                self.resolution.tags[id.0 as usize]
                    .name
                    .is_some_and(|written| self.sources.snippet(written) == spelled)
            })
            .copied()
    }

    /// Add a tag to the innermost scope, or to none for a struct with no tag,
    /// which nothing can name.
    fn declare_tag(&mut self, name: Option<Span>, defined: Option<Span>) -> TagId {
        let id = TagId(self.resolution.tags.len() as u32);
        self.resolution.tags.push(Tag { name, defined });
        if name.is_some() {
            self.tag_scopes
                .last_mut()
                .expect("the file scope is never popped")
                .push(id);
        }
        id
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

    /// Record the definition of the function named at `name`, or report it
    /// as the second definition of that name (C17 6.9 p5).
    ///
    /// The second is still declared and its body still walked by the caller,
    /// so that a name inside it that is undeclared is reported as well, as it
    /// would be in a body nothing was wrong with.
    fn define(&mut self, name: Span, diagnostics: &mut DiagnosticSink) {
        let spelled = self.sources.snippet(name);
        let first = self
            .definitions
            .iter()
            .find(|&&first| self.sources.snippet(first) == spelled);
        match first {
            Some(&first) => diagnostics.report(redefined(spelled, name, first, "C17 6.9 p5")),
            None => self.definitions.push(name),
        }
    }
}

/// `what`, defined at `again`, which was defined at `first` where `clause`
/// says it can be defined only once: `struct S` for a tag, and the name for a
/// function.
///
/// `what` is interpolated raw, for the reason [`undeclared`] gives.
fn redefined(what: &str, again: Span, first: Span, clause: &str) -> Diagnostic {
    Diagnostic::error(format!("redefinition of `{what}`"))
        .with_code(REDEFINED)
        .with_label(Label::primary(again, "defined again here"))
        .with_label(Label::secondary(first, "first defined here"))
        .with_note(clause)
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

        /// The tag the struct whose tag is the `nth` written `name` is bound to.
        fn tag_at(&self, name: &str, nth: usize) -> TagId {
            let at = self.occurrence(name, nth);
            let ty = self
                .ast
                .type_ids()
                .find(|&ty| matches!(self.ast.ty(ty), Type::Struct { tag: Some(tag), .. } if *tag == at))
                .unwrap_or_else(|| panic!("no struct's tag is written at {at:?}"));
            self.resolution.tag(ty)
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
    /// Mutation: remove the `self.close_scope()` that closes the parameter
    /// scope in `Resolver::item`. `a` is still in scope inside `g`, nothing is
    /// reported, and this fails. Before it existed, that `pop` was held by
    /// nothing in the suite: every other program declares one function.
    #[test]
    fn a_parameter_does_not_reach_the_function_after_it() {
        let resolved =
            resolved("int f(int a) {\n    return a;\n}\n\nint g(void) {\n    return a;\n}\n");

        assert_eq!(resolved.messages(), ["use of undeclared identifier `a`"]);
    }

    /// A tag and an ordinary name of one spelling are two names (C17 6.2.3),
    /// in one scope, and a use of the name is the variable.
    ///
    /// The variable is declared first, so that a tag kept among the ordinary
    /// names, being the later declaration, is what an innermost-first lookup
    /// would find.
    ///
    /// Mutation: keep tags in the ordinary table as well, by declaring each
    /// new tag through `declare`; the use of `S` resolves to the tag, the
    /// second `S` written, and this fails.
    #[test]
    fn a_tag_and_a_variable_of_one_name_are_two_names() {
        let resolved = resolved("int S;\nstruct S { int x; };\nint f(void) {\n    return S;\n}\n");

        assert_eq!(resolved.messages(), Vec::<&str>::new());
        assert_eq!(
            resolved.declaration_of("S", 2),
            Some(resolved.occurrence("S", 0)),
            "the use is the variable"
        );
        let _ = resolved.tag_at("S", 1);
    }

    /// A tag defined in a block binds nothing after the block: the `struct T`
    /// written after it declares a new, incomplete `T` (C17 6.7.2.3 p8).
    ///
    /// Mutation: leave the tag scope open at a block's end, by popping only
    /// `scopes` in `close_scope`; the outer use finds the inner `T` and this
    /// fails on the two being one.
    #[test]
    fn an_inner_tag_binds_no_use_after_its_block() {
        let resolved = resolved(
            "int f(void) {\n    {\n        struct T { int y; } inner;\n    }\n    struct T *outer;\n    return 0;\n}\n",
        );

        assert_ne!(resolved.tag_at("T", 0), resolved.tag_at("T", 1));
    }

    /// A use before the definition, with nothing visible, declares the tag
    /// the definition then completes: one type (C17 6.7.2.3 p8 and p4).
    ///
    /// Mutation: declare a new tag at every definition, ignoring one already
    /// declared in the scope; two tags, and this fails.
    #[test]
    fn a_tag_used_before_its_definition_is_the_tag_it_defines() {
        let resolved = resolved("struct U *p;\nstruct U { int a; };\n");

        assert_eq!(resolved.tag_at("U", 0), resolved.tag_at("U", 1));
        assert_eq!(resolved.messages(), Vec::<&str>::new());
    }

    /// A struct naming itself in its own members is one type, because a tag's
    /// scope begins just after it appears (C17 6.2.1 p7).
    ///
    /// Mutation: leave a definition's tag out of the scope until its members
    /// are walked, by declaring it in no scope; the member's `struct L`
    /// finds nothing, declares a second `L`, and this fails.
    #[test]
    fn a_struct_that_names_itself_is_one_type() {
        let resolved = resolved("struct L { struct L *next; };\n");

        assert_eq!(resolved.tag_at("L", 0), resolved.tag_at("L", 1));
    }

    /// `struct S;` alone declares `S` in its own scope and hides an outer one
    /// (C17 6.7.2.3 p7), so a use after it in that block is the inner `S`.
    ///
    /// Mutation: treat `struct S;` as a use, in `Resolver::specified`; it
    /// finds the outer `S`, and so does the use, and this fails.
    #[test]
    fn a_declaration_of_a_tag_alone_hides_an_outer_one() {
        let resolved = resolved(
            "struct S { int a; };\nint f(void) {\n    struct S;\n    struct S *p;\n    return 0;\n}\n",
        );

        assert_ne!(resolved.tag_at("S", 0), resolved.tag_at("S", 1));
        assert_eq!(resolved.tag_at("S", 1), resolved.tag_at("S", 2));
    }

    /// A tag given its content twice in one scope is reported (C17 6.7.2.3
    /// p1), and given it again in an inner scope is a new tag and is not.
    ///
    /// Mutation: look a definition up in every visible scope rather than this
    /// one; the inner `W` is reported as a redefinition, and this fails.
    #[test]
    fn a_tag_defined_twice_in_one_scope_is_reported_and_in_two_is_not() {
        let twice = resolved("struct V { int a; };\nstruct V { int b; };\n");
        assert_eq!(twice.messages(), ["redefinition of `struct V`"]);

        let nested = resolved(
            "struct W { int a; };\nint g(void) {\n    struct W { int b; } w;\n    return 0;\n}\n",
        );
        assert_eq!(nested.messages(), Vec::<&str>::new());
        assert_ne!(nested.tag_at("W", 0), nested.tag_at("W", 1));
    }

    /// A tag a definition's parameter list declares is in the body's
    /// outermost block (C17 6.2.1 p4), so the body's `struct S` completes it
    /// rather than declaring a second `S`, and defining it in both is a
    /// redefinition.
    ///
    /// Mutation: walk the body through `self.stmt(function.body, ..)` in
    /// `Resolver::item`, which opens a second scope; the two `S` are two tags
    /// and the redefinition goes unreported, and this fails.
    #[test]
    fn a_tag_in_a_definitions_parameters_is_in_its_body() {
        let completed =
            resolved("int f(struct S *p) {\n    struct S { int a; } s;\n    return 0;\n}\n");
        assert_eq!(completed.messages(), Vec::<&str>::new());
        assert_eq!(completed.tag_at("S", 0), completed.tag_at("S", 1));

        let twice = resolved(
            "int f(struct S { int a; } x) {\n    struct S { int b; } y;\n    return 0;\n}\n",
        );
        assert_eq!(twice.messages(), ["redefinition of `struct S`"]);
    }

    /// A tag first written in a prototype's parameters is declared in that
    /// prototype's scope (C17 6.7.2.3 p8), which ends with the declarator,
    /// so a definition after it is a different tag.
    ///
    /// Mutation: open only the ordinary scope around the parameters in
    /// `Resolver::declaration`, by `self.scopes.push(Vec::new())` and
    /// `self.scopes.pop()`; the prototype's `S` lands at file scope, the
    /// definition completes it, and this fails.
    #[test]
    fn a_tag_first_written_in_a_prototype_is_not_the_one_defined_after_it() {
        let resolved = resolved("void f(struct S *p);\nstruct S { int a; };\n");

        assert_eq!(resolved.messages(), Vec::<&str>::new());
        assert_ne!(resolved.tag_at("S", 0), resolved.tag_at("S", 1));
    }

    /// A use in a block finds a tag declared in a scope outside it (C17
    /// 6.7.2.3 p8 only declares a new one when none is visible).
    ///
    /// Mutation: look a use up in the innermost scope only, by making
    /// `tag_visible` return `self.tag_here(name)`; the use in `f` declares a
    /// second `S`, and this fails.
    #[test]
    fn a_use_in_a_block_is_the_tag_declared_outside_it() {
        let resolved =
            resolved("struct S { int a; };\nint f(void) {\n    struct S *p;\n    return 0;\n}\n");

        assert_eq!(resolved.tag_at("S", 0), resolved.tag_at("S", 1));
    }

    /// `struct S;` where `S` is declared already in the same scope declares
    /// nothing new (C17 6.7.2.3 p4): one type, before and after it.
    ///
    /// Mutation: declare a new tag at every `struct S;`, by replacing
    /// `self.tag_here(*tag)` in `Resolver::specified` with `None`; the `S`
    /// after it is a second tag, and this fails.
    #[test]
    fn a_declaration_of_a_tag_alone_after_its_definition_is_that_tag() {
        let resolved = resolved("struct S { int a; };\nstruct S;\nstruct S *p;\n");

        assert_eq!(resolved.tag_at("S", 0), resolved.tag_at("S", 1));
        assert_eq!(resolved.tag_at("S", 0), resolved.tag_at("S", 2));
    }

    /// A tag completed after a use records where, so a third `struct U { }`
    /// is a redefinition, and [`Resolution::tag_declaration`] answers for
    /// the tag asked about. `T` comes first so that `U` is not the first tag.
    ///
    /// Mutations: leave `defined` unset when a definition completes a tag in
    /// `bind_tag`, and the redefinition is not reported; answer `&self.tags[0]`
    /// in `tag_declaration`, and `T`'s span comes back. Both fail this.
    #[test]
    fn a_tag_completed_after_a_use_is_defined_there() {
        let resolved = resolved(
            "struct T { int t; };\nstruct U *p;\nstruct U { int a; };\nstruct U { int b; };\n",
        );

        assert_eq!(resolved.messages(), ["redefinition of `struct U`"]);
        let tag = resolved.resolution.tag_declaration(resolved.tag_at("U", 0));
        assert_eq!(tag.name, Some(resolved.occurrence("U", 0)));
        assert_eq!(tag.defined, Some(resolved.occurrence("U", 1)));
    }

    /// Every struct with no tag is a type of its own, since nothing can name
    /// it again (C17 6.7.2.3 p5).
    ///
    /// Mutation: bind every struct with no tag after the first to `TagId(0)`
    /// in `bind_tag`; `x` and `y` are one type, and this fails.
    #[test]
    fn structs_with_no_tag_are_each_a_type_of_their_own() {
        let resolved = resolved("struct { int a; } x;\nstruct { int b; } y;\n");

        let untagged: Vec<TagId> = resolved
            .ast
            .type_ids()
            .filter(|&ty| matches!(resolved.ast.ty(ty), Type::Struct { tag: None, .. }))
            .map(|ty| resolved.resolution.tag(ty))
            .collect();
        assert_eq!(untagged.len(), 2);
        assert_ne!(untagged[0], untagged[1]);
    }

    /// A struct the walk did not bind is a panic naming it, not an answer
    /// that compares equal to another unbound struct's: the struct in `cb`'s
    /// parameters is one such today.
    ///
    /// Mutation: answer `.unwrap_or(TagId(0))` in `Resolution::tag`; nothing
    /// panics, and this fails.
    #[test]
    #[should_panic(expected = "was bound to no tag")]
    fn a_struct_bound_to_no_tag_is_a_panic_and_not_an_answer() {
        let resolved = resolved("void f(void (*cb)(struct A *));\n");

        let unbound = resolved
            .ast
            .type_ids()
            .find(|&ty| matches!(resolved.ast.ty(ty), Type::Struct { .. }))
            .expect("the struct is in the tree");
        let _ = resolved.resolution.tag(unbound);
    }

    /// A function given a second body is reported once, at the second, and
    /// only the definitions are counted (C17 6.9 p5): a prototype before and
    /// after the definition is the same function.
    ///
    /// Mutation: drop the call to `define` in `Resolver::item`; nothing is
    /// reported and the first assertion fails. Mutation: record every
    /// declaration as a definition too, by calling `self.define(name, ..)`
    /// in `Resolver::declaration`; the definition after the prototype is
    /// reported and the second fails.
    #[test]
    fn a_function_defined_twice_is_reported_and_one_declared_twice_is_not() {
        let twice = resolved("int f(void) {\n    return 1;\n}\nint f(void) {\n    return 2;\n}\n");
        assert_eq!(twice.messages(), ["redefinition of `f`"]);

        let declared = resolved("int g(void);\nint g(void) {\n    return 1;\n}\nint g(void);\n");
        assert_eq!(declared.messages(), Vec::<&str>::new());
    }
}
