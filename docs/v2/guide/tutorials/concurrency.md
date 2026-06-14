# Tutorial: concurrency with goroutines and channels

Raven runs concurrent work as goroutines: lightweight green threads scheduled
across a pool of OS threads. You start one with the `spawn` keyword and pass
values between them over channels from [`std/sync`](../stdlib/sync.md). This
tutorial builds up from a single goroutine to a parallel reduction, with a
detour through `select` and the synchronization primitives. Every step
compiles and runs.

A couple of facts to keep in mind as you go:

- `spawn` takes a `fun() -> Unit` closure and is a statement, not an
  expression: you write `spawn(...)` on its own line, you do not assign its
  result.
- Channels carry `Int` values. To move richer data, send an index or a count
  and keep the payload in shared state, or send the parts and reassemble them.

## Step 1: start a goroutine

`spawn` runs a closure concurrently. The closure can capture variables from
the surrounding scope:

```rust
import std/sync { channel }

fun main() {
    let ch = channel()
    spawn(fun() -> Unit {
        ch.send(42)
    })
    print(ch.recv())        // 42
}
```

`channel()` creates an unbuffered channel. `send` blocks until another
goroutine is ready to `recv`, and `recv` blocks until a value arrives, so the
two goroutines hand the value across and `main` prints `42`. Blocking here
means "yield to the scheduler," not "burn a thread": while one goroutine waits,
others run.

## Step 2: a producer and a consumer

A common shape is one goroutine producing a stream of values and another
consuming them. The producer sends `1` through `5`; `main` receives five values
and sums them:

```rust
import std/sync { channel }

fun main() {
    let ch = channel()
    spawn(fun() -> Unit {
        let i = 1
        while i <= 5 {
            ch.send(i)
            i = i + 1
        }
    })

    let sum = 0
    let n = 0
    while n < 5 {
        sum = sum + ch.recv()
        n = n + 1
    }
    print(sum)              // 15
}
```

The consumer counts how many values it expects (`5`) and stops there. There is
no "channel closed" signal in this model, so the receiver decides when it has
read enough, usually because it knows how many producers there are or how many
items each will send.

## Step 3: fan-in from several goroutines

Channels are many-to-one safe: several goroutines can send on the same channel
and one receiver collects the results. Order is not guaranteed, so design the
result to be order-independent (a sum, a count, a set):

```rust
import std/sync { channel }

fun main() {
    let fan = channel()
    spawn(fun() -> Unit {
        fan.send(3)
    })
    spawn(fun() -> Unit {
        fan.send(5)
    })
    spawn(fun() -> Unit {
        fan.send(7)
    })

    let total = 0
    let k = 0
    while k < 3 {
        total = total + fan.recv()
        k = k + 1
    }
    print(total)            // 15
}
```

Three goroutines each send one value; `main` receives exactly three and adds
them. The total is `15` no matter which goroutine runs first.

## Step 4: buffered channels and cooperative yielding

An unbuffered channel makes every `send` wait for a matching `recv`. A buffered
channel holds up to a fixed number of values, so a sender can get ahead of the
receiver:

```rust
import std/sync { channel_buffered, yield_now }

fun main() {
    let ch = channel_buffered(8)
    spawn(fun() -> Unit {
        ch.send(0)
        yield_now()
        ch.send(2)
        yield_now()
        ch.send(4)
    })
    ch.send(1)
    yield_now()
    ch.send(3)
    yield_now()
    ch.send(5)

    let acc = 0
    let m = 0
    while m < 6 {
        acc = acc + ch.recv()
        m = m + 1
    }
    print(acc)              // 15
}
```

`channel_buffered(8)` has room for eight pending values, so neither side blocks
on a full buffer here. `yield_now()` voluntarily hands the scheduler a chance
to run the other goroutine, interleaving the even and odd sends. The final sum
of `0..=5` is `15` regardless of the exact interleaving.

