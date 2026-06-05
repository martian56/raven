# Tutorial: a word-frequency counter

This tutorial builds a small program that counts how often each word appears
in a piece of text and prints the words from most to least common. Along the
way you will use regular expressions, a hash map, structs, and a custom sort.
Every step compiles and runs, so you can follow along and check your output.

If you have not yet, read [Getting started](../getting-started.md) to install
the compiler and learn `raven build`.

## Step 1: print some text

Start with the smallest thing that runs. Create `wordfreq.rv`:

```raven
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
returns every match as a `List<String>`.

```raven
import std/regex { compile }
import std/string

fun main() {
    let text = "the cat sat on the mat the cat ran"

    match compile("[a-z]+") {
        Ok(re) -> {
            for word in re.find_all(text.to_lower()) {
                print(word)
            }
            re.free()
        }
        Err(e) -> print("bad pattern"),
    }
}
```

A few things to notice:

- `compile` returns a `Result<Regex, Error>`, so you `match` on it. A valid
  pattern gives `Ok(re)`; a malformed one gives `Err(e)`.
- `text.to_lower()` (from [std/string](../stdlib/string.md)) folds the text to
  lowercase first, so `The` and `the` count as the same word.
- A compiled regex holds a runtime handle outside the garbage collector, so
  you call `re.free()` when you are done with it. See
  [std/regex](../stdlib/regex.md).

Running this prints each word on its own line.

## Step 3: count with a map

To count, keep a `Map<String, Int>` from each word to how many times you have
seen it. For each word, look it up: if it is already there, store one more;
if not, start it at one. [std/collections](../stdlib/collections.md) provides
the map.

```raven
import std/regex { compile }
import std/collections
import std/string

fun tally(text: String) -> Map<String, Int> {
    let counts: Map<String, Int> = Map.new()
    match compile("[a-z]+") {
        Ok(re) -> {
            for word in re.find_all(text.to_lower()) {
                match counts.get(word) {
                    Some(n) -> counts.set(word, n + 1),
                    None -> counts.set(word, 1),
                }
            }
            re.free()
        }
        Err(e) -> print("bad pattern"),
    }
    return counts
}

fun main() {
    let counts = tally("the cat sat on the mat the cat ran")
    for key in counts.keys() {
        match counts.get(key) {
            Some(n) -> print("${key}: ${n}"),
            None -> {},
        }
    }
}
```

The annotation `let counts: Map<String, Int> = Map.new()` tells the compiler
the key and value types, since an empty map has nothing to infer them from.
`counts.get(word)` returns an `Option<Int>`: `Some(n)` when the word is
present, `None` when it is not. That is how the language models "might be
absent" without a `null`.

This prints each word and its count, but in no particular order.

## Step 4: rank by frequency

To print the words from most to least common, gather the entries into a list
and sort it. A `Map` does not order its keys, so build a list of pairs first.
Model a pair as a small struct, then sort with a comparator from
[std/cmp](../stdlib/cmp.md):

```raven
import std/regex { compile }
import std/collections
import std/string
import std/cmp { sorted_by }

struct WordCount {
    word: String,
    count: Int,
}

fun tally(text: String) -> Map<String, Int> {
    let counts: Map<String, Int> = Map.new()
    match compile("[a-z]+") {
        Ok(re) -> {
            for word in re.find_all(text.to_lower()) {
                match counts.get(word) {
                    Some(n) -> counts.set(word, n + 1),
                    None -> counts.set(word, 1),
                }
            }
            re.free()
        }
        Err(e) -> print("bad pattern"),
    }
    return counts
}

fun ranked(counts: Map<String, Int>) -> List<WordCount> {
    let pairs: List<WordCount> = []
    for key in counts.keys() {
        match counts.get(key) {
            Some(n) -> pairs.push(WordCount { word: key, count: n }),
            None -> {},
        }
    }
    // A comparator returns a negative number when `a` should come first. Using
    // `b.count - a.count` orders by count, highest first.
    return sorted_by(pairs, fun(a: WordCount, b: WordCount) -> Int = b.count - a.count)
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
mat: 1
sat: 1
on: 1
ran: 1
```

`sorted_by` takes the list and a comparator closure. The comparator returns an
`Int`: negative when the first argument should sort before the second, zero
when they tie, positive otherwise. `b.count - a.count` produces a negative
number when `b` has fewer occurrences than `a`, which puts the larger count
first.

## Step 5: read from a file

The hardcoded string was handy for testing. To count a real file, read it
with [std/fs](../stdlib/fs.md) and feed its contents to `tally`. `read`
returns `Result<String, Error>`, so a missing file is handled the same way a
bad regex was: with a `match`. Here is the complete program, reading from
`input.txt`:

```raven
import std/fs { read }
import std/regex { compile }
import std/collections
import std/string
import std/cmp { sorted_by }

struct WordCount {
    word: String,
    count: Int,
}

fun tally(text: String) -> Map<String, Int> {
    let counts: Map<String, Int> = Map.new()
    match compile("[a-z]+") {
        Ok(re) -> {
            for word in re.find_all(text.to_lower()) {
                match counts.get(word) {
                    Some(n) -> counts.set(word, n + 1),
                    None -> counts.set(word, 1),
                }
            }
            re.free()
        }
        Err(e) -> print("bad pattern"),
    }
    return counts
}

fun ranked(counts: Map<String, Int>) -> List<WordCount> {
    let pairs: List<WordCount> = []
    for key in counts.keys() {
        match counts.get(key) {
            Some(n) -> pairs.push(WordCount { word: key, count: n }),
            None -> {},
        }
    }
    return sorted_by(pairs, fun(a: WordCount, b: WordCount) -> Int = b.count - a.count)
}

fun main() {
    match read("input.txt") {
        Ok(text) -> {
            for wc in ranked(tally(text)) {
                print("${wc.word}: ${wc.count}")
            }
        }
        Err(e) -> print("could not read input.txt"),
    }
}
```

## Where to go next

- Limit the output to the top ten words by stopping the loop after ten
  iterations.
- Skip common stop words ("the", "on", "a") by checking each word against a
  `Set<String>` from [std/collections](../stdlib/collections.md) before
  counting it.
- Read the filename from the command line with
  [std/env](../stdlib/env.md)'s `args()` instead of hardcoding `input.txt`.

See the [Modeling data tutorial](task-tracker.md) next for a deeper look at
structs, enums, and pattern matching.
