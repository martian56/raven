# std.signal

Notice when the process is asked to stop: Ctrl+C, `SIGINT`, `SIGTERM`, or the
console closing on Windows. A server uses it to stop taking requests, finish
the ones in flight, close its database pool, and exit on its own terms.

```rust
from std.signal import on_interrupt

fun main() {
    let stops = on_interrupt()
    // start the server in a goroutine here
    if stops.wait_timeout(10) {
        print("stopping")
    }
    stops.stop()
}
```

## Functions

| Call | Does |
|------|------|
| `on_interrupt() -> Interrupts` | Starts a subscription. While one is open, a stop request no longer ends the process. |
| `interrupts.wait()` | Blocks until the next stop request. |
| `interrupts.wait_timeout(ms) -> bool` | Waits up to `ms` milliseconds; true if a request arrived. |
| `interrupts.stop()` | Ends the subscription. With none left, a stop request ends the process again (exit code 130). |
| `wait_for_interrupt()` | Blocks until a stop request, then ends its subscription. |

Waiting parks the goroutine, not its thread. An open subscription also keeps
the deadlock detector from stopping a program whose goroutines are all
waiting, because a stop request can still arrive.

## Graceful shutdown

```rust
from std.signal import on_interrupt
from std.sync import WaitGroup, sleep_millis, AtomicInt

fun worker(running: AtomicInt, wg: WaitGroup) {
    while running.load() == 1 {
        sleep_millis(5) // take a job, handle it
    }
    wg.done()
}

fun main() {
    let stops = on_interrupt()
    let running = AtomicInt.new(1)
    let wg = WaitGroup.new()
    wg.add(4)
    for _ in range(4) {
        spawn worker(running, wg)
    }
    let _ = stops.wait_timeout(20) // in a real server: stops.wait()
    running.store(0)
    wg.wait()
    stops.stop()
    print("all workers finished")
}
```

Stop requests are not told apart: Ctrl+C and `SIGTERM` arrive the same way.
`SIGKILL` cannot be caught by any program.
