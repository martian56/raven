# Tutorial: modeling data with structs and enums

This tutorial builds a small in-memory task tracker. It is a tour of Raven's
type system: structs to group data, enums (sum types) to model a value that is
one of several shapes, methods with `impl`, exhaustive `match`, and `Option`
for a lookup that might find nothing. Every step compiles and runs.

If you are new to the language, skim the
[language reference](../language-reference.md) for syntax as you go.

## Step 1: a struct for a task

A struct groups related fields under one type. A task has an id and a title:

```rust
struct Task {
    id: Int,
    title: String,
}

fun main() {
    let t = Task { id: 1, title: "write docs" }
    print("#${t.id} ${t.title}")        // #1 write docs
}
```

You build a struct value with `Type { field: value, ... }` and read a field
with `value.field`. Types are checked: leaving out a field or giving one the
wrong type is a compile error, not a surprise at runtime.

## Step 2: a status that is one of several

A task has a status, and a status is exactly one of a fixed set: to do, doing,
or done. That is a sum type, written as an `enum`. Add it and a field for it:

```rust
enum Status {
    Todo,
    Doing,
    Done,
}

struct Task {
    id: Int,
    title: String,
    status: Status,
}

fun main() {
    let t = Task { id: 1, title: "write docs", status: Status.Doing }
    print(t.title)
}
```

You construct a variant with the qualified form `Status.Doing`. To turn a
status into text, use `match`, which checks that you handle every variant:

```rust
fun label(s: Status) -> String {
    return match s {
        Todo -> "todo",
        Doing -> "doing",
        Done -> "done",
    }
}
```

In a `match` pattern the variants are written bare (`Todo`, not
`Status.Todo`). If you add a fourth variant later and forget to handle it
here, the compiler rejects the program until you do. That is exhaustiveness
checking, and it is one of the main reasons to reach for an enum.

## Step 3: a variant that carries data

Some statuses need more than a name. A blocked task should say what it is
waiting on. A variant can carry a payload, so give `Blocked` a `String`:

```rust
enum Status {
    Todo,
    Doing,
    Done,
    Blocked(String),
}
```

Now a `match` arm for `Blocked` binds the payload to a name you can use:

```rust
fun label(s: Status) -> String {
    return match s {
        Todo -> "todo",
        Doing -> "doing",
        Done -> "done",
        Blocked(reason) -> "blocked: ${reason}",
    }
}
```

Construct it with the payload: `Status.Blocked("waiting on review")`. The
payload travels with the value, and the only way to read it is to `match`,
which forces you to consider the blocked case wherever a status is inspected.

## Step 4: behavior with `impl`

Functions that belong to a type live in an `impl` block and take `self`. Move
the formatting onto `Task` as a method:

```rust
impl Task {
    fun line(self) -> String {
        let tag = match self.status {
            Todo -> "todo",
            Doing -> "doing",
            Done -> "done",
            Blocked(reason) -> "blocked: ${reason}",
        }
        return "#${self.id} [${tag}] ${self.title}"
    }
}
```

You call it as `t.line()`. Methods keep the data and the operations on it
together, and `self` gives access to every field.

## Step 5: look something up with `Option`

Searching a list might find nothing, and Raven has no `null` to return in that
case. The answer is `Option<T>`: `Some(value)` when there is a result, `None`
when there is not. A lookup by id:

```rust
fun find(tasks: List<Task>, id: Int) -> Option<Task> {
    for t in tasks {
        if t.id == id {
            return Some(t)
        }
    }
    return None
}
```

The caller `match`es on the result and cannot forget the missing case:

```rust
match find(tasks, 2) {
    Some(t) -> print("found: ${t.title}"),
    None -> print("not found"),
}
```

## The whole program

Putting it together:

```rust
enum Status {
    Todo,
    Doing,
    Done,
    Blocked(String),
}

struct Task {
    id: Int,
    title: String,
    status: Status,
}

impl Task {
    fun line(self) -> String {
        let tag = match self.status {
            Todo -> "todo",
            Doing -> "doing",
            Done -> "done",
            Blocked(reason) -> "blocked: ${reason}",
        }
        return "#${self.id} [${tag}] ${self.title}"
    }
}

fun find(tasks: List<Task>, id: Int) -> Option<Task> {
    for t in tasks {
        if t.id == id {
            return Some(t)
        }
    }
    return None
}

fun main() {
    let tasks: List<Task> = [
        Task { id: 1, title: "write docs", status: Status.Doing },
        Task { id: 2, title: "ship release", status: Status.Blocked("waiting on review") },
        Task { id: 3, title: "fix bug", status: Status.Done },
    ]

    for t in tasks {
        print(t.line())
    }

    match find(tasks, 2) {
        Some(t) -> print("found: ${t.title}"),
        None -> print("not found"),
    }
}
```

Output:

```
#1 [doing] write docs
#2 [blocked: waiting on review] ship release
#3 [done] fix bug
found: ship release
```

## Where to go next

- Count how many tasks are in each status with a `Map<String, Int>` from
  [std/collections](../stdlib/collections.md), keyed by `label(t.status)`.
- Filter to just the blocked tasks and print their reasons.
- Persist the list as JSON with [std/json](../stdlib/json.md), or derive
  serialization with `@derive(ToJson)` (see the language reference).

See the [word-frequency tutorial](word-frequency.md) for a program that reads
input and uses regular expressions.
