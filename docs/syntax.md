# Raven syntax reference

This document describes the **concrete syntax** of the Raven language as implemented by the reference interpreter (lexer and parser). Source files use the `.rv` extension.

For semantic rules (typing, runtime behavior), see the language reference and standard library docs.

---

## Lexical structure

### Whitespace

Spaces, tabs, and newlines separate tokens. Whitespace is required only where two tokens would otherwise merge (for example, between a keyword and an identifier).

### Comments

- **Line comment**: from `//` to the end of the line.
- **Block comment**: from `/*` to `*/`. Block comments do not nest.

```raven
// line comment
let x: int = 1;

/* multi-line
   comment */
let y: int = 2;
```

### Identifiers

An identifier starts with an ASCII letter or underscore and continues with letters, digits, or underscores.

```ebnf
identifier = ( letter | "_" ) { letter | digit | "_" } ;
letter     = "a"…"z" | "A"…"Z" ;
digit      = "0"…"9" ;
```

Identifiers are case-sensitive. Type names for built-in string types are written as `String` in type positions (see [Types](#types)).

### Keywords

The following words are reserved and cannot be used as identifiers:

`let`, `const`, `fun`, `return`, `if`, `elseif`, `else`, `while`, `for`, `import`, `export`, `from`, `struct`, `impl`, `enum`, `print`, `and`, `or`, `not`, `int`, `float`, `bool`, `String`, `void`, `true`, `false`.

Note: `const` is recognized by the lexer as a keyword; the current parser does not treat `const` as a declaration. Use `let` for variables.

### Literals

| Kind    | Form |
|--------|------|
| Integer | Sequence of digits; optional leading `-` only as a unary operator on an expression, not as part of the literal token. |
| Float   | Digits with a single `.` (for example `3.14`). |
| Boolean | `true` or `false`. |
| String  | Characters between double quotes `"..."`. The lexer does not process escape sequences inside strings; newlines inside a string are not supported by the quote rules. |

### Punctuation and operators

Single- and multi-character tokens include:

| Token | Meaning |
|-------|---------|
| `=` | Assignment |
| `:` | Type annotation separator |
| `;` | Statement terminator |
| `,` | Separator |
| `.` | Member access |
| `(` `)` | Grouping, calls |
| `{` `}` | Blocks, struct literals |
| `[` `]` | Arrays, indexing |
| `->` | Function return type |
| `+` `-` `*` `/` `%` | Arithmetic |
| `==` `!=` `<` `>` `<=` `>=` | Comparisons |
| `&&` `||` | Logical AND, OR (same token kinds as `and` / `or`) |
| `!` | Logical NOT (same token kind as `not`) |
| `::` | Enum variant (after a type name) |

The token `..` is lexed (`DotDot`) but is not used by the core parser for range syntax.

Characters that are not valid tokens in context produce a lexer error.

---

## Types

### Primitive and array types

In type positions (after `:`), the following built-in spellings are accepted:

| Syntax | Meaning |
|--------|---------|
| `int` | 64-bit signed integer |
| `float` | Floating-point |
| `bool` | Boolean |
| `String` | String (also accepted as `string` in some positions inside the implementation) |
| `void` | No value (return type) |
| `int[]`, `float[]`, `bool[]`, `String[]` | Homogeneous arrays |

User-defined types are referred to by identifier: struct and enum names.

### Typing in declarations

Every `let` declaration must include an explicit type:

```raven
let name: type = expression;
```

Omitting `= expression` is allowed only for types that have a default: `int` (0), `float` (0.0), `bool` (`false`), `String` / string (`""`), and array types (`[]`). Other types require an initializer.

---

## Operator precedence and associativity

Binary operators are parsed with the following precedence (highest to lowest). All binary operators are left-associative except that unary operators bind tighter than binary ones.

| Level | Operators |
|-------|-----------|
| 7 | Unary `-`, unary `!` / `not` |
| 6 | `*`, `/`, `%` |
| 5 | `+`, `-` (binary) |
| 4 | `<`, `>`, `<=`, `>=` |
| 3 | `==`, `!=` |
| 2 | `&&`, `and` |
| 1 | `\|\|`, `or` |

Parentheses `( )` override precedence.

---

## Expressions

Expressions are built from literals, identifiers, operators, calls, indexing, and the forms below.

### Primary forms

- **Literals**: integer, float, boolean, string.
- **Identifiers**: variable names; may be followed by suffixes (call, struct literal, index, field, method, enum variant).
- **Parenthesized**: `( expression )`.
- **Array literal**: `[` optional `expression` (`,` `expression`)* `]` .
- **Struct literal**: `TypeName {` field `:` expression (`,` field `:` expression)* `}` .
- **Function call**: `name (` optional arguments `)` .
- **Method call / field access**: `expr .` identifier with optional `( arguments )` for methods; chaining is supported.
- **Indexing**: `expr [ expression ]` (arrays and strings).
- **Enum variant**: `EnumName :: VariantName` (after the type name, `::` introduces the variant).

### Assignment expressions (statements)

At statement level, assignment is `expression = expression ;` where the left-hand side is an assignable expression (identifier, field access, array index).

---

## Statements and program structure

A program is a sequence of **items**. The parser builds a single block of statements from the file.

### Top-level items

These may appear at the outermost level:

- `let` variable declaration
- `fun` function declaration
- `struct` declaration
- `impl` block
- `enum` declaration
- `import` / `export`
- `if` / `while` / `for`
- `return` (at top level is unusual but parsed)
- `print ( … ) ;`
- Expression statement or assignment (for example a bare call)

Nested `fun`, `struct`, `enum`, `impl`, `import`, and `export` are **not** accepted inside `{ }` blocks by the parser—only a subset of statements is allowed in block bodies (see below).

### Block bodies

Inside `{ }` (function bodies, `if`, `while`, `for`), allowed statements include:

- `let`
- `if`, `while`, `for`
- `return`
- `print ( … ) ;`
- Expression statements and assignments

### Variable declaration

```raven
let name: type = expression;
let name: type;   // only for types with defaults (see above)
```

### Expression statement

```raven
expression;
```

### Assignment

```raven
assignable_expression = expression;
```

### `return`

```raven
return expression;
```

Every function path should match the declared return type; `void` functions typically use `return` with a value compatible with `void` per the type checker, or omit—follow the compiler’s rules for your version.

### `print`

```raven
print ( argument ( , argument )* ) ;
```

`print` is a dedicated statement form in the parser (not an ordinary identifier call).

### Conditional

```raven
if ( expression ) {
  statements
} elseif ( expression ) {
  statements
} …
else {
  statements
}
```

`elseif` chains are parsed as nested conditionals. Each branch body is a block `{ }`.

### Loops

**While:**

```raven
while ( expression ) {
  statements
}
```

**For** (C-style; initialization must be a `let` declaration):

```raven
for ( let name: type = expression; expression; name = expression ) {
  statements
}
```

The increment clause must be an assignment to an identifier (for example `i = i + 1`).

---

## Functions

```raven
fun name ( parameter : type ( , parameter : type )* ) -> return_type {
  statements
}
```

If `-> return_type` is omitted, the return type defaults to `void`.

Parameters use the same type syntax as variables. The body is a single block.

---

## User-defined types

### Struct

```raven
struct Name {
  field : type ( , field : type )*
}
```

Fields are separated by commas. Trailing commas are not required.

### `impl` (methods)

```raven
impl StructName {
  fun method(self) -> return_type { statements }
  fun method(self : StructName, param: type) -> return_type { statements }
  …
}
```

The first parameter must be named `self`. An optional `: StructName` (or compatible) type annotation is allowed after `self`. Additional parameters use the usual `name: type` form.

Methods are called with `value.method(arguments)`; fields use `value.field`.

### Enum

```raven
enum Name {
  Variant ( , Variant )*
}
```

Variants are identifiers, separated by commas. Variants are referenced as `Name::Variant`.

---

## Modules

### Import

**Whole module path (string):**

```raven
import "path.rv";
```

**Import with namespace alias:**

```raven
import alias from "path.rv";
```

**Selective import:**

```raven
import { name ( , name )* } from "path.rv";
```

Each import ends with `;`. The path is a string literal.

### Export

```raven
export let name: type = expression;
export fun name(…) -> type { … }
```

Only `export` followed by `let` or `fun` is accepted.

---

## Built-in and standard library (syntax-related)

Calls to runtime and standard functions use the normal `identifier ( … )` syntax. Commonly used names include `format`, `len`, `type`, `input`, `read_file`, `write_file`, `append_file`, `file_exists`, `enum_from_string`, and methods on strings and arrays (for example `push`, `pop`, `slice`, `join`). See [STDLIB_SPEC.md](STDLIB_SPEC.md) and the standard library overview for the full API.

---

## Summary grammar (informal)

```text
program           = { top_level_statement }
top_level_statement = let_decl | fun_decl | struct_decl | enum_decl | impl_block
                    | import | export | if_stmt | while_stmt | for_stmt
                    | return_stmt | print_stmt | expr_stmt | assign_stmt

let_decl          = "let" identifier ":" type ( "=" expr )? ";"

fun_decl          = "fun" identifier "(" params ")" ( "->" type )? block

struct_decl       = "struct" identifier "{" field_list "}"
enum_decl         = "enum" identifier "{" variant_list "}"
impl_block        = "impl" identifier "{" { method } "}"

import            = "import" ( string | identifier … ) … ";"   // see [Modules](#modules)
export            = "export" ( let_decl | fun_decl )

if_stmt           = "if" "(" expr ")" block { "elseif" "(" expr ")" block } [ "else" block ]
while_stmt        = "while" "(" expr ")" block
for_stmt          = "for" "(" let_decl expr ";" assign ")" block

print_stmt        = "print" "(" [ expr { "," expr } ] ")" ";"
return_stmt       = "return" expr ";"
expr_stmt         = expr ";"
assign_stmt       = expr "=" expr ";"

block             = "{" { inner_statement } "}"
inner_statement   = let_decl | if_stmt | while_stmt | for_stmt | return_stmt | print_stmt
                  | expr_stmt | assign_stmt

expr              = … precedence climbing / unary / primaries …
```

---

## Version

This syntax reference is aligned with the Raven toolchain version **1.3.0** (see `src/main.rs` and `Cargo.toml`). Minor implementation details may evolve; when in doubt, refer to the parser and lexer sources under `src/`.
