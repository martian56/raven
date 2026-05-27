# Resolver Spec

## Goal

Walk an `ast::File`, bind every identifier use to the declaration it refers to, and resolve every import path to a target. The output is a `ResolvedFile` that pairs the original AST with a `ResolutionMap` keyed by identifier span. Downstream passes (type checker, lowering) read the map to answer the question "given this identifier site, what does it bind to?"

The resolver is total: every malformed program produces a `RavenError::Resolve(ResolveError, Span, Option<String>)` with the offending span highlighted. The resolver does not infer types, does not dispatch methods, and does not load any actual stdlib content. Method dispatch and stdlib member lookup are deferred to the type checker.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> (type check -> hir -> mir -> codegen)
```

The resolver consumes nodes defined in `src/ast` and produces nodes in `src/resolve`. The first error halts resolution of the current file; multi error recovery is out of scope for this release.

## Passes

Resolution runs in two passes per file:

1. **Item collection.** Walk top level `Decl`s and bind every declared name (functions, structs, traits, enums, externs, top level lets and consts, import aliases) into a fresh `ModuleScope`. Duplicate declarations in the same scope raise `DuplicateDeclaration`. No expressions are walked yet.
2. **Body walk.** Walk every body (function bodies, struct field types, enum payload types, const initializers, top level let initializers, impl items) in declaration order. Each identifier use is recorded in the `ResolutionMap` against the binding it resolves to.

The two passes exist so module level forward references work without ordering constraints: a function defined later in the file can be called from one defined earlier, and a struct can reference a trait declared further down. The first pass is purely declarative; the second is purely a use site walk.

## Scope kinds

The scope stack is a linked list of frames. Each frame stores a name to `Binding` map and a `ScopeKind`:

* `Module`: top level of a file. Contains items from item collection and any import aliases. Implicit root frame.
* `Impl`: opened on entry to an `impl` block. Binds `Self` (the implementing type's path) and `self` (a parameter style binding when methods take a `self` receiver). Generic parameters of the impl live here.
* `Function`: opened on entry to a function body. Binds parameters and the function's own generic parameters.
* `Block`: opened for every `{ ... }` block expression. Holds `let` bindings introduced inside the block. Inner blocks shadow outer blocks.
* `Pattern`: a synthetic frame opened by `match` arms, `for` heads, and `let` patterns. Holds identifiers bound by the pattern for the duration of the arm or binding body.

Lookup walks frames from innermost to outermost. The first match wins (shadowing).

## Bindings

A `Binding` is a tagged reference to the declaration responsible for the name. The variants are:

* `Function(decl_id)`: a top level function.
* `Struct(decl_id)`: a struct type.
* `Trait(decl_id)`: a trait.
* `Enum(decl_id)`: an enum type.
* `Variant { enum_id, variant_index }`: a variant of an enum (in scope when the enum is in scope by name).
* `Extern(decl_id, item_index)`: a foreign function from an extern block.
* `Const(decl_id)`: a top level constant.
* `Static(decl_id)`: a top level `let` (mutable module global).
* `Param(span)`: a function parameter, identified by its declaration span.
* `Local(span)`: a `let` binding inside a function body, identified by its declaration span.
* `PatternBinding(span)`: a name bound by a pattern.
* `GenericParam(owner_span, name)`: a generic parameter in scope.
* `SelfType`: refers to the enclosing `impl`'s target type.
* `SelfValue`: refers to the enclosing `impl`'s `self` parameter.
* `ImportAlias(import_id)`: an `import ... as alias` brings in a single name pointing at the resolved import target.
* `ImportedItem { import_id, name }`: a specific selector from `import std/io { println }` (resolution of the inner name is deferred).
* `ExternalPackage { import_id }`: a `github.com/...` package; member lookup deferred to rvpm.

`decl_id` is an opaque newtype index into the file's `Vec<Decl>`. `import_id` is the index into the file's import list.

## Resolution algorithm

In pseudocode:

```
fn resolve_file(file):
  let module = collect_items(file)         # pass 1
  for decl in file.items:
    walk_decl(decl, module)                # pass 2
  return ResolvedFile { ast: file, map }

fn walk_decl(decl, module):
  match decl:
    Function(f) -> with Function frame holding f.generics + f.params,
                   walk body, recording uses
    Struct(s)   -> with frame holding s.generics, walk field types
    Trait(t)    -> with frame holding t.generics, walk member sigs and default bodies
    Impl(i)     -> with Impl frame holding i.generics, Self, walk items
    Enum(e)     -> with frame holding e.generics, walk variant payload types
    Const(c)    -> walk type, walk init expression
    Let(l)      -> walk optional type, walk optional init
    Extern(_)   -> walk parameter and return types of each item
    Import(_)   -> already handled in pass 1; nothing to walk

