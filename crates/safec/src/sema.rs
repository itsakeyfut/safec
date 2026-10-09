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
    StmtId, Type, TypeId, spell_type,
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
/// content defined at most once"), or a function given two bodies.
///
/// The second is not a constraint for a function with external linkage. 6.9
/// p5 is a semantics rule, exactly one external definition where the name is
/// used and no more than one where it is not, so breaking it is undefined
/// rather than a diagnostic C requires. It is refused all the same, as
/// `clang` refuses it: a program with two bodies for one function has no
/// meaning to analyse. 6.9 p3 is the constraint, for internal linkage, which
/// this compiler does not read yet.
///
/// One code for both, because it is one class of fault, and the note names
/// the clause that differs. The wording follows `clang`'s `redefinition of
/// 'S'`, for the reason [`UNDECLARED`] gives, but keeps `struct` for a tag: a
/// tag and an ordinary name of one spelling are two names, and a message that
/// dropped the keyword would read the same for either.
const REDEFINED: Code = Code::new("SC0312");

/// Two declarations of one name in one scope whose types are not compatible,
/// which C17 6.7 p4 forbids: "All declarations in the same scope that refer
/// to the same object or function shall specify compatible types."
///
/// Its own code rather than [`REDEFINED`]'s: one thing given twice and two
/// declarations that disagree about what a thing is are different faults,
/// and a reader filtering on a code should see them apart. The wording is
/// `clang`'s `conflicting types for 'f'`, for the reason [`UNDECLARED`]
/// gives.
const CONFLICTING: Code = Code::new("SC0313");

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
    /// Every binding whose name's standing declaration, when its scope
    /// closed, is another binding. See [`Resolution::standing`].
    standing: HashMap<BindingId, BindingId>,
    /// The standing type of every file-scope name, by its spelling.
    declared: HashMap<String, TypeId>,
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

    /// The declaration that says what `id`'s name is: the standing one its
    /// scope ended with, which stands in for the composite type C17 6.2.7 p3
    /// gives a name declared more than once (see `Declared`), or `id` itself
    /// where nothing else stands for it.
    ///
    /// The type checker types a function's name by this rather than by the
    /// declaration the use resolved to, and the lowering builds a function
    /// from it, so that the two agree on what a function is: `int f(); int
    /// f(int *a);` is a function of one parameter to both, wherever a call
    /// to it is written. For a function at file scope that is the
    /// translation unit's last word, and for one declared in a block the
    /// block's, so a call written before the prototype is held to it too:
    /// stricter than C, which checks a call against the declaration in scope,
    /// and recorded in `docs/frontend.md`. An object is typed by the
    /// declaration in scope, as C types it, because what a pointer to a
    /// function holds is whatever was assigned to it.
    pub fn standing(&self, id: BindingId) -> BindingId {
        self.standing.get(&id).copied().unwrap_or(id)
    }

    /// The type a file-scope name was given by its standing declaration, or
    /// `None` if no file-scope declaration has that spelling.
    ///
    /// By the text, for the lowering, which keys a function by its name
    /// because a prototype and its definition are one function at two spans.
    pub fn declared(&self, name: &str) -> Option<TypeId> {
        self.declared.get(name).copied()
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
            standing: HashMap::new(),
            declared: HashMap::new(),
        },
        // The file scope, which is never popped.
        scopes: vec![Vec::new()],
        tag_scopes: vec![Vec::new()],
        spelled: vec![HashMap::new()],
        children: Vec::new(),
        walked: HashSet::new(),
        definitions: HashMap::new(),
    };

    for item in ast.items() {
        resolver.item(item, diagnostics);
    }

    // The file scope is never popped, so it is settled here, and its names'
    // standing types are what the lowering reads.
    resolver.settle();
    let file = resolver
        .spelled
        .last()
        .expect("the file scope is never popped");
    for (name, declared) in file {
        let ty = resolver.resolution.binding(declared.standing.id).ty;
        resolver.resolution.declared.insert(name.clone(), ty);
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
    /// The declarations of each spelling, per scope and in step with
    /// `scopes`, which is what a second declaration in one scope is compared
    /// with. A map rather than a scan of the innermost scope, which would
    /// make a block of many declarations quadratic to resolve.
    spelled: Vec<HashMap<String, Declared>>,
    /// Reused across every expression walk, so that a run allocates once.
    children: Vec<ExprId>,
    /// The struct definitions [`Resolver::walk_type`] has walked, so that each
    /// is walked once: every declarator of `struct S { ... } a, b;` shares the
    /// one definition, and walking it per declarator reported a member's
    /// undeclared length once per declarator and took time exponential in how
    /// deep such declarations nest.
    walked: HashSet<TypeId>,
    /// Where every function defined so far was named, by its spelling, which
    /// is what a second definition of one is compared with. A declaration,
    /// `int f(void);`, is never here: 6.9 p5 counts definitions, and a
    /// prototype before or after its definition is the same function.
    ///
    /// A map rather than a list scanned per definition, which made a file of
    /// forty thousand distinct functions take thirty seconds in a debug
    /// build.
    definitions: HashMap<String, Span>,
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
                self.declare(function.name, function.ty, true, diagnostics);

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
                // a shadowing, and `declare` reports it.
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
            self.declare(name, declaration.ty, false, diagnostics);
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
    /// violation, and not this walk's to invent an answer for. The lowering
    /// reports it when nothing before it has: a body that uses a parameter is
    /// reported here first, as using an undeclared name, because none was
    /// declared, and the lowering is then never reached.
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
                self.declare(name, parameter.ty, false, diagnostics);
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
        self.spelled.push(HashMap::new());
    }

    /// Close the scope [`Resolver::open_scope`] opened last.
    fn close_scope(&mut self) {
        self.settle();
        self.scopes.pop();
        self.tag_scopes.pop();
        self.spelled.pop();
    }

    /// Record, for every binding of the innermost scope, the standing
    /// declaration its name ended the scope with. See
    /// [`Resolution::standing`].
    fn settle(&mut self) {
        let scope = self.scopes.last().expect("the file scope is never popped");
        let spelled = self.spelled.last().expect("the file scope is never popped");
        let settled: Vec<(BindingId, BindingId)> = scope
            .iter()
            .filter_map(|&id| {
                let name = self.sources.snippet(self.resolution.binding(id).name);
                let standing = spelled
                    .get(name)
                    .expect("every binding of a scope is declared in it")
                    .standing
                    .id;
                (standing != id).then_some((id, standing))
            })
            .collect();
        self.resolution.standing.extend(settled);
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
                        diagnostics.report(redefined(
                            &what,
                            tag,
                            first,
                            "defined",
                            "C17 6.7.2.3 p1",
                        ));
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

    /// Add a name to the innermost scope, reporting it if that scope already
    /// declares it and C17 6.7 p3 forbids the second: "If an identifier has
    /// no linkage, there shall be no more than one declaration of the
    /// identifier ... with the same scope and in the same name space".
    ///
    /// Outside file scope only, because every name there has linkage while
    /// `static` and `extern` are not read: `int x; int x;` at file scope is
    /// two tentative definitions of one object. Inside a block, a function
    /// declaration has external linkage and an object or a parameter has
    /// none, so two declarations are allowed only when both are of functions.
    /// The second is declared all the same, so a use after it resolves as
    /// before.
    ///
    /// Two declarations that are allowed refer to one entity, and 6.7 p4
    /// wants their types compatible. The new one is compared with the name's
    /// standing declaration, see [`Declared`], and a type that reaches a
    /// struct is not compared, see [`reaches_a_struct`].
    ///
    /// The id is taken before the push, not from `len()` after it, which is the
    /// mistake ADR-0008 records for the tree's arenas and the same one here.
    fn declare(
        &mut self,
        name: Span,
        ty: TypeId,
        definition: bool,
        diagnostics: &mut DiagnosticSink,
    ) {
        let spelled = self.sources.snippet(name);
        let here = self.spelled.last().expect("the file scope is never popped");
        let earlier = here.get(spelled).copied();
        let mut conflicts = false;
        if let Some(earlier) = earlier {
            let function = |ty| matches!(self.ast.ty(ty), Type::Function { .. });
            let latest = self.resolution.binding(earlier.latest);
            let standing = self.resolution.binding(earlier.standing.id);
            if self.scopes.len() > 1 && !(function(latest.ty) && function(ty)) {
                diagnostics.report(redefined(
                    spelled,
                    name,
                    latest.name,
                    "declared",
                    "C17 6.7 p3",
                ));
            } else if !earlier.standing.reaches_a_struct
                && !reaches_a_struct(self.ast, ty)
                && !self.agrees(earlier.standing, ty, definition)
            {
                conflicts = true;
                diagnostics.report(conflicting(
                    self.sources,
                    self.ast,
                    spelled,
                    (name, ty),
                    (standing.name, standing.ty),
                ));
            }
        }

        let id = BindingId(self.resolution.bindings.len() as u32);
        self.resolution.bindings.push(Binding { name, ty });
        self.scopes
            .last_mut()
            .expect("the file scope is never popped")
            .push(id);
        // A declaration that conflicts is not what the next one is compared
        // with, so a later line that agrees with the earlier ones is not
        // blamed for the one that did not, which is how `clang` reads it too.
        // Nor is one that says less than the standing declaration already
        // does: no prototype, after a prototype or after a definition with
        // an empty list, which fixes the count at none.
        let standing = match earlier {
            Some(earlier)
                if conflicts
                    || ((has_a_prototype(
                        self.ast,
                        self.resolution.binding(earlier.standing.id).ty,
                    ) || earlier.standing.defined_without_a_prototype)
                        && !has_a_prototype(self.ast, ty)) =>
            {
                earlier.standing
            }
            _ => self.standing(id, ty, definition),
        };
        self.spelled
            .last_mut()
            .expect("the file scope is never popped")
            .insert(
                spelled.to_owned(),
                Declared {
                    latest: id,
                    standing,
                },
            );
    }

    /// What [`Standing`] records of the declaration `id`, worked out once
    /// rather than at every later declaration it is compared with.
    fn standing(&self, id: BindingId, ty: TypeId, definition: bool) -> Standing {
        let accepts_no_prototype = match self.ast.ty(called(self.ast, ty)) {
            Type::Function {
                parameters: Parameters::Prototype(parameters),
                ..
            } => parameters
                .iter()
                .all(|parameter| !self.ast.is_promoted_by_default(parameter.ty)),
            _ => true,
        };
        Standing {
            id,
            reaches_a_struct: reaches_a_struct(self.ast, ty),
            accepts_no_prototype,
            defined_without_a_prototype: definition && !has_a_prototype(self.ast, ty),
        }
    }

    /// Whether a declaration of type `ty` agrees with the `standing` one,
    /// C17 6.7 p4 through 6.2.7 and 6.7.6.3 p15.
    ///
    /// [`Ast::compatible`], except in the two places 6.7.6.3 p15 tells a
    /// function definition apart from a declaration, which `compatible`
    /// cannot see: a definition with an empty identifier list fixes the
    /// count of parameters at none, so it agrees only with a prototype that
    /// has none, in either order. A declaration with no prototype against a
    /// standing prototype is answered from what [`Standing`] recorded,
    /// rather than by walking the prototype's parameters again, so that one
    /// long prototype followed by many `int f();` stays linear.
    fn agrees(&self, standing: Standing, ty: TypeId, definition: bool) -> bool {
        let earlier = self.resolution.binding(standing.id).ty;
        let (left, right) = (called(self.ast, earlier), called(self.ast, ty));
        match (self.ast.ty(left), self.ast.ty(right)) {
            (
                Type::Function {
                    returns: left_returns,
                    parameters: Parameters::Prototype(prototype),
                },
                Type::Function {
                    returns: right_returns,
                    parameters: Parameters::Unspecified,
                },
            ) if pointers(self.ast, earlier) == pointers(self.ast, ty) => {
                self.ast.compatible(*left_returns, *right_returns)
                    && if definition {
                        prototype.is_empty()
                    } else {
                        standing.accepts_no_prototype
                    }
            }
            (
                Type::Function {
                    parameters: Parameters::Unspecified,
                    ..
                },
                Type::Function {
                    parameters: Parameters::Prototype(prototype),
                    ..
                },
            ) if standing.defined_without_a_prototype => {
                prototype.is_empty() && self.ast.compatible(earlier, ty)
            }
            _ => self.ast.compatible(earlier, ty),
        }
    }

    /// Record the definition of the function named at `name`, or report it
    /// as the second definition of that name (C17 6.9 p5).
    ///
    /// The second is still declared and its body still walked by the caller,
    /// so that a name inside it that is undeclared is reported as well, as it
    /// would be in a body nothing was wrong with.
    fn define(&mut self, name: Span, diagnostics: &mut DiagnosticSink) {
        let spelled = self.sources.snippet(name);
        match self.definitions.get(spelled) {
            Some(&first) => {
                diagnostics.report(redefined(spelled, name, first, "defined", "C17 6.9 p5"))
            }
            None => {
                self.definitions.insert(spelled.to_owned(), name);
            }
        }
    }
}

