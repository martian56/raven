# std/process

Run external programs and read back their output. A run spawns the program,
waits for it to finish, and hands you the captured exit code, stdout, and
stderr as an `Output` value. There is no streaming: a child runs to
completion in a single call.

```raven
import std/process { run }

fun main() {
    let args = ["--version"]
    match run("git", args) {
        Ok(out) -> print(out.stdout),
        Err(e) -> print("could not run git: ${e.message}"),
    }
}
```

## Importing

```raven
import std/process { run, run_with_input }
```

Bring in just the functions you call. The `success` method on `Output` comes
in with the type and needs no separate selector.

## The `Output` type

```raven
struct Output { code: Int, stdout: String, stderr: String }
```

A finished child's captured result.

| Field | Meaning |
|-------|---------|
| `code` | The exit code. `0` is a clean exit; any other value is the program's own status. A child killed by a signal with no exit code reports `-1`. |
| `stdout` | Everything the child wrote to standard output, captured in full. |
| `stderr` | Everything the child wrote to standard error, captured in full. |

### `success(self) -> Bool`

True when `code` is `0`.

```raven
import std/process { run }

fun main() {
    let no_args: List<String> = []
    match run("true", no_args) {
        Ok(out) -> print(out.success()),     // true
        Err(e) -> print(e.message),
    }
}
```

## Running a program

### `run(program: String, args: List<String>) -> Result<Output, Error>`

Spawn `program` with `args`, feed it no stdin, and wait for it to finish. On
success returns `Ok(Output)` with the captured code, stdout, and stderr.

`args` is the argument list with no leading program name (that is `program`).
When a program takes no arguments, pass an explicitly typed empty list so the
element type is known:

```raven
let no_args: List<String> = []
```

A bare `[]` has no element type the checker can infer here, so the annotation
is required.

```raven
import std/process { run }

fun main() {
    let args = ["-l", "/tmp"]
    match run("ls", args) {
        Ok(out) -> print(out.stdout),
        Err(e) -> print("spawn failed: ${e.message}"),
    }
}
```

### `run_with_input(program: String, args: List<String>, input: String) -> Result<Output, Error>`

Like `run`, but writes `input` to the child's stdin and then closes it. Use
this for programs that read from standard input.

```raven
import std/process { run_with_input }

fun main() {
    let no_args: List<String> = []
    match run_with_input("cat", no_args, "hello\n") {
        Ok(out) -> print(out.stdout),        // hello
        Err(e) -> print(e.message),
    }
}
```

## A non-zero exit is not an error

A program that spawns and runs but exits with a non-zero code is a normal,
successful run: its code and output are captured and `run` returns
`Ok(Output)`. You decide what a non-zero exit means by inspecting `code` (or
calling `success()`).

The `Err` path is taken only on a spawn failure, for example when the program
is not found or cannot be executed. The error is
`Err(Error { kind: "process", message })`, where `message` is the OS error
text. The `Error` type comes from [std/error](error.md).

```raven
import std/process { run }

fun main() {
    let args = ["status", "--short"]
    match run("git", args) {
        Ok(out) -> {
            if out.success() {
                print(out.stdout)
            } else {
                // The program ran but reported a non-zero status.
                print("git exited with ${out.code}")
                print(out.stderr)
            }
        },
        Err(e) -> print("could not start git: ${e.message}"),
    }
}
```

## Worked example: read the current branch

Run `git` to print the current branch name, falling back to a label when the
command is missing or reports a failure.

```raven
import std/process { run }
import std/string

fun current_branch() -> String {
    let args = ["rev-parse", "--abbrev-ref", "HEAD"]
    return match run("git", args) {
        Ok(out) -> {
            if out.success() {
                out.stdout.trim()
            } else {
                "unknown"
            }
        },
        Err(_) -> "no git",
    }
}

fun main() {
    print(current_branch())
}
```

## Notes

- A run captures stdout and stderr in full before returning. There is no
  incremental or streaming output.
- An argument that itself contains a NUL byte (`\0`) is not supported.
- A child that reads no stdin under `run` sees end of input immediately.
  Under `run_with_input`, a broken pipe (the child exits without reading) is
  ignored and you still get its captured output.

## See also

- [std/error](error.md) for the `Error` type and the `Result` model used by
  `run` and `run_with_input`.
- [std/string](string.md) for trimming and inspecting captured output.
