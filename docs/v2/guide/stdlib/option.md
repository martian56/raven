# std/option

Free combinator functions over the built-in `Option<T>`. `Option` and its
variants `Some` and `None` are part of the language and need no import for the
type itself; this module adds the common combinators, since methods cannot be
attached to the built-in generic type.

```rust
import std/option

fun main() {
    let a: Option<Int> = Some(5)
    print(option.unwrap_or(a, 0))       // 5
    match option.map(a, fun(x: Int) -> Int = x * 2) {
        Some(v) -> print(v),            // 10
        None -> print("none"),
    }
}
```

## Importing

```rust
import std/option
```

A bare `import std/option` binds the module alias, so the functions are called
as `option.is_some(...)`. To call them unqualified, list the names in a
selective import:

```rust
import std/option { is_some, map, unwrap_or }
```

`Option<T>`, `Some`, and `None` are built into the language and need no import.

## Inspecting

### `is_some<T>(o: Option<T>) -> Bool`

True when the option holds a value.

### `is_none<T>(o: Option<T>) -> Bool`

True when the option is empty.

```rust
import std/option

fun main() {
    let a: Option<Int> = Some(5)
    let n: Option<Int> = None
    print(option.is_some(a))    // true
    print(option.is_none(n))    // true
}
```

## Unwrapping

### `unwrap_or<T>(o: Option<T>, default: T) -> T`

The contained value, or `default` when the option is `None`.

```rust
import std/option

fun main() {
    let a: Option<Int> = Some(5)
    let n: Option<Int> = None
    print(option.unwrap_or(a, 0))   // 5
    print(option.unwrap_or(n, 0))   // 0
}
```

## Transforming

### `map<T, U>(o: Option<T>, f: fun(T) -> U) -> Option<U>`

Apply `f` to the contained value, or pass `None` through. `f` is a lambda.

### `and_then<T, U>(o: Option<T>, f: fun(T) -> Option<U>) -> Option<U>`

Chain an option-returning function: `Some(v)` becomes `f(v)`, `None` stays
`None`. The flat-map combinator.

```rust
import std/option

fun main() {
    let a: Option<Int> = Some(5)
    match option.map(a, fun(x: Int) -> Int = x * 2) {
        Some(v) -> print(v),            // 10
        None -> print("none"),
    }
    match option.and_then(a, fun(x: Int) -> Option<Int> = Some(x + 1)) {
        Some(v) -> print(v),            // 6
        None -> print("none"),
    }
}
```

### `filter<T>(o: Option<T>, pred: fun(T) -> Bool) -> Option<T>`

Keep the value only when it satisfies `pred`, otherwise `None`.

### `or_else<T>(o: Option<T>, f: fun() -> Option<T>) -> Option<T>`

The option itself when it is `Some`, otherwise the result of calling `f()`.
`f` is a zero-arg lambda returning an `Option`, so the fallback is built only
when needed.

```rust
import std/option

fun main() {
    let a: Option<Int> = Some(5)
    let n: Option<Int> = None
    match option.filter(a, fun(x: Int) -> Bool = x > 10) {
        Some(v) -> print(v),
        None -> print("filtered"),      // filtered
    }
    match option.or_else(n, fun() -> Option<Int> = Some(99)) {
        Some(v) -> print(v),            // 99
        None -> print("none"),
    }
}
```

## See also

- [std/error](error.md) for `Result` combinators and the `ok` / `err` bridges
  between `Result` and `Option`.
- [std/cmp](cmp.md), whose `min_of` and `max_of` return an `Option`.
- The [language reference](../language-reference.md) for `Option`, `Some`,
  `None`, `match`, and the `?` operator.