/// `what`, written at `again`, which was written at `first` where `clause`
/// says it can be written only once: `struct S` for a tag, and the name for a
/// function, an object or a parameter.
///
/// `verb` is what the labels say happened at each, "defined" or "declared",
/// because `int h(void);` and a parameter of a prototype are declarations,
/// and a label calling them definitions would be false about the program.
/// The headline is `clang`'s for every one of them.
///
/// `what` is interpolated raw, for the reason [`undeclared`] gives.
fn redefined(what: &str, again: Span, first: Span, verb: &str, clause: &str) -> Diagnostic {
    Diagnostic::error(format!("redefinition of `{what}`"))
        .with_code(REDEFINED)
        .with_label(Label::primary(again, format!("{verb} again here")))
        .with_label(Label::secondary(first, format!("previously {verb} here")))
        .with_note(clause)
}

/// The declarations of one spelling in one scope that a new one is compared
/// with.
///
/// `standing` stands in for the composite type C17 6.2.7 p3 gives a name
/// declared more than once, without building it: it is the most recent
/// declaration that agreed and said the most, a prototype over none, and a
/// definition with an empty list over a declaration with one, since that
/// fixes the count at none. A declaration without a prototype otherwise adds
/// nothing a later one can disagree with beyond its return type, which is
/// compared all the same. So `int f(); int f(char *a); int f(); int f(int
/// *a);` compares the fourth with the second and reports it, where comparing
/// with the third would not. A declaration that conflicts does not become
/// it. An object of pointer-to-function type is read through its pointers,
/// `int (*p)(int);` as the prototype it points at. The parameters of a
/// parameter of function type are not composed in turn, which no program
/// this compiler can build tells apart.
#[derive(Clone, Copy)]
struct Declared {
    /// The most recent, which a second declaration of a name with no linkage
    /// is reported against.
    latest: BindingId,
    standing: Standing,
}

