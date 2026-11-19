# Raven syntax redesign: Python with braces

Date: 2026-11-20. Status: agreed with the maintainer.

## Intent

Raven should read and write like Python while keeping braces, compile at
Go-class speed, and run close to C. Breaking changes are accepted; existing
code moves over with an automatic migration (`rvpm migrate`).

## Decisions

| Area | Decision |
|------|----------|
| Blocks | Braces, always. Newlines end statements; `;` optional. |
| Functions | `fun name(a: int, b: int = 3) -> int { ... }`; one-line `fun f(x: int) -> int = x * x`. |
| Variables | `let` immutable, `var` mutable. Types inferred; `let x: int = 1` when wanted. |
| Built-in types | lowercase: `int`, `float`, `bool`, `str`, `char`, `list[T]`, `dict[K, V]`, `set[T]`, `Option[T]`, `Result[T, E]`, tuples `(int, str)`. |
| Sized numbers | `i8 i16 i32 i64 u8 u16 u32 u64 f32 f64`; `x as i32` casts. |
| Overflow | Panics in debug builds, wraps in release builds. |
| Generics | Square brackets: `fun first[T](xs: list[T]) -> Option[T]`, bounds inline `[T: Eq + Hash]`. |
| Logic | `and` / `or` / `not`, and `&&` / `||` / `!` also accepted; the formatter writes the words. |
| Lambdas | `x => x * 2`, `(a, b) => a + b`, `() => { ... }`. |
| Strings | `"hello ${name}"`, format specs `"${price:.2}"`. |
| Comments | `//` and `/* */`. |
| Types | `class Point { x: float = 0.0  fun len(self) -> float { ... } }`. Fields and methods together. Every value of a class is a reference. |
| Traits | `trait Shape { fun area(self) -> float }`; `class Circle: Shape { ... }` implements it. No inheritance. Default methods allowed. |
| Foreign impls | `impl Trait for int { ... }` and `impl int { ... }` for types you did not declare. |
| Construction | Call syntax: `Point(1.0, 2.0)`, `Point(x: 1.0, y: 2.0)`, update `Point(..p, y: 5.0)`. Field defaults fill omitted fields. |
| Arguments | Defaults and named arguments: `connect("db", retries: 5)`. |
| Receiver | Explicit `self`; a method without `self` is static: `Point.origin()`. |
| Enums | `enum Shape { Circle(r: float), Rect(w: float, h: float), Dot }`; construction always qualified `Shape.Circle(2.0)`. |
| Match | Statement with case blocks: `match s { case Shape.Circle(r) { ... } case _ { ... } }`. Patterns: literals, ranges, `_`, variants, tuples, guards `case x if x > 0`. |
| Control flow | `if`/`match`/blocks are statements. Conditional value: `a if cond else b`. `if let`, `while let`, `let ... else` kept. |
| Loops | `for x in xs`, `for i in 0..n`, `0..=n`, `(0..n).step(2)`, `for (i, x) in xs.enumerate()`, `for (k, v) in d`. `while`, `loop`, `break`, `continue`. |
| Literals | `[1, 2]`, `{"a": 1}`, `{1, 2}`, `{}` is an empty dict, `set()` an empty set, `(1, "a")` a tuple, `()` unit. |
| Errors | `Result[T, E]` with `?`. `panic(msg)` aborts. |
| Absence | `Option[T]` with `Some(x)` / `None`. |
| Modules | `import std.fs`, `from std.json import parse, stringify`, `from "./models" import User`. Private by default, `pub` exports, `pub import` re-exports. |
| Operators | Classes overload operators through methods: `add sub mul div mod neg eq lt le gt ge index`. |
| Equality etc. | Classes and enums whose fields allow it get `==`, hashing, and printing automatically. `@derive(Ord, Debug, ToJson, FromJson)` for the rest. |
| Concurrency | `spawn { ... }` returns a handle; `Channel[T]` carries any type. |

## Performance goals

- Compile: the 10k-line benchmark corpus builds in well under a second
  (Go-class), measured by `benchmarks/compile-time`.
- Runtime: numeric and allocation benchmarks within a small factor of C and
  at or better than Go, measured by `benchmarks/runtime`.

## Migration

`rvpm migrate` parses old syntax with the legacy parser and prints the new
syntax with the formatter, so the stdlib, examples, tests, and packages move
over mechanically.
