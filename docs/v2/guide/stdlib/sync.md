# std/sync

Goroutine-style concurrency primitives: channels for passing values between
green threads, plus a mutex, a wait group, timed sleeping, and a multi-channel
receive. `std/sync` pairs with the `spawn` keyword, which starts a goroutine
running a `fun() -> Unit` closure. Goroutines run on an M:N scheduler that
spreads them across a pool of worker threads (one per available core), so they
run in parallel. Within a worker, scheduling is cooperative: a goroutine keeps
running until it hits a yield point (a blocking channel operation or an explicit
`yield_now()`), at which the worker resumes another ready goroutine.

```rust
import std/sync { channel, channel_buffered, yield_now }

fun main() {
    let ch = channel()
    spawn(fun() -> Unit {
        ch.send(42)
    })
    print(ch.recv())        // 42
}
```

Channels in this slice carry `Int` values.

## Importing

```rust
import std/sync { channel, channel_buffered, yield_now }
```

`channel`, `channel_buffered`, and `yield_now` are free functions, so import
the ones you use by name. `send` and `recv` are methods on `Channel` and need
no separate import once you hold a channel value.

The `spawn` keyword is part of the language itself and needs no import. It
takes a closure of type `fun() -> Unit` and starts it as a goroutine.

## Creating channels

### `channel() -> Channel`

Create an unbuffered (rendezvous) channel. A `send` blocks until a receiver is
ready to take the value, and a `recv` blocks until a sender hands one over. The
two sides meet: nothing is stored in between.

```rust
import std/sync { channel, channel_buffered, yield_now }

fun main() {
    let ch = channel()
    spawn(fun() -> Unit {
        ch.send(7)          // blocks until main receives
    })
    print(ch.recv())        // 7
}
```

### `channel_buffered(cap: Int) -> Channel`

Create a buffered channel of capacity `cap`. A `send` returns immediately while
there is room in the buffer, and only blocks once the buffer is full. A `recv`
takes the oldest buffered value, and blocks only when the buffer is empty.

```rust
import std/sync { channel, channel_buffered, yield_now }

fun main() {
    let ch = channel_buffered(2)
    ch.send(1)              // room: returns at once
    ch.send(2)              // room: returns at once
    print(ch.recv())        // 1
    print(ch.recv())        // 2
}
```

## Channel methods

### `send(self, value: Int)`

Send `value`, blocking until the channel can accept it. On an unbuffered
channel it blocks until a receiver is ready; on a buffered channel it blocks
only when the buffer is full. When a `send` blocks, the goroutine yields to the
scheduler so other goroutines can run.

### `recv(self) -> Int`

Receive a value, blocking until one is available. On an empty channel the
goroutine yields to the scheduler and resumes when a sender delivers a value.

```rust
import std/sync { channel, channel_buffered, yield_now }

fun main() {
    let ch = channel()
    spawn(fun() -> Unit {
        ch.send(10)
        ch.send(20)
    })
    print(ch.recv())        // 10
    print(ch.recv())        // 20
}
```

## Yielding

### `yield_now()`

Yield control to the scheduler so other ready goroutines can run, then resume
later. This is the explicit cooperative yield point. You rarely need it when
your goroutines communicate over channels, since `send` and `recv` already
yield when they block, but it is useful for handing off in a tight loop that
otherwise never reaches a blocking operation.

```rust
import std/sync { channel, channel_buffered, yield_now }

fun main() {
    spawn(fun() -> Unit {
        print(1)
        yield_now()
        print(3)
    })
    print(2)
}
```

## Pairing channels with `spawn`

`spawn` starts a goroutine from a `fun() -> Unit` closure. The closure can
capture channels from the surrounding scope and use them to communicate with
`main` (which is itself goroutine 0) or with other goroutines.

```rust
import std/sync { channel, channel_buffered, yield_now }

fun main() {
    let ch = channel()

    // Producer goroutine: send three values, then a sentinel.
    spawn(fun() -> Unit {
        ch.send(100)
        ch.send(200)
        ch.send(300)
        ch.send(-1)
    })

    // Main consumes until the sentinel.
    loop {
        let v = ch.recv()
        if v == -1 {
            break
        }
        print(v)            // 100, 200, 300
    }
}
```

When `main` returns the program exits, and any goroutines still alive (ready or
blocked) are abandoned without finishing. If every goroutine ends up blocked
with none ready, the scheduler reports a deadlock and exits.

## Worked example: a worker over a buffered channel

A buffered channel decouples the producer from the consumer so the producer can
get ahead while the consumer catches up.

```rust
import std/sync { channel_buffered, yield_now }

fun main() {
    let jobs = channel_buffered(4)

    // Worker: read each job and print its square.
    spawn(fun() -> Unit {
        loop {
            let n = jobs.recv()
            if n == 0 {
                break
            }
            print(n * n)        // 1, 4, 9, 16
        }
    })

    // Feed work, then a 0 to signal done.
    let k = 1
    while k <= 4 {
        jobs.send(k)
        k = k + 1
    }
    jobs.send(0)

    // Give the worker a turn to drain the channel before main exits.
    yield_now()
}
```

## Releasing a channel

### `free(self)`

Release a channel's runtime resources when you are done with it. A channel is
otherwise reclaimed when the program exits; `free` is for a long-running program
that creates many channels over time.

## Sleeping

### `sleep_millis(ms: Int)`

Block the current goroutine for `ms` milliseconds, yielding so other goroutines
run in the meantime.

```rust
import std/sync { sleep_millis }

fun main() {
    spawn(fun() -> Unit {
        sleep_millis(50)
        print(2)
    })
    print(1)
    sleep_millis(100)           // give the goroutine time to finish
}
```

## Mutex

A mutex guards a critical section so only one goroutine holds it at a time,
which matters now that goroutines run in parallel across worker threads.

### `mutex() -> Mutex`, `lock(self)`, `unlock(self)`

```rust
import std/sync { mutex }

fun main() {
    let m = mutex()
    m.lock()
    // ... exclusive section ...
    m.unlock()
}
```

## Wait group

A wait group counts outstanding work and lets one goroutine block until it
reaches zero, the usual way to wait for spawned goroutines to finish.

### `wait_group() -> WaitGroup`

Create a wait group with a zero counter.

### `add(self, n: Int)`, `done(self)`, `wait(self)`, `free(self)`

`add(n)` raises the counter by `n` (call it before spawning the work),
`done()` lowers it by one (call it as each unit finishes), `wait()` blocks
until the counter reaches zero, and `free()` releases the wait group.

```rust
import std/sync { wait_group, sleep_millis }

fun main() {
    let wg = wait_group()
    wg.add(2)
    spawn(fun() -> Unit {
        sleep_millis(20)
        wg.done()
    })
    spawn(fun() -> Unit {
        sleep_millis(20)
        wg.done()
    })
    wg.wait()                   // both goroutines have finished
    wg.free()
}
```

## Selecting over several channels

### `select_recv(channels: List<Channel>) -> SelectResult`

Block until one of `channels` has a value to receive, then return a
`SelectResult` with `index` (the position in the list of the channel that
produced the value, or `-1` when the list is empty) and `value` (that value).
When several channels are ready, the lowest index wins.

```rust
import std/sync { channel_buffered, select_recv }

fun main() {
    let a = channel_buffered(1)
    let b = channel_buffered(1)
    b.send(22)
    let chosen = select_recv([a, b])
    print(chosen.index)         // 1
    print(chosen.value)         // 22
}
```

## See also

- The [language reference](../language-reference.md#concurrency) for `spawn`,
  goroutines, the parallel scheduler, and deadlock behavior.