/// The standing declaration of a name, with what every later comparison
/// asks of it, so that asking costs nothing more than once.
#[derive(Clone, Copy)]
struct Standing {
    id: BindingId,
    /// See [`reaches_a_struct`].
    reaches_a_struct: bool,
    /// Whether a declaration with no prototype agrees with this one's
    /// parameters, which by C17 6.7.6.3 p15 is whether none of them is
    /// changed by the default argument promotions. True where this has no
    /// prototype.
    accepts_no_prototype: bool,
    /// Whether this is a function definition with an empty identifier list,
    /// `int f() { ... }`, which 6.7.6.3 p15 holds to a count of none.
    defined_without_a_prototype: bool,
}

/// The type `ty` names once its pointers are read through: the function an
/// object of pointer-to-function type points at, or `ty` itself.
fn called(ast: &Ast, ty: TypeId) -> TypeId {
    let mut current = ty;
    while let Type::Pointer(inner) = ast.ty(current) {
        current = *inner;
    }
    current
}

/// How many pointers `ty` is read through by [`called`].
fn pointers(ast: &Ast, ty: TypeId) -> usize {
    let mut count = 0;
    let mut current = ty;
    while let Type::Pointer(inner) = ast.ty(current) {
        current = *inner;
        count += 1;
    }
    count
}

