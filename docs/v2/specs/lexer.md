# Lexer Spec

## Goal

Tokenize Raven v2 source text into a flat stream of `Token` values with byte ranges and line/column positions, suitable for consumption by the parser. The lexer recognizes every lexical construct in the v2 grammar (keywords, identifiers, literals, operators, punctuation, comments, newlines), reports lex errors with precise spans, and produces output deterministically for any well formed UTF-8 input.

## Token kinds

The `TokenKind` enum has one variant per category below.

* `Identifier(String)`: any `[a-zA-Z_][a-zA-Z0-9_]*` not matching a keyword.
* Keywords (each its own variant): `Let`, `Const`, `Fun`, `Return`, `If`, `Else`, `While`, `For`, `Loop`, `In`, `Break`, `Continue`, `Match`, `Struct`, `Trait`, `Impl`, `Enum`, `Import`, `As`, `Extern`, `Defer`, `True`, `False`, `Self_` (lowercase `self`), `SelfType` (uppercase `Self`).
* `IntLit(i64)`: integer literal, already parsed to `i64`. Bases 10, 16 (`0x`), 2 (`0b`), 8 (`0o`). Underscores stripped.
* `FloatLit(f64)`: float literal, already parsed to `f64`. Optional decimal part and optional `e[+-]?digits` exponent.
* `StringLit(String)`: regular `"..."` string. Escapes are processed and `${...}` interpolation fragments are kept verbatim inside the cooked text. The parser splits interpolation later.
* `BlockStringLit(String)`: `"""..."""` raw block string. No escape processing. Preserves newlines and whitespace exactly.
* `CharLit(char)`: `'...'` containing exactly one Unicode scalar value, escapes processed.
* `CStringLit(String)`: `c"..."` FFI string. Escapes processed. The null terminator is appended by codegen, not by the lexer.
* Operators (each its own variant): `Plus`, `Minus`, `Star`, `Slash`, `Percent`, `PlusEq`, `MinusEq`, `StarEq`, `SlashEq`, `PercentEq`, `EqEq`, `NotEq`, `Lt`, `Gt`, `LtEq`, `GtEq`, `AndAnd`, `OrOr`, `Bang`, `Amp`, `Pipe`, `Caret`, `Tilde`, `Shl`, `Shr`, `AmpEq`, `PipeEq`, `CaretEq`, `ShlEq`, `ShrEq`, `Eq`, `DotDot`, `DotDotEq`, `Question`, `Arrow` (`->`), `FatArrow` (`=>`), `ColonColon`, `Dot`.
* Punctuation: `LParen`, `RParen`, `LBrace`, `RBrace`, `LBracket`, `RBracket`, `Comma`, `Semi`, `Colon`, `At`.
* `Newline`: one or more consecutive line terminators collapsed into one token.
* `Eof`: zero width sentinel at end of source.

Built in type names like `Int`, `String`, `Bool` are NOT keywords. They are regular `Identifier` tokens, recognized by the type checker by context.

## Span representation

```rust
pub struct Span {
    pub file: Arc<PathBuf>,
    pub start: usize,   // inclusive byte offset
    pub end: usize,     // exclusive byte offset
    pub line: u32,      // 1 indexed line of `start`
    pub col: u32,       // 1 indexed column of `start`, counted in chars
}
```

Convention: `start..end` is a half open byte range into the source string. `line` and `col` point to the first character of the span. `Display for Span` formats as `path:line:col`.

## Error model

`RavenError` is the top level error enum, currently with a single variant:

```rust
pub enum RavenError {
    Lex(LexError, Span, Option<String>),
}
```

The optional third field is an error hint, printed on the line below the source pointer when present.

```rust
pub enum LexError {
    UnexpectedChar(char),
    UnterminatedString,
    UnterminatedBlockString,
    UnterminatedBlockComment,
    InvalidEscape(char),
    InvalidUnicodeEscape,
    InvalidNumber(String),
    InvalidCharLit(String),
}
```

`RavenError::display(&self, source: &str) -> String` produces a colored, multi line message: red header with the error kind and location, a context line of source, and a row of red `^` carets under the offending span. If a hint is present, it appears dim on the line below.

## Edge cases handled

* Integer underscores: `1_000_000`, `0xff_ff`. Stripped before `i64::from_str_radix`.
* Float exponent without decimal: `1e10` is a valid float.
* Float without exponent but with decimal: `3.14`.
* Leading dot floats (`.5`) are NOT valid; report `InvalidNumber`.
* `\r\n` and bare `\r` are both treated as one newline. Consecutive newlines coalesce into one `Newline` token spanning the whole run.
* Escapes: `\n`, `\t`, `\r`, `\\`, `\"`, `\'`, `\0`, `\x41` (two hex digits), `\u{1F600}` (1 to 6 hex digits).
* `${...}` inside a regular string is preserved verbatim in the cooked text; the parser splits interpolation later.
* Triple quoted block strings (`"""..."""`) are raw: no escape processing, newlines preserved.
* Char literal must contain exactly one Unicode scalar value after escape processing.
* `c"..."` is recognized only when `c` is immediately followed by a double quote.
* Longest match wins for operators: `..=` before `..`, `<<=` before `<<` before `<=` before `<`, etc.
* Line comments `//` consume to end of line but do not include the newline.
* Block comments `/* ... */` do NOT nest; an unterminated block comment is `UnterminatedBlockComment`.

## Out of scope

* String interpolation splitting. The parser receives the cooked string verbatim with `${...}` segments inside and re tokenizes those segments itself.
* Automatic semicolon insertion. The parser consumes or ignores `Newline` tokens based on grammar context.
* Macro expansion, raw string sigils beyond `c"..."`, byte string literals, and number suffixes (`42i32`). Not in v2.0 scope.

## Test coverage

Every token category has at least one positive unit test, every error variant has at least one negative test, and the operator tests cover longest match boundaries. `RavenError::display` has its own snapshot test confirming the colored pointer renders with the expected layout.
