# Parser Spec

## Goal

Consume the lexer's `Vec<Token>` and produce an Abstract Syntax Tree (AST) for the full Raven v2 grammar. The parser is a hand written recursive descent implementation with operator precedence climbing for expressions. Every AST node carries a `Span` so downstream passes (name resolution, type checking, code generation) can produce errors anchored at the source.

The parser is total: it never panics on user input. Every malformed program produces a `RavenError::Parse(ParseError, Span, Option<String>)` and the parser does not attempt aggressive recovery in this release (one error, stop). Recovery is tracked separately and is out of scope for this PR.

## Pipeline position

```
Source -> Lexer -> Parser -> AST -> (name resolution -> type check -> hir -> mir -> codegen)
```

The parser consumes tokens produced by `src/lexer` and produces nodes defined in `src/ast`. The full grammar is implemented; the spec below is the source of truth for that grammar.

## Grammar

### File and items

```
File         := { Item NewlinesOrSemis }*
Item         := Function | Struct | Trait | Impl | Enum | Extern | Import | Const | Let
```

Top level `let` is a mutable module level binding (matches the variables are mutable design). `const` is a compile time constant. Items are separated by newlines, semicolons, or both. Trailing separators are accepted.

### Imports

```
Import       := "import" ImportSource [ "as" Identifier ] [ "{" IdentList [ "," ] "}" ]
ImportSource := StdPath | StringLit
StdPath      := "std" "/" Identifier { "/" Identifier }*
IdentList    := Identifier { "," Identifier }*
```

Examples:

* `import std/io`
* `import std/collections/{Map, Set}`
* `import "github.com/martian56/raven-http"`
* `import "github.com/martian56/raven-http" as http`
* `import "./helpers"`

#### Path token disambiguation

The lexer does not emit a dedicated `PathSep` token: it sees `/` as `TokenKind::Slash` (division). The parser switches to a path parsing mode immediately after the `import` keyword: when the next token is the identifier `std`, the parser consumes a sequence of `(Slash Identifier)` pairs greedily before falling back to its normal item parsing.

