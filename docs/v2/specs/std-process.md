# std/process Spec

Spawn a subprocess and either run it to completion (`run`) or keep it
running while its output streams in (`start`). The primitives bind the
raven-runtime C ABI (backed by `std::process::Command`); the wrappers add
the Result/Error model and the `Output` and `Child` value types in pure
Raven.

## Import

```rust
import std/process { run, run_with_input, start, Child }
```

The methods on `Output` and `Child` come in with the types and need no
separate selectors.

## Surface

```rust
struct Output { code: Int, stdout: String, stderr: String }
struct Child { id: Int }

fun run(program: String, args: List<String>) -> Result<Output, Error>
fun run_with_input(program: String, args: List<String>, input: String) -> Result<Output, Error>
fun start(program: String, args: List<String>) -> Result<Child, Error>

impl Output {
    fun success(self) -> Bool   // code == 0
}

impl Child {
    fun read_stdout(self) -> String   // drained since the last read, "" when nothing new
    fun read_stderr(self) -> String
    fun poll(self) -> Option<Int>     // None while running, Some(code) once exited
    fun running(self) -> Bool
    fun wait(self) -> Int             // block (polling) until exit
    fun kill(self) -> Bool
    fun free(self)                    // drop the runtime entry; does not kill
}
```

`run` spawns `program` with `args`, feeds it no stdin, waits for it to
finish, and returns its captured output. `run_with_input` is the same but
writes `input` to the child's stdin (which is then closed). `start` spawns
the child and returns immediately with a handle. (`spawn` is the goroutine
keyword, hence `start`.)

## Two capture models

A `run` executes the child to completion in a single runtime call that
captures the whole of stdout and stderr into an owned result before
returning.

A `start` leaves the child running: runtime reader threads pump its stdout
and stderr into growable buffers as they arrive, and each `read_stdout` /
`read_stderr` call drains what accumulated since the previous read. `poll`
try-waits without blocking (`running` is its Bool shorthand); `wait` loops
poll with a short scheduler-friendly sleep so other goroutines proceed.
`kill` terminates the child (killing an already-exited child reports
success). `free` drops the registry entry without killing, so a caller that
wants the child stopped must `kill` first. The child's stdin is null: it
sees end of input immediately and does not inherit the parent's terminal.

## Argument encoding (NUL-joined)

A Raven `List` cannot cross the C ABI, so `run` joins the args into one
String with a single NUL byte (`\0`) between elements and the runtime splits
that String back into the child's argument vector. An empty list is the
empty String (zero args). Program arguments effectively never contain a NUL
byte, so the encoding is unambiguous; an argument that itself contains a NUL
byte is not supported.

## Handle registry model

A finished child's output cannot cross the FFI as a struct, so the runtime
captures the exit code, stdout, and stderr into an entry in a process-wide
registry keyed by an incrementing `i64` id and hands Raven only that id. The
wrappers read the three fields by id through the extractors, then free the
entry, so a caller never sees the id. Ids start at 1; an id of 0 is the
spawn-failure sentinel paired with a set last-error.

Started children live in a second registry sharing the same id sequence
(so an id is unique across both): the entry holds the live process handle,
the two output buffers the reader threads fill, and the exit code once
observed, cached so polling after exit stays cheap. `poll` reports `-2`
over the FFI while the child runs; the wrapper maps that to `None`.

## Non-zero exit is not an error

A child that spawns and runs but exits with a non-zero code is a normal,
successful run: its code and output are captured and `run` returns
`Ok(Output)`. The caller inspects `code` (or `success()`) to decide what a
non-zero exit means. Only a spawn failure (for example the program is not
found, or it cannot be executed) takes the `Err` path.

## Error model

A spawn failure returns `Err(Error { kind: "process", message })`, the
message being the OS error text the runtime captured in its thread-local
last-error slot. The `Error` value type comes from `std/error`.

## Signal termination

A child terminated by a signal with no exit code (a Unix concern; Windows
processes always carry a code) reports a code of `-1`. This sentinel also
covers an unknown id.

## Stdin

`run` writes nothing to the child's stdin and closes it immediately, so a
child that reads stdin sees end of input at once. `run_with_input` writes
the given `input` and then closes stdin. A broken pipe (the child exits
without reading) is ignored: the run still yields the child's captured
output.
