# std/process

Run external programs either to completion or as a live child. `run` waits and
returns captured output; `start` returns a `Child` that can exchange stdin,
drain stdout and stderr incrementally, poll or wait for exit, and be killed.

```rust
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

```rust
import std/process { run, run_with_input, start }
```

Bring in just the functions you call. `Output`, `Child`, and their methods
come in with the module and need no separate selectors.

## The `Output` type

```rust
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

```rust
import std/process { run }

fun main() {
    let no_args: List<String> = []
    match run("true", no_args) {
        Ok(out) -> print(out.success()),     // true
        Err(e) -> print(e.message),
    }
}
```

## The `Child` type

```rust
struct Child { id: Int }
```

`start` returns a handle to a child that may still be running. Its opaque
`id` is managed by the runtime; use the methods below instead of inspecting
the field.

### `read_stdout(self) -> String` and `read_stderr(self) -> String`

Drain all bytes accumulated on that stream since the previous read. An empty
string means nothing new is available. Each stream retains at most 8 MiB; keep
draining a noisy long-running child if every byte matters.

### `write_stdin(self, data: String) -> Bool`

Write and flush `data` to the child's stdin. It returns `false` if stdin was
closed, the child exited, the pipe broke, or the handle is no longer valid.
Raven strings are byte buffers, so binary data is passed through unchanged.

### `close_stdin(self)`

Close stdin and send EOF. Call this after the final write when the child waits
for end of input before producing output or exiting.

### `poll(self) -> Option<Int>` and `running(self) -> Bool`

`poll` returns `None` while the child runs and `Some(code)` once it exits.
`running` is the boolean form. A signal termination without an OS exit code is
reported as `-1`.

### `wait(self) -> Int`

Wait for exit and return the code. Waiting polls with a short scheduler-aware
sleep, so other Raven goroutines continue to run.

### `kill(self) -> Bool`

Stop the child. It returns `true` when the kill was delivered or the process
had already exited.

### `free(self)`

Release the handle after collecting the final output and status. `free` does
not kill a running process, although the runtime still guarantees it will be
reaped after exit.

## Running a program

### `start(program: String, args: List<String>) -> Result<Child, Error>`

Start a child without waiting. Runtime reader threads continuously collect
stdout and stderr so the process does not block on full pipes. Drain the
buffers while it runs and call `free` when finished.

```rust
import std/process { start }
import std/sync { sleep_millis }

fun main() {
    let args = ["--version"]
    match start("git", args) {
        Ok(child) -> {
            while child.running() {
                let chunk = child.read_stdout()
                if chunk.length() > 0 {
                    print(chunk)
                }
                sleep_millis(10)
            }
            print(child.read_stdout())       // drain bytes written before exit
            let code = child.wait()
            child.free()
            print("exit ${code}")
        },
        Err(e) -> print("could not start git: ${e.message}"),
    }
}
```

For an interactive child, call `write_stdin` as needed and then
`close_stdin` to send EOF:

```rust
import std/process { start }

fun main() {
    let no_args: List<String> = []
    match start("cat", no_args) {
        Ok(child) -> {
            child.write_stdin("hello\n")
            child.close_stdin()
            let code = child.wait()
            print(child.read_stdout())
            child.free()
            print("exit ${code}")
        },
        Err(e) -> print(e.message),
    }
}
```

### `run(program: String, args: List<String>) -> Result<Output, Error>`

Spawn `program` with `args`, feed it no stdin, and wait for it to finish. On
success returns `Ok(Output)` with the captured code, stdout, and stderr.

`args` is the argument list with no leading program name (that is `program`).
When a program takes no arguments, pass an explicitly typed empty list so the
element type is known:

```rust
let no_args: List<String> = []
```

A bare `[]` has no element type the checker can infer here, so the annotation
is required.

```rust
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

```rust
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

```rust
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

```rust
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

- `run` and `run_with_input` capture stdout and stderr in full before
  returning. Use `start` for incremental output or interactive stdin.
- An argument that itself contains a NUL byte (`\0`) is not supported.
- A child that reads no stdin under `run` sees end of input immediately.
  Under `run_with_input`, a broken pipe (the child exits without reading) is
  ignored and you still get its captured output.

## See also

- [std/error](error.md) for the `Error` type and the `Result` model used by
  `run`, `run_with_input`, and `start`.
- [std/sync](sync.md) for scheduler-aware sleeps while polling a child.
- [std/string](string.md) for trimming and inspecting captured output.