For `github.com/...` and `./...` paths, the lexer treats them as a single `StringLit`. Users must therefore quote those forms (matching the grammar's `StringLit` production).

### Functions

```
Function     := "fun" Identifier [ GenericParams ] "(" ParamList ")" [ "->" Type ] FunctionBody
FunctionBody := Block | "=" Expr
ParamList    := [ SelfReceiver [ "," Param ]* | Param { "," Param }* ] [ "," ]
SelfReceiver := "self"
Param        := Identifier ":" Type
GenericParams := "<" GenericParam { "," GenericParam }* [ "," ] ">"
GenericParam := Identifier [ ":" TraitBound { "+" TraitBound }* ]
TraitBound   := TypePath
```

A leading `self` parameter is allowed in `impl` and `trait` methods. The parser desugars it to a parameter named `"self"` carrying a synthetic `Self` type so that downstream passes do not need a special case. The full `self: Self` spelling is also accepted and parses identically.

The expression body form (`= expr`) makes the return value the trailing expression. A function with a block body and no return type defaults to `()`.

### Structs, traits, impls, enums

```
Struct       := "struct" Identifier [ GenericParams ] StructBody
StructBody   := "{" [ FieldList ] "}"
FieldList    := Field { Separator Field }* [ Separator ]
Field        := Identifier ":" Type
Separator    := "," | Newline | ( "," Newline )

Trait        := "trait" Identifier [ GenericParams ] "{" { TraitMember NewlinesOrSemis }* "}"
TraitMember  := "fun" Identifier [ GenericParams ] "(" ParamList ")" [ "->" Type ] [ FunctionBody ]

Impl         := "impl" [ GenericParams ] TypePath [ "for" TypePath ] "{" { Function NewlinesOrSemis }* "}"

Enum         := "enum" Identifier [ GenericParams ] "{" { EnumVariant Separator }* "}"
EnumVariant  := Identifier [ "(" VariantPayload ")" ]
VariantPayload := NamedFields | TypeList
NamedFields  := Identifier ":" Type { "," Identifier ":" Type }* [ "," ]
TypeList     := Type { "," Type }* [ "," ]
```

A trait member with a body is a default method. An impl block's only allowed members are functions (in this release).

Variant payload disambiguation: the parser parses the first item between the parentheses optimistically. If it sees `Identifier ":"` followed by a type, the variant has named fields; otherwise it parses a comma separated `TypeList`.

### Externs and constants

```
Extern       := "extern" StringLit "{" { ExternItem NewlinesOrSemis }* "}"
ExternItem   := "fun" Identifier "(" ParamList ")" [ "->" Type ]

Const        := "const" Identifier ":" Type "=" Expr
Let          := "let" Identifier [ ":" Type ] [ "=" Expr ]
```

`extern "C" { ... }` declares a block of foreign function signatures bound to the given ABI string. The parser does not validate the ABI name; that is a job for the resolver.

### Types

```
Type         := PrimaryType [ "?" ]
PrimaryType  := TypePath
              | "dyn" TypePath
              | "(" ")"
              | "fun" "(" [ TypeList ] ")" "->" Type
TypePath     := Identifier [ "<" TypeArgs ">" ] { "." Identifier [ "<" TypeArgs ">" ] }*
TypeArgs     := Type { "," Type }* [ "," ]
```

`T?` is sugar for `Option<T>`. It is parsed as a unary postfix on the `PrimaryType`. The AST records it as `Type::Optional(Box<Type>)` so later passes can desugar uniformly.

Qualified types use `.` as the separator (e.g., `std.collections.Map<String, Int>`). `::` is reserved for explicit associated paths in expressions (parser accepts the token; semantics are deferred).

### Statements

```
Statement    := Let
              | "return" [ Expr ]
              | "break" [ Expr ]
              | "continue"
              | "defer" Expr
              | Assignment
              | ExprStmt
Let          := "let" Identifier [ ":" Type ] "=" Expr
ExprStmt     := Expr
Assignment   := LValue AssignOp Expr
AssignOp     := "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>="
LValue       := Identifier { ( "." Identifier | "[" Expr "]" ) }*
```

Statement separators inside a block are `Newline`, `Semi`, or both. The block parser keeps consuming statements until it sees `RBrace`.

#### Assignment vs expression

Assignment is a statement, not an expression. The parser disambiguates by parsing an expression first; if the next non newline token is an assignment operator AND the parsed expression is a syntactically valid LValue, the result is an `Assignment` statement. Otherwise the parsed expression becomes an `ExprStmt`. An invalid LValue followed by `=` produces `ParseError::InvalidAssignmentTarget`.

### Expressions

Precedence, lowest to highest. All binary operators are left associative except `Range`, which is non associative (chaining two range operators is a parse error).

```
LogicalOr      := LogicalAnd { "||" LogicalAnd }*
LogicalAnd     := Comparison  { "&&" Comparison  }*
Comparison     := BitOr       { ( "==" | "!=" | "<" | ">" | "<=" | ">=" ) BitOr }*
BitOr          := BitXor      { "|" BitXor }*
BitXor         := BitAnd      { "^" BitAnd }*
BitAnd         := Shift       { "&" Shift }*
Shift          := Range       { ( "<<" | ">>" ) Range }*
Range          := Additive    [ ( ".." | "..=" ) Additive ]
Additive       := Multiplicative { ( "+" | "-" ) Multiplicative }*
Multiplicative := Unary       { ( "*" | "/" | "%" ) Unary }*
Unary          := { ( "-" | "!" | "&" ) }* Postfix
Postfix        := Primary { Suffix }*
Suffix         := "." Identifier [ TypeArgs ] [ "(" ArgList ")" ]
              | "(" ArgList ")"
              | "[" Expr "]"
              | "?"
Primary       := Literal
              | Identifier [ TypeArgs ] [ StructLit ]
              | "(" Expr ")"
              | "[" ExprList "]"            // list literal
              | "[" MapEntries "]"          // map literal
              | "[" ":" "]"                 // empty map
              | SetLit
              | "{" Block "}"
              | IfExpr | MatchExpr | LoopExpr | WhileExpr | ForExpr | LambdaExpr
              | "self" | "Self"
Literal       := IntLit | FloatLit | BoolLit | StringLit | BlockStringLit | CharLit | CStringLit
StructLit     := "{" [ FieldInit { Separator FieldInit }* [ Separator ] ] "}"
FieldInit     := Identifier ":" Expr
              | Identifier
SetLit        := "{" Expr { "," Expr }* [ "," ] "}"   // at least one element
MapEntries    := Expr ":" Expr { "," Expr ":" Expr }* [ "," ]
ArgList       := [ Expr { "," Expr }* [ "," ] ]
ExprList      := [ Expr { "," Expr }* [ "," ] ]
```

Comparisons are not chained: `a < b < c` is a parse error. The grammar above does technically allow chaining; the parser explicitly rejects more than one comparison operator at the `Comparison` level with `ParseError::ChainedComparison`.

#### Control flow

```
IfExpr        := "if" Expr Block { "else" "if" Expr Block } [ "else" Block ]
MatchExpr     := "match" Expr "{" { MatchArm Separator }* "}"
MatchArm      := Pattern [ "if" Expr ] "->" Expr
LoopExpr      := "loop" Block
WhileExpr     := "while" Expr Block
ForExpr       := "for" Pattern "in" Expr Block

LambdaExpr    := "fun" "(" ParamList ")" [ "->" Type ] FunctionBody
              | "{" [ Identifier { "," Identifier }* "->" ] Block "}"
```

Both `if` and `match` are expressions and have a value. `while` and `for` evaluate to `()`. `loop` evaluates to the operand of `break value` (or `()` if `break` carries no value).

#### Lambda shorthand

`{ x, y -> body }` is a closure whose parameter types are inferred. The parser distinguishes shorthand lambdas from plain block expressions by looking ahead for an `Arrow` token (`->`) at the same brace depth before a statement separator. If found, the brace introduces a shorthand lambda; otherwise it is a block expression. A `{}` empty brace is always a block (the unit value).

#### Set and map literals

A set literal is `{ e1, e2, ... }`: a brace around one or more comma-separated expressions. A map literal is `[ k1: v1, k2: v2, ... ]`: a bracket around one or more comma-separated `key: value` pairs. Both lower to the bundled `std/collections` constructors (`Set.new()` plus `add` per element, `Map.new()` plus `set` per pair), so the literal needs `import std/collections` in scope. The parser produces dedicated `ExprKind::SetLit` and `ExprKind::MapLit` nodes; the desugaring to constructors happens during HIR lowering.

Disambiguation rules:

- Brace `{ ... }`. The parser first checks for a shorthand lambda (an `->` at brace depth 1 before a separator). Failing that, it checks for a set literal: a set is a brace whose first element is an expression followed, at brace depth 1, by a `,` before any statement separator (`;` or newline) or the closing `}`. A leading statement keyword (`let`, `return`, `break`, `continue`) marks a block immediately. A single-element `{ x }` (no comma) stays a block whose tail expression is `x`, preserving existing block behavior, so a single-element set is written `{x,}`. An empty `{}` is a block; an empty set is `Set.new()`.
- Bracket `[ ... ]`. The empty `[]` is an empty list and `[:]` is an empty map. Otherwise the parser reads the first element expression, then looks at the next top-level token: a `:` makes it a map literal (the first element was a key), anything else makes it a list literal. So `[1, 2, 3]` is a list and `["a": 1]` is a map.

These rules do not change how blocks, struct literals, `match` arm bodies, or `if`/`while`/`for` bodies parse: a set literal requires the comma form, and a map literal requires a top-level `:`, neither of which those forms produce.

#### Patterns

```
Pattern       := "_"
              | LiteralPat
              | RangePat
              | IdentPat [ "(" PatternList ")" | "{" FieldPatList "}" ]
              | "(" PatternList ")"
LiteralPat    := IntLit | FloatLit | StringLit | CharLit | BoolLit
RangePat      := IntLit ( ".." | "..=" ) IntLit
IdentPat      := Identifier
PatternList   := Pattern { "," Pattern }* [ "," ]
FieldPatList  := FieldPat { "," FieldPat }* [ "," ]
FieldPat      := Identifier [ ":" Pattern ]
```

`IdentPat` covers three cases. Standalone, it binds the scrutinee. Followed by `(...)`, it is an enum tuple variant. Followed by `{...}`, it is a struct or enum struct variant. Resolution between "bind a name" and "match a constructor" is deferred to name resolution; the parser records the pattern shape.

Tuple patterns require at least two elements (a single parenthesized pattern is just a grouping). Single element tuple patterns are deferred with the rest of tuple support.

### Disambiguation rules

#### `<` as comparison vs generic args

In expression contexts, `<` is ambiguous between the comparison operator and the start of a generic type argument list. The parser disambiguates with bounded lookahead: when the postfix layer sees `Identifier <`, it tries to parse `<` ... `>` as a `TypeArgs`. The trial parse succeeds only if (a) the next token after `<` starts a `Type`, (b) the trial reaches a `>` before hitting a statement separator or an obviously non type token (`==`, `!=`, `&&`, `||`, `?`, `+`, `-`, `*`, `%`, etc.), and (c) the token immediately after the matched `>` is one of the syntactic frames that confirms a generic application (`(`, `.`, `::`, `{`, `=`, `,`, `)`, `]`, `;`, `Newline`, `Eof`).

If the trial fails, the parser rewinds to the position of the `<` and treats it as comparison. The implementation uses an explicit checkpoint and rewind in the parser; lookahead does not consume tokens.

In type contexts (after `:` in a parameter or let, after `->` in a return type, inside another `TypeArgs`, etc.), `<` is unambiguously a type argument opener and no trial is needed.

`>>` inside nested generics is split into two `>` for type argument list parsing (e.g., `Vec<Vec<Int>>` lexes as `Vec Lt Vec Lt Int Shr` and the parser splits the trailing `Shr` into two close angles when needed).

#### `{` after an expression

A `{` immediately after an identifier (or `Self`) parses as a struct literal. After `if Expr {`, `while Expr {`, `for ... in Expr {`, and `match Expr {`, the `{` is a block (or match body), not a struct literal. The parser tracks a `no_struct_literal` flag while parsing the condition expression of `if`, `while`, and similar, matching the standard Rust solution.

#### Newlines inside expressions

`Newline` tokens are statement separators at block and item scope. Inside an expression, the parser consumes and ignores them at every continuation point: after binary operators, after `,` and `;` in argument lists, after `(`, `[`, `{`, before a postfix `.`, after `=`. A helper `skip_newlines_inside_expression()` is invoked liberally. A statement therefore cannot terminate mid expression: once an expression has begun, only an actual end of expression token (closing brace, closing paren at the matching depth, or end of file) ends it.

#### Block expressions

A block `{ ... }` parses zero or more statements followed by an optional trailing expression. If the last item parses as an `ExprStmt` and only newlines separate it from the closing `}` (no `;`), that expression becomes the block's value. Otherwise the block evaluates to `()`. An explicit `;` terminates the expression as a statement and forces the block to evaluate to `()`.

## Error model

The parser uses the existing `RavenError::Parse(ParseError, Span, Option<String>)` variant.

```rust
pub enum ParseError {
    UnexpectedToken { expected: String, found: String },
    UnexpectedEof { expected: String },
    InvalidAssignmentTarget,
    ChainedComparison,
    DuplicateField(String),
    InvalidImportPath,
    UnsupportedTuple,
    InvalidPattern(String),
    Custom(String),
}
```

Most errors render as `expected X, found Y`. Hints are attached at call sites where the parser can offer concrete advice (for example: missing arrow before return type, missing `=` on a single expression body, etc.).

## Deferred to follow up issues

* **Tuples.** `(a, b)` parses but produces `ParseError::UnsupportedTuple`. Pattern tuples are similarly deferred. The grammar is in place for v2.x.
* **String interpolation splitting.** The lexer preserves `${...}` segments verbatim in `StringLit`. Re tokenizing them into expression fragments is tracked in issue #69.
* **`::` semantics.** The token is accepted in trial paths but rejected in real ones, except inside `extern` ABIs where it is not used. Resolver level path syntax lands when name resolution does.
* **Attribute and visibility modifiers.** No `@deriving`, no `pub`, no `mut` keyword in v2.0. The parser does not consume them.

## Test coverage

* Inline unit tests under `src/parser/tests.rs` exercise each grammar production with positive and negative cases.
* Golden snapshot tests under `tests/parser_golden.rs` walk a corpus of small `.rv` programs in `tests/parser_corpus/`, parse them, pretty print the AST, and diff against committed `.rv.ast` baselines. Run with `cargo test --test parser_golden`. Refresh baselines after an intentional change with `RAVEN_UPDATE_PARSER_GOLDEN=1 cargo test --test parser_golden`.

## Design notes

* The AST is split by category: `expr`, `stmt`, `decl`, `pattern`, `ty`. Every node embeds a `Span`. Public types are `Expr`, `Stmt`, `Decl`, `Pattern`, `Type` and a wrapper `File` for the top level.
* The parser is a single struct holding a token slice and a cursor. Internal modules group functions by grammar category; the cursor is shared.
* Errors are non recovering in this release. The parser stops at the first error.
* Operator precedence is encoded in dedicated recursive functions per level rather than a Pratt table. The level count is small enough that the explicit form is easier to read and modify.