/// Whether `ty`, read through its pointers, is a function type with a
/// parameter type list.
fn has_a_prototype(ast: &Ast, ty: TypeId) -> bool {
    matches!(
        ast.ty(called(ast, ty)),
        Type::Function {
            parameters: Parameters::Prototype(_),
            ..
        }
    )
}

/// Whether `ty` reaches a struct: along its pointers, arrays and return type
/// as `types.rs`'s `holds_a_struct` walks, and also through a prototype's
/// parameters, because [`Ast::compatible`] compares those too.
///
/// A type that does is not compared, because `compatible` answers every pair
/// of structs incompatible until #27 gives a tag its meaning, and comparing
/// would report `struct S *p; struct S *p;`, which is valid C. #27 removes
/// this along with the struct gate. A recursion only into parameter lists,
/// which the parser bounds.
fn reaches_a_struct(ast: &Ast, ty: TypeId) -> bool {
    let mut current = ty;
    loop {
        match ast.ty(current) {
            Type::Struct { .. } => return true,
            Type::Pointer(inner) | Type::Array { element: inner, .. } => current = *inner,
            Type::Function {
                returns,
                parameters,
            } => {
                if let Parameters::Prototype(parameters) = parameters {
                    if parameters
                        .iter()
                        .any(|parameter| reaches_a_struct(ast, parameter.ty))
                    {
                        return true;
                    }
                }
                current = *returns;
            }
            Type::Int | Type::Char | Type::Void => return false,
        }
    }
}