fn walk_expr(expr):
  match expr.kind:
    Ident { name, generics } -> lookup(name); walk generics
    SelfLower                -> require SelfValue in scope, else SelfOutsideImpl
    SelfUpper                -> require SelfType in scope, else SelfOutsideImpl
    Block(b)                 -> push Block frame; walk stmts; pop
    Match { scrutinee, arms} -> walk scrutinee; for each arm push Pattern frame
                                with names from pattern, walk guard and body
    Lambda { params, body }  -> push Function-like frame with params; walk body
    ... recurse on every sub expression
```

`lookup(name)` walks frames inner to outer. If no binding is found at the file level and an import alias matches, return that import binding. Otherwise raise `UnresolvedName`.

## Import resolution

Three shapes are produced by the parser as `ImportSource`:

1. **`std/<segments>`** (`ImportSource::Std`). Looked up in a static registry of v2 stdlib module names: `io`, `collections`, `string`, `math`, `fs`, `net`, `http`, `time`, `json`, `ffi`. Successful lookup binds the resulting `ImportTarget::StdlibModule { segments }`. The resolver does NOT load any contents because the stdlib does not exist yet; member resolution against the imported name is deferred to the type checker. Unknown module names raise `UnresolvedImport`.

2. **`"github.com/<user>/<repo>[/<sub>]"`** (`ImportSource::Quoted` with a leading `github.com/` host). Parsed into `ImportTarget::ExternalPackage { host, user, repo, subpath }`. Fetching is deferred to `rvpm`; the resolver records the target and continues. The alias (or last path segment if no alias) is bound as `ImportAlias`.

3. **`"./<path>"` or `"../<path>"`** (relative path strings). The resolver asks the `SourceLoader` to read the file relative to the importing file's directory. If the loader returns content, the resolver lexes, parses, and recursively resolves it, then records `ImportTarget::LocalModule { resolved_path, module }` and merges the inner file's top level names into the outer scope under the alias. If the loader cannot find the file, raise `UnresolvedImport`. The resolver tracks an in progress set of canonical paths and raises `CyclicImport` if it would recurse into a path it is already resolving.

If the import provides selectors (`import std/io { println, eprintln }`), each selector becomes an `ImportedItem` binding in the module scope. Otherwise the import binds a single alias (the `as` name, or the last path segment as a fallback). Duplicate aliases or selectors raise `DuplicateDeclaration`. Conflicting imports of the same name from different sources raise `AmbiguousName`.

## SourceLoader trait

```rust
pub trait SourceLoader {
    fn load(&mut self, importing: &Path, target: &str) -> Option<LoadedSource>;
}

pub struct LoadedSource {
    pub canonical_path: PathBuf,
    pub source: String,
}
```

Tests inject an in memory loader keyed by relative path so resolver behavior is exercised without filesystem state. The real CLI ships with a `FsLoader` that resolves `./foo` against `importing.parent()` and reads the file from disk.

## Errors

`ResolveError` lives next to `LexError` and `ParseError` in `src/error.rs`. Variants:

* `UnresolvedName(name)`: identifier not in any enclosing scope.
* `DuplicateDeclaration { name, first_span }`: same identifier declared twice in the same scope.
* `UnresolvedImport(path)`: import target not found.
* `CyclicImport(path)`: import graph contains a cycle.
* `AmbiguousName { name, candidates }`: name visible via multiple imports.
* `SelfOutsideImpl`: `self` or `Self` used outside an `impl` block.

Each error reaches the user via the existing `RavenError::Resolve(error, span, hint)` arm. The renderer is unchanged.

## Out of scope

* Cross package import fetching (rvpm responsibility, future PR).
* Type aware resolution: method dispatch, trait method selection, struct field access through inferred types. These belong to the type checker.
* Visibility modifiers (`pub`, `private`). Every module level name is importable.
* Macro expansion. No macros exist yet.

## Tests

* Unit tests inline at `src/resolve/tests.rs`: cover scope basics, shadowing, forward references, duplicate declarations, unresolved names, `self` outside impl, import alias binding, import selector binding, std module recognition, in memory recursive local imports, cyclic import detection.
* Golden snapshot tests at `tests/resolver_golden.rs` over a corpus at `tests/resolver_corpus/`. Each `.rv` source has a committed `.rv.resolved` baseline produced by dumping every identifier use and its resolved binding. Refresh with `RAVEN_UPDATE_RESOLVER_GOLDEN=1`.
