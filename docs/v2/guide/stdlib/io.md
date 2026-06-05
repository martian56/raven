# std/io

Console input and output: write lines to standard output and read lines from
standard input. The functions here are free functions, so you import the ones
you need by name.

```rust
import std/io { println, input }

fun main() {
    let name = input("Your name: ")
    println("Hello, ${name}")
}
```

## Importing

```rust
import std/io { print, println, input, read_line }
```

`std/io` is a set of free functions, so use a selective import and list the
names you want. A bare `import std/io` does not bring the functions into
scope.

### The global `print` builtin

You do not need `std/io` to print. The `print` builtin is always in scope and
accepts any value whose type implements `ToString`, so it works on numbers,
booleans, and your own types without conversion:

```rust
fun main() {
    print(42)               // 42
    print(true)             // true
    print(3.14)             // 3.14
}
```

The `print` function from `std/io` is the `String`-only form below. The
always-available builtin is the more convenient choice for most output; reach
for `std/io` when you want `println`, `input`, or `read_line`.

## Output

### `print(s: String)`

Write `s` to standard output with no trailing newline. The argument must be a
`String`; build one with interpolation or [std/string](string.md) methods if
you have other values to include.

```rust
import std/io { print }

fun main() {
    print("no newline here")
    print(" and more on the same line")
}
```

### `println(s: String)`

Write `s` to standard output followed by a newline.

```rust
import std/io { println }

fun main() {
    println("first line")
    println("second line")
}
```

## Input

### `input(prompt: String) -> String`

Write `prompt` to standard output with no trailing newline, then read one line
from standard input and return it without its trailing newline. At end of
input the returned string is empty.

```rust
import std/io { input }

fun main() {
    let answer = input("Continue? (y/n) ")
    if answer == "y" {
        print("ok")
    }
}
```

### `read_line() -> String`

Read one line from standard input and return it without its trailing newline,
printing no prompt. At end of input the returned string is empty, which is how
you detect that the stream is exhausted.

```rust
import std/io { read_line }
import std/string

fun main() {
    let line = read_line()
    print(line.trim().to_upper())
}
```

## Worked example: echo non-empty lines

Read lines until the input ends, skipping blank ones. The empty string from
`read_line` at end of input ends the loop.

```rust
import std/io { read_line, println }
import std/string

fun main() {
    let line = read_line()
    while !line.is_blank() {
        println(line.trim())
        line = read_line()
    }
}
```

## See also

- [std/string](string.md) for building and trimming the strings you read and
  print.
- [std/fmt](fmt.md) for padding, joining, and number formatting before
  output.
- [std/fs](fs.md) when you want to read from and write to files instead of the
  console.
</content>
</invoke>