/// `name`, declared at `again` as one type, where it was declared at `first`
/// as another that is not compatible with it.
///
/// Both types are spelled, which is the whole of what the reader needs to see
/// why: `int (void)` beside `char (void)`. `name` is interpolated raw, for
/// the reason [`undeclared`] gives.
fn conflicting(
    sources: &SourceMap,
    ast: &Ast,
    name: &str,
    again: (Span, TypeId),
    first: (Span, TypeId),
) -> Diagnostic {
    let spelled = |ty| spell_type(sources, ast, ty);
    Diagnostic::error(format!("conflicting types for `{name}`"))
        .with_code(CONFLICTING)
        .with_label(Label::primary(
            again.0,
            format!("declared here as `{}`", spelled(again.1)),
        ))
        .with_label(Label::secondary(
            first.0,
            format!("previously declared as `{}`", spelled(first.1)),
        ))
        .with_note("C17 6.7 p4")
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

        /// Where the standing declaration of what the `nth` occurrence of
        /// `text` resolves to names it. See [`Resolution::standing`].
        fn standing_of(&self, text: &str, nth: usize) -> Option<Span> {
            let use_site = self.used_at(self.occurrence(text, nth));
            let binding = self.resolution.resolved(use_site)?;
            Some(
                self.resolution
                    .binding(self.resolution.standing(binding))
                    .name,
            )
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
    /// The other `rev`, over the bindings within one scope, decides which of
    /// two declarations of one name in one scope answers, and
    /// `a_use_after_a_name_declared_twice_is_the_second_declaration` holds
    /// it. At file scope two compatible declarations can still differ, `int
    /// f(); int f(int *a);`, and a call after them is checked against the
    /// second, which is the standing declaration, so `f()` there is too few
    /// arguments wherever the call is written; see `Resolution::standing`.
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

    /// Every definition is remembered, not only the first, and the second
    /// body is still read (C17 6.9 p5): a function defined after another and
    /// then again is reported, and an undeclared name in its second body is
    /// reported too.
    ///
    /// Mutation: remember only the first definition, by inserting in
    /// `Resolver::define` only while `definitions` is empty; `b` is not
    /// reported and this fails. Mutation: return from `Resolver::item`'s
    /// function arm after `define` when it reports; `zzz` is not reported
    /// and this fails.
    #[test]
    fn a_later_function_defined_twice_is_reported_and_its_second_body_is_read() {
        let resolved = resolved(
            "int a(void) {\n    return 1;\n}\nint b(void) {\n    return 2;\n}\nint b(void) {\n    return zzz;\n}\n",
        );

        assert_eq!(
            resolved.messages(),
            ["redefinition of `b`", "use of undeclared identifier `zzz`"]
        );
    }

    /// A name with no linkage declared twice in one scope is reported (C17
    /// 6.7 p3): two locals in a block, a local in a definition's body beside
    /// its parameter, and two parameters of the declared function's own
    /// prototype. A prototype nested inside a type, a function pointer's, is
    /// not walked yet: #409.
    ///
    /// Mutation: drop the report in `Resolver::declare`; nothing is reported
    /// and this fails.
    #[test]
    fn a_name_with_no_linkage_declared_twice_in_one_scope_is_reported() {
        let block = resolved("int main(void) {\n    int x;\n    int x;\n    return 0;\n}\n");
        assert_eq!(block.messages(), ["redefinition of `x`"]);

        let parameter = resolved("int f(int a) {\n    int a;\n    return 0;\n}\n");
        assert_eq!(parameter.messages(), ["redefinition of `a`"]);

        let prototype = resolved("void f(int a, int a);\n");
        assert_eq!(prototype.messages(), ["redefinition of `a`"]);
    }

    /// Two declarations of a function in one block are allowed, since a
    /// function declared there has external linkage, and a function and an
    /// object of one name there are not.
    ///
    /// Both orders of a function and an object are checked.
    ///
    /// Mutation: drop the both-functions exception in `Resolver::declare`;
    /// the two declarations of `g` are reported and this fails. Mutation:
    /// exempt a pair when either is a function; `h` and `k` are not reported.
    /// Mutation: ask only whether the second is a function; `k`, an object
    /// and then a function, is not reported. Either fails this.
    #[test]
    fn a_function_declared_twice_in_a_block_is_allowed_and_beside_an_object_is_not() {
        let resolved = resolved(
            "int main(void) {\n    int g(void);\n    int g(void);\n    int h(void);\n    int h;\n    int k;\n    int k(void);\n    return 0;\n}\n",
        );

        assert_eq!(
            resolved.messages(),
            ["redefinition of `h`", "redefinition of `k`"]
        );
    }

    /// File scope is not checked, since every name there has linkage, and a
    /// declaration in an inner block hides an outer one rather than repeating
    /// it.
    ///
    /// Mutation: check at file scope as well, by testing `len() > 0` in
    /// `Resolver::declare`; the second `x` is reported. Mutation: look for
    /// the first declaration in every visible scope rather than the innermost;
    /// the inner `y` is reported. Either fails this.
    #[test]
    fn a_name_declared_twice_at_file_scope_or_again_in_an_inner_block_is_not_reported() {
        let resolved = resolved(
            "int x;\nint x;\nint main(void) {\n    int y;\n    {\n        int y;\n    }\n    return 0;\n}\n",
        );

        assert_eq!(resolved.messages(), Vec::<&str>::new());
    }

    /// The second declaration of a name in a block is declared all the same,
    /// so a use after it is that declaration, as it would be had nothing been
    /// wrong: the rest of the function is resolved as written.
    ///
    /// Mutation: return from `Resolver::declare` after reporting, before the
    /// binding is pushed; the use resolves to the first `x` and this fails.
    #[test]
    fn a_use_after_a_name_declared_twice_is_the_second_declaration() {
        let resolved = resolved("int main(void) {\n    int x;\n    int x;\n    return x;\n}\n");

        assert_eq!(resolved.messages(), ["redefinition of `x`"]);
        assert_eq!(
            resolved.declaration_of("x", 2),
            Some(resolved.occurrence("x", 1))
        );
    }

    /// Declarations of one name at file scope whose types are not compatible
    /// are reported (C17 6.7 p4): two functions, two objects, an object and
    /// then a function's definition, a `void` return beside an `int` one, and
    /// a `char` parameter beside `()`, which the default promotions part.
    ///
    /// Mutation: drop the comparison in `Resolver::declare`; nothing is
    /// reported and this fails. Mutation: let `reaches_a_struct` answer true
    /// of `void`; `h` is not compared. Mutation: record every standing
    /// declaration as accepting `()`; `k` is not reported.
    #[test]
    fn declarations_of_one_name_with_incompatible_types_are_reported() {
        for (text, name) in [
            ("int f(void);\nchar f(void);\n", "f"),
            ("int x;\nchar x;\n", "x"),
            ("int g;\nint g(void) {\n    return 2;\n}\n", "g"),
            ("void h(void);\nint h(void);\n", "h"),
            ("int k(char c);\nint k();\n", "k"),
        ] {
            let resolved = resolved(text);
            assert_eq!(
                resolved.messages(),
                [format!("conflicting types for `{name}`").as_str()],
                "{text}"
            );
        }
    }

    /// A declaration is compared with the type the earlier ones composed,
    /// which a prototype carries: `int f(int *a)` agrees with the `int f()`
    /// just before it and not with the `int f(char *a)` before that. Pointer
    /// parameters, because a `char` would disagree with `int f()` as well,
    /// through 6.7.6.3 p15's default promotions, and tell nothing apart.
    ///
    /// Mutation: compare with `latest` rather than `standing` in
    /// `Resolver::declare`; the fourth is not reported and this fails.
    /// Mutation: let a declaration with no prototype replace `standing`; the
    /// same.
    #[test]
    fn a_declaration_is_compared_with_the_prototype_before_it() {
        let resolved = resolved("int f();\nint f(char *a);\nint f();\nint f(int *a);\n");

        assert_eq!(resolved.messages(), ["conflicting types for `f`"]);
    }

    /// Compatible declarations of one name are not reported: two prototypes
    /// whose parameters are named differently, a declaration with no
    /// prototype after one, and two tentative definitions of one object.
    ///
    /// Mutation: report every pair, by negating the `compatible` test in
    /// `Resolver::declare`; this fails.
    #[test]
    fn compatible_declarations_of_one_name_are_not_reported() {
        let resolved = resolved("int f(int a);\nint f(int b);\nint f();\nint x;\nint x;\n");

        assert_eq!(resolved.messages(), Vec::<&str>::new());
    }

    /// Two function declarations in one block are compared too, being one
    /// function, while a name with no linkage declared twice there is a
    /// redefinition and not also a conflict.
    ///
    /// Mutation: compare at file scope only; `g` is not reported. Mutation:
    /// compare a pair already reported under 6.7 p3 as well; `h` gets a
    /// second report. Either fails this.
    #[test]
    fn functions_declared_twice_in_a_block_are_compared_and_objects_are_redefined() {
        let resolved = resolved(
            "int main(void) {\n    int g(void);\n    char g(void);\n    int h;\n    char h;\n    return 0;\n}\n",
        );

        assert_eq!(
            resolved.messages(),
            ["conflicting types for `g`", "redefinition of `h`"]
        );
    }

    /// A type that reaches a struct, along its spine or through a parameter,
    /// is not compared until #27 says when two struct types are one.
    ///
    /// Mutation: drop the struct test in `Resolver::declare`; both pairs are
    /// reported. Mutation: walk only the spine in `reaches_a_struct`, as
    /// `types.rs`'s `holds_a_struct` does; `f` is reported. Either fails this.
    #[test]
    fn a_type_that_reaches_a_struct_is_not_compared() {
        let resolved = resolved(
            "struct S;\nstruct S *p;\nstruct S *p;\nint f(struct S *q);\nint f(struct S *q);\n",
        );

        assert_eq!(resolved.messages(), Vec::<&str>::new());
    }

    /// A definition with an empty identifier list agrees only with a
    /// prototype of no parameters, in either order (C17 6.7.6.3 p15).
    ///
    /// Mutation: answer a `()` definition after a prototype as a `()`
    /// declaration in `Resolver::agrees`; `f` is not reported. Mutation: drop
    /// the arm for a prototype after a `()` definition; `g` is not reported.
    /// Either fails this.
    #[test]
    fn a_definition_with_an_empty_list_agrees_only_with_a_prototype_of_none() {
        for (text, expected) in [
            (
                "int f(int *a);\nint f() {\n    return 0;\n}\n",
                vec!["conflicting types for `f`"],
            ),
            (
                "int g() {\n    return 0;\n}\nint g(int *a);\n",
                vec!["conflicting types for `g`"],
            ),
            (
                "int h() {\n    return 0;\n}\nint h();\nint h(int *a);\n",
                vec!["conflicting types for `h`"],
            ),
            (
                "int k(void);\nint k() {\n    return 0;\n}\nint k(void);\n",
                vec![],
            ),
        ] {
            assert_eq!(resolved(text).messages(), expected, "{text}");
        }
    }

    /// A declaration that conflicts does not become the one the next is
    /// compared with, so a later line that agrees with the earlier ones is
    /// not reported for the one that did not.
    ///
    /// Mutation: let a conflicting declaration become the standing one in
    /// `Resolver::declare`; the third `f` is reported too, and this fails.
    #[test]
    fn a_conflicting_declaration_is_not_what_the_next_is_compared_with() {
        let resolved = resolved("int f(int *a);\nint f(char *a);\nint f(int *a);\n");

        assert_eq!(resolved.messages(), ["conflicting types for `f`"]);
    }

    /// An object of pointer-to-function type is read through its pointer:
    /// `int (*p)(int *a)` is the prototype a later `int (*p)()` does not
    /// replace, so a third that disagrees with it is reported.
    ///
    /// Mutation: have `has_a_prototype` look at the type itself rather than
    /// through `called`; the second `p` replaces the first and this fails.
    #[test]
    fn a_pointer_to_a_function_is_compared_with_the_prototype_it_points_at() {
        let resolved = resolved("int (*p)(int *a);\nint (*p)();\nint (*p)(char *a);\n");

        assert_eq!(resolved.messages(), ["conflicting types for `p`"]);
    }

    /// A declaration in a block that hides a file-scope one of another type
    /// is not compared with it: they are two scopes and two entities.
    ///
    /// Mutation: find the earlier declaration in every visible scope in
    /// `Resolver::declare`; the inner `x` is reported and this fails.
    #[test]
    fn a_declaration_hiding_one_of_another_type_is_not_compared_with_it() {
        let resolved = resolved("int x;\nint main(void) {\n    char x;\n    return 0;\n}\n");

        assert_eq!(resolved.messages(), Vec::<&str>::new());
    }

    /// A pair is not compared when either side reaches a struct, whichever
    /// is first, and through an array or a return type as well as a pointer.
    ///
    /// Mutation: drop the test of the standing side; `p` is reported.
    /// Mutation: drop the test of the new side; `q` is reported. Mutation:
    /// let `reaches_a_struct` stop at an array; `a` is reported. Mutation:
    /// let it stop at a function's return; `f` is reported.
    #[test]
    fn a_pair_is_not_compared_when_either_side_reaches_a_struct() {
        let resolved = resolved(
            "struct S;\nstruct S *p;\nint p;\nint q;\nstruct S *q;\nstruct S *a[2];\nint a[2];\nstruct S *f(void);\nint f(void);\n",
        );

        assert_eq!(resolved.messages(), Vec::<&str>::new());
    }

    /// A name with no linkage declared again in a block is reported against
    /// the declaration just before it, which is not the standing one when an
    /// earlier declaration had a prototype.
    ///
    /// Mutation: report it against `standing` rather than `latest` in
    /// `Resolver::declare`; the label points at the first `g` and this fails.
    #[test]
    fn a_redeclaration_in_a_block_points_at_the_declaration_before_it() {
        let resolved = resolved(
            "int main(void) {\n    int g(int *a);\n    int g();\n    int g;\n    return 0;\n}\n",
        );

        assert_eq!(resolved.messages(), ["redefinition of `g`"]);
        let previous: Vec<Span> = resolved.diagnostics.diagnostics()[0]
            .labels()
            .iter()
            .filter(|label| !label.is_primary())
            .map(|label| label.span())
            .collect();
        assert_eq!(previous, [resolved.occurrence("g", 1)]);
    }

    /// Every declaration of a name answers the standing declaration its
    /// scope ended with, which is the prototype here, and a use written
    /// before the prototype answers it too. A name never redeclared answers
    /// itself, and a function declared in a block answers its block's.
    ///
    /// Mutation: drop `settle` from `close_scope`; the block's `h` answers
    /// itself and this fails. Mutation: drop `settle` at the end of
    /// `resolve`; `f` answers the declaration it resolves to. Mutation:
    /// settle each binding on `latest` rather than `standing`; `f` answers
    /// its last declaration, the third. Each fails this.
    #[test]
    fn every_declaration_of_a_name_answers_its_standing_one() {
        let resolved = resolved(
            "int f();\nint g(void) {\n    int h(int *a);\n    int h();\n    return f(0) + h(0);\n}\nint f(int *a);\nint f();\nint k(void) {\n    return f(0);\n}\n",
        );

        assert_eq!(resolved.messages(), Vec::<&str>::new());
        // The use in `g`, written before the prototype, and the one in `k`.
        assert_eq!(
            resolved.standing_of("f", 1),
            Some(resolved.occurrence("f", 2))
        );
        assert_eq!(
            resolved.standing_of("f", 4),
            Some(resolved.occurrence("f", 2))
        );
        assert_eq!(
            resolved.standing_of("h", 2),
            Some(resolved.occurrence("h", 0))
        );
        assert_eq!(
            resolved
                .resolution
                .declared("k")
                .map(|ty| matches!(resolved.ast.ty(ty), Type::Function { .. })),
            Some(true)
        );
    }
}
