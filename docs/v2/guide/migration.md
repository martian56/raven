# Migrating from v1

Raven v2 is a compiled language with a stricter type system, generics,
traits, and a package manager. This page maps the v1 surface to v2 so you
can update existing code. It is practical rather than exhaustive; see the
[language reference](language-reference.md) for the full v2 surface.

## Quick reference

| v1 | v2 |
|----|----|
| `int`, `float`, `bool`, `string` | `Int`, `Float`, `Bool`, `String` |
| `int[]` | `List<Int>` |
| statements end with `;` | newlines separate statements; `;` optional |
| `fun main() -> void { ... }` then `main();` | `fun main() { ... }` runs automatically |
| `export` | removed; modules expose declared items directly |
| sentinel return values for errors | `Result<T, E>` and `Option<T>` |
| C-style `for (i = 0; i < n; i = i + 1)` | `for i in 0..n` |
| top level statements | code runs from `fun main()` |

## Types are PascalCase

Primitive type names changed from lowercase to PascalCase:

```raven
// v1
let name: string = "Raven";
let count: int = 0;

// v2
let name: String = "Raven"
let count: Int = 0
```

Array types become the generic `List<T>`:

```raven
// v1
let numbers: int[] = [1, 2, 3];

// v2
let numbers: List<Int> = [1, 2, 3]
```

## No semicolons, no top level execution

v2 separates statements with newlines, and semicolons are optional. There
is no top level statement execution and no trailing `main();` call:
execution begins at `fun main()`.

```raven
// v1
fun main() -> void {
    print("hi");
}
main();

// v2
fun main() {
    print("hi")
}
```

A function with no return type returns `Unit`, so the `-> void`
annotation is dropped.

## export is gone

v1 used `export` to mark module items as public. v2 has no `export`. A
module exposes its declared functions, structs, enums, and traits; a
consumer imports them by name with a selective import.

```raven
// v2 consumer
import std/io { println }
import "./helpers" { greet }
```

## Errors use Result and Option

v1 signaled failure with sentinel return values. v2 uses `Result<T, E>`
for fallible operations and `Option<T>` for optional values, matched with
their variant names and propagated with `?`.

```raven
import std/error { error }

fun divide(a: Int, b: Int) -> Result<Int, Error> {
    if b == 0 {
        return Err(error("divide by zero"))
    }
    return Ok(a / b)
}

fun main() {
    match divide(10, 2) {
        Ok(v) -> print(v),
        Err(e) -> print(e.message()),
    }
}
```

## Loops use for ... in

The C-style `for` is replaced by `for ... in` over a range or a list.
Ranges are `a..b` (half open) and `a..=b` (inclusive).

```raven
// v1
for (i = 0; i < 5; i = i + 1) {
    print(i);
}

// v2
for i in 0..5 {
    print(i)
}
```

## New in v2

These constructs have no v1 equivalent:

- Generics with trait bounds: `fun show<T: ToString>(x: T) -> String`.
- Traits and `impl Trait for Type`, plus `dyn Trait` for runtime dispatch.
- `match` with exhaustive patterns and guards.
- Associated functions as constructors: `Type.new()`.
- Lazy iterators in `std/iter`.
- `defer` for cleanup that runs on function return.
- Package imports through `rvpm`: `import "github.com/<user>/<repo>"`.

## Module imports

v1 imported a whole module by name. v2 standard library modules use a
`std/...` path with a selective import for free functions, a bare import
for modules that add methods or constructors (`std/string`,
`std/collections`), and quoted paths for local (`./`) and GitHub
dependencies.

```raven
import std/io { println }
import std/string
import std/collections
import "./util"
```
