# Tutorial: a word-frequency counter

This tutorial builds a small program that counts how often each word appears
in a piece of text and prints the words from most to least common. Along the
way you will use regular expressions, a dict, classes, and a custom sort.
Every step compiles and runs, so you can follow along and check your output.

If you have not yet, read [Getting started](../getting-started.md) to install
the compiler and learn `raven build`.

## Step 1: print some text

Start with the smallest thing that runs. Create `wordfreq.rv`:

```rust
fun main() {
    let text = "the cat sat on the mat the cat ran"
    print(text)
}
```

```bash
raven build wordfreq.rv -o wordfreq
./wordfreq
```

You should see the line printed back. Now make it do something useful.

## Step 2: pull out the words

Text is made of words separated by spaces and punctuation. A regular
expression is the cleanest way to extract just the word-like runs. The
pattern `[a-z]+` matches one or more lowercase letters, and `find_all`
returns every match as a `list[str]`.

```rust
from std.regex import compile
import std.string

fun main() {
    let text = "the cat sat on the mat the cat ran"

    match compile("[a-z]+") {
        case Ok(re) {
            for word in re.find_all(text.to_lower()) {
                print(word)
            }
            re.free()
        }
        case Err(e) { print("bad pattern") }
    }
}
```

A few things to notice:

- `compile` returns a `Result[Regex, Error]`, so you `match` on it. A valid
  pattern gives `Ok(re)`; a malformed one gives `Err(e)`.
- `text.to_lower()` (from [std.string](../stdlib/string.md)) folds the text to
  lowercase first, so `The` and `the` count as the same word.
- A compiled regex holds a runtime handle outside the garbage collector, so
  you call `re.free()` when you are done with it. See
  [std.regex](../stdlib/regex.md).

Running this prints each word on its own line.

## Step 3: count with a dict

To count, keep a `dict[str, int]` from each word to how many times you have
seen it. `counts.get_or(word, 0)` is the current count, or `0` for a word not
seen yet, so each word stores one more than that.

```rust
from std.regex import compile

fun tally(text: str) -> dict[str, int] {
    var counts: dict[str, int] = {}
    match compile("[a-z]+") {
        case Ok(re) {
            for word in re.find_all(text.to_lower()) {
                counts[word] = counts.get_or(word, 0) + 1
            }
            re.free()
        }
        case Err(e) { print("bad pattern") }
    }
    return counts
}

fun main() {
    let counts = tally("the cat sat on the mat the cat ran")
    for (word, n) in counts {
        print("${word}: ${n}")
    }
}
```

The annotation `var counts: dict[str, int] = {}` tells the compiler the key
and value types, since an empty dict has nothing to infer them from. Looping
over a dict yields `(key, value)` pairs in insertion order. To look up a key
that might be missing, `counts.get(word)` returns an `Option[int]`: `Some(n)`
when the word is present and `None` when it is not. That is how the language
models "might be absent" without a `null`.

This prints each word and its count, in the order the words first appeared.

## Step 4: rank by frequency

To print the words from most to least common, gather the entries into a list
and sort it. A dict keeps insertion order, not frequency order, so build a list of
pairs first. Model a pair as a small class, then sort with a comparator from
[std.cmp](../stdlib/cmp.md):

```rust
from std.regex import compile
from std.cmp import sorted_by

class WordCount {
    word: str
    count: int
}

fun tally(text: str) -> dict[str, int] {
    var counts: dict[str, int] = {}
    match compile("[a-z]+") {
        case Ok(re) {
            for word in re.find_all(text.to_lower()) {
                counts[word] = counts.get_or(word, 0) + 1
            }
            re.free()
        }
        case Err(e) { print("bad pattern") }
    }
    return counts
}

fun ranked(counts: dict[str, int]) -> list[WordCount] {
    let pairs: list[WordCount] = []
    for (word, n) in counts {
        pairs.push(WordCount(word, n))
    }
    // A comparator returns a negative number when `a` should come first. Using
    // `b.count - a.count` orders by count, highest first.
    return sorted_by(pairs, (a: WordCount, b: WordCount) -> int => b.count - a.count)
}

fun main() {
    let counts = tally("the cat sat on the mat the cat ran")
    for wc in ranked(counts) {
        print("${wc.word}: ${wc.count}")
    }
}
```

Output:

```
the: 3
cat: 2
sat: 1
on: 1
mat: 1
ran: 1
```

`sorted_by` takes the list and a comparator closure. The comparator returns an
`int`: negative when the first argument should sort before the second, zero
when they tie, positive otherwise. `b.count - a.count` produces a negative
number when `b` has fewer occurrences than `a`, which puts the larger count
first. The sort is stable, so words with the same count keep their first-seen
order.

## Step 5: read from a file

The hardcoded string was handy for testing. To count a real file, read it
with [std.fs](../stdlib/fs.md) and feed its contents to `tally`. `read`
returns `Result[str, Error]`, so a missing file is handled the same way a
bad regex was: with a `match`. Here is the complete program, reading from
`input.txt`:

```rust
from std.fs import read
from std.regex import compile
from std.cmp import sorted_by

class WordCount {
    word: str
    count: int
}

fun tally(text: str) -> dict[str, int] {
    var counts: dict[str, int] = {}
    match compile("[a-z]+") {
        case Ok(re) {
            for word in re.find_all(text.to_lower()) {
                counts[word] = counts.get_or(word, 0) + 1
            }
            re.free()
        }
        case Err(e) { print("bad pattern") }
    }
    return counts
}

fun ranked(counts: dict[str, int]) -> list[WordCount] {
    let pairs: list[WordCount] = []
    for (word, n) in counts {
        pairs.push(WordCount(word, n))
    }
    return sorted_by(pairs, (a: WordCount, b: WordCount) -> int => b.count - a.count)
}

fun main() {
    match read("input.txt") {
        case Ok(text) {
            for wc in ranked(tally(text)) {
                print("${wc.word}: ${wc.count}")
            }
        }
        case Err(e) { print("could not read input.txt") }
    }
}
```

## Where to go next

- Limit the output to the top ten words by stopping the loop after ten
  iterations.
- Skip common stop words ("the", "on", "a") by checking each word against a
  `set[str]` such as `{"the", "on", "a"}` before counting it.
- Read the filename from the command line with
  [std.env](../stdlib/env.md)'s `args()` instead of hardcoding `input.txt`.

See the [Modeling data tutorial](task-tracker.md) next for a deeper look at
classes, enums, and pattern matching.