## Step 5: waiting on whichever channel is ready first

When a goroutine listens to more than one source, `select_recv` receives from
whichever channel has a value next. It returns a small result with the `value`
and the `index` of the channel it came from (its position in the list you
passed):

```rust
import std/sync { channel, select_recv }

fun main() {
    let a = channel()
    let b = channel()

    spawn(fun() -> Unit {
        a.send(10)
        a.send(20)
    })
    spawn(fun() -> Unit {
        b.send(30)
        b.send(40)
    })

    let total = 0
    let from_a = 0
    let from_b = 0
    let received = 0
    while received < 4 {
        let r = select_recv([a, b])
        total = total + r.value
        if r.index == 0 {
            from_a = from_a + 1
        }
        if r.index == 1 {
            from_b = from_b + 1
        }
        received = received + 1
    }

    print(total)            // 100
    print(from_a)           // 2
    print(from_b)           // 2
}
```

`select_recv` lets one consumer drain several producers fairly without deciding
up front which to read from. The value sum (`100`) and the per-channel counts
(two each) are deterministic even though the ready order is not.

## Step 6: synchronizing with a wait group and a mutex

Channels move values; sometimes you instead want many goroutines to update
shared state and a way to wait for them all to finish. `std/sync` provides a
`wait_group` to join a set of goroutines and a `mutex` to guard a shared value:

```rust
import std/sync { wait_group, mutex }

struct Counter {
    value: Int,
}

fun main() {
    let wg = wait_group()
    let m = mutex()
    let counter = Counter { value: 0 }

    wg.add(8)
    let spawned = 0
    while spawned < 8 {
        spawn(fun() -> Unit {
            let i = 0
            while i < 1000 {
                m.lock()
                counter.value = counter.value + 1
                m.unlock()
                i = i + 1
            }
            wg.done()
        })
        spawned = spawned + 1
    }

    wg.wait()
    print(counter.value)    // 8000
}
```

Eight goroutines each increment the shared counter a thousand times. The mutex
serializes the read-modify-write, so the total is exactly `8000`: without it,
parallel increments would race and lose updates. `wg.add(8)` declares how many
goroutines to wait for, each calls `wg.done()` as it finishes, and `wg.wait()`
blocks `main` until all eight are done.

## Step 7: a parallel reduction

Putting it together, here is a parallel sum. Eight workers each compute a
partial result and send it back; `main` adds the partials. Because each worker
allocates as it runs, this also exercises the garbage collector concurrently,
yet the answer stays exact:

```rust
import std/sync { Channel, channel }

fun worker(out: Channel, n: Int) -> Unit {
    let acc = 0
    let i = 0
    while i < n {
        let items = [i, i + 1, i + 2]
        acc = acc + items.len()
        i = i + 1
    }
    out.send(acc)
}

fun main() {
    let out = channel()
    let spawned = 0
    while spawned < 8 {
        spawn(fun() -> Unit {
            worker(out, 2000)
        })
        spawned = spawned + 1
    }

    let total = 0
    let received = 0
    while received < 8 {
        total = total + out.recv()
        received = received + 1
    }
    print(total)            // 48000
}
```

Each worker adds `3` to its accumulator `2000` times, so it sends `6000`; eight
workers sum to `48000`. Note the named `worker` function takes the channel by
its type `Channel` (imported from `std/sync`), and the `spawn` closure simply
calls it. A wrong total here would point at a real concurrency bug (a lost
value or a scheduler race), which is what makes a deterministic reduction a good
smoke test.

## Where to go next

- The [`std/sync` reference](../stdlib/sync.md) documents every channel, select,
  and synchronization function in detail.
- The [language reference](../language-reference.md#concurrency) covers the
  `spawn` keyword and the scheduling model.
- For CPU-bound pipelines, combine a buffered channel (Step 4) with a fixed pool
  of workers (Step 7) so producers and consumers run at their own pace.
