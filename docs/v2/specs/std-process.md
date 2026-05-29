# std/process Spec

Spawn a subprocess, run it to completion, and read its exit code plus
captured stdout and stderr. Optionally feed data to the child's stdin. The
primitives bind the raven-runtime C ABI (backed by `std::process::Command`);
the wrappers add the Result/Error model and the `Output` value type in pure
Raven.

## Import

```raven
import std/process { run, run_with_input }
```

The `success` method on `Output` comes in with the type and needs no
separate selector.

## Surface

```raven
struct Output { code: Int, stdout: String, stderr: String }

fun run(program: String, args: List<String>) -> Result<Output, Error>
fun run_with_input(program: String, args: List<String>, input: String) -> Result<Output, Error>

impl Output {
    fun success(self) -> Bool   // code == 0
}
```

`run` spawns `program` with `args`, feeds it no stdin, waits for it to
finish, and returns its captured output. `run_with_input` is the same but
writes `input` to the child's stdin (which is then closed).

## Run-to-completion model

There is no streaming in v2.0. A run executes the child to completion in a
single runtime call that captures the whole of stdout and stderr into an
owned result before returning. A caller that needs incremental output is
not served by this surface.

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
