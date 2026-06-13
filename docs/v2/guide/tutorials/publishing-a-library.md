# Tutorial: building and publishing a library

Raven packages live on GitHub. There is no central registry and no publish
command: you write a library, push it to a repo, tag a version, and anyone can
depend on it with `rvpm add`. This tutorial walks the whole loop. We will build
a small but real library, `raven-slug`, that turns text like `"Hello, World!"`
into a URL slug like `"hello-world"`, test and document it, publish it, and then
use it from another project.

By the end you will know how a library is laid out, the patterns that make one
pleasant to use, and the exact steps to ship it.

If you have not set up a project before, skim the
[rvpm guide](../rvpm.md) first; this tutorial uses its commands throughout.

## Step 1: scaffold the library

A library is a package whose entry file is `lib.rv` at the root. That file is
the API other projects import. Scaffold one with `--lib`:

```bash
rvpm init --lib raven-slug
cd raven-slug
```

You get a small tree:

```
raven-slug/
  rv.toml          # the manifest: name, version, edition, dependencies
  lib.rv           # the API others import
  .gitignore       # ignores the generated target/ directory
```

A name like `raven-slug` is just a convention. Prefixing packages with `raven-`
makes them easy to spot on GitHub, and the repo name is the last segment of the
import path, so the repo and the package share one name.

## Step 2: write the API

Open `lib.rv` and replace the starter code. We will offer two things: a
`slugify` function for the common case, and a `Slugger` builder for when someone
needs to change the separator or cap the length. Designing for both is a small
habit that pays off: the easy thing stays a one-liner, and the flexible thing is
there when you need it.

```rust
import std/string

// Turn arbitrary text into a URL slug: lowercased, with runs of
// non-alphanumeric characters collapsed to a single separator.
//
//     slugify("Hello, World!")   // "hello-world"
fun slugify(text: String) -> String {
    return Slugger.new().slugify(text)
}

// A configurable slug builder. Start from Slugger.new(), adjust it, then call
// slugify(). Each setter returns a new Slugger, so configurations are cheap to
// reuse and never mutate each other.
struct Slugger {
    separator: String,
    max_len: Int,
}

impl Slugger {
    // A slugger with sensible defaults: "-" between words and no length cap.
    fun new() -> Slugger {
        return Slugger { separator: "-", max_len: 0 }
    }

    // Join words with `sep` instead of "-".
    fun separator(self, sep: String) -> Slugger {
        return Slugger { separator: sep, max_len: self.max_len }
    }

    // Cap the result at `n` characters. 0 means no limit.
    fun max_len(self, n: Int) -> Slugger {
        return Slugger { separator: self.separator, max_len: n }
    }

    // Build the slug for `text`.
    fun slugify(self, text: String) -> String {
        let lower = text.to_lower()
        let out = ""
        let pending = false
        let i = 0
        while i < lower.length() {
            let b = lower.byte_at(i)
            if _is_alnum(b) {
                if pending {
                    out = out.concat(self.separator)
                    pending = false
                }
                out = out.concat(lower.substring(i, i + 1))
            } else if out != "" {
                pending = true
            }
            i += 1
        }
        if self.max_len > 0 && out.length() > self.max_len {
            out = out.substring(0, self.max_len)
        }
        return out
    }
}

// True when byte `b` is an ASCII lowercase letter or digit.
fun _is_alnum(b: Int) -> Bool {
    let digit = b >= 48 && b <= 57
    let letter = b >= 97 && b <= 122
    return digit || letter
}
```

Two things worth calling out:

- **Everything is public, except names starting with `_`.** Raven has no
  `pub` keyword: every top-level `fun`, `struct`, `enum`, and `trait` is part of
  the package's API. The leading underscore on `_is_alnum` is a convention that
  marks it internal; `rvpm doc` hides it, and it signals to readers "don't
  depend on this."
- **The builder is immutable.** Each setter returns a fresh `Slugger` rather
  than mutating `self`. That means a caller can keep a base configuration around
  and branch off it safely, which is a friendlier contract than in-place
  mutation.

## Step 3: check that it compiles

A library has no `main`, so `rvpm build` type-checks it instead of producing a
binary. Run it early and often:

```bash
rvpm build
# Checked library raven-slug (lib.rv)
```

## Step 4: write tests

A test is a zero-argument function named `test_*` in a `*_test.rv` file. It
asserts with `std/test`, and a failed assertion panics, which the runner reports
as a failure. Put the tests right next to the code, in `lib_test.rv`:

```rust
import std/test { assert_eq_str }
import "./lib" { slugify, Slugger }

fun test_basic() {
    assert_eq_str(slugify("Hello, World!"), "hello-world")
}

fun test_collapses_runs() {
    assert_eq_str(slugify("  Multiple   spaces -- here  "), "multiple-spaces-here")
}

fun test_custom_separator() {
    assert_eq_str(Slugger.new().separator("_").slugify("a b c"), "a_b_c")
}

fun test_max_len() {
    assert_eq_str(Slugger.new().max_len(5).slugify("hello world"), "hello")
}
```

Run them:

```bash
rvpm test
```

```
running 4 tests
  ok   test_basic
  ok   test_collapses_runs
  ok   test_custom_separator
  ok   test_max_len
test result: ok. 4 passed; 0 failed
```

Each test runs in its own process, so one failure does not take the others down
with it. Writing the tricky cases as tests (the leading spaces, the double
dashes, the length cap) is also the cheapest documentation you will ever write:
they show exactly what the library does.

## Step 5: document it

Raven has no separate doc-comment syntax. Any run of `//` lines directly above
an item is its documentation, and `rvpm doc` collects them into Markdown:

```bash
rvpm doc
# Wrote target/doc/raven-slug.md (2 item(s) from 1 file(s))
```

The output lists each public item with its signature and the comment above it:

````markdown
### slugify

```rust
fun slugify(text: String) -> String
```

Turn arbitrary text into a URL slug: lowercased, with runs of
non-alphanumeric characters collapsed to a single separator.
````

Because the comments you already wrote in Step 2 are the docs, keeping them
accurate is the whole job. Note that `_is_alnum` does not appear: internal items
are left out.

## Step 6: format

One formatter, no options. Run it before every commit so diffs stay about the
code, not the whitespace:

```bash
rvpm fmt            # format everything in the package
rvpm fmt --check    # verify formatting in CI; exits non-zero if anything differs
```

## Step 7: write a README

The README is the first thing a person reads before adding your package, so it
earns its keep. Keep it short and lead with a usage example:

````markdown
# raven-slug

Turn text into URL slugs. No dependencies.

## Install

```toml
[dependencies]
"github.com/martian56/raven-slug" = "0.1"
```

## Usage

```rust
import "github.com/martian56/raven-slug" { slugify, Slugger }

slugify("Hello, World!")                       // "hello-world"
Slugger.new().separator("_").slugify("a b")    // "a_b"
Slugger.new().max_len(5).slugify("hello world") // "hello"
```
````

Add a `LICENSE` file too (MIT is a common, permissive choice). A package with a
clear README and a license reads as something you can trust.

## Step 8: publish

Publishing is just pushing the repo and tagging a version. There is no upload
step and no account to register.

```bash
git init
git add .
git commit -m "raven-slug: turn text into URL slugs"

# create the repo on GitHub (via the website, or the gh CLI):
gh repo create martian56/raven-slug --public --source=. --remote=origin

git push -u origin main
```

Then tag a release. Versions are git tags in `vMAJOR.MINOR.PATCH` form, and
**pushing the tag is the publish**:

```bash
git tag -a v0.1.0 -m "raven-slug 0.1.0"
git push origin v0.1.0
```

That is the entire release. The tag is what `rvpm` resolves a version
constraint against, so once it is on GitHub, the package is consumable.

## Step 9: use it from another project

From any other package, add the dependency and import it:

```bash
rvpm add github.com/martian56/raven-slug@v0.1.0
```

That records it in `rv.toml`, resolves and pins it in `rv.lock`, and fetches it
into the shared cache. Now import it like any module:

```rust
import std/io { println }
import "github.com/martian56/raven-slug" { slugify }

fun main() {
    println(slugify("My First Blog Post!"))   // my-first-blog-post
}
```

If a name from your package clashes with one from another, rename it on the way
in. Imports support `as` for both single names and whole modules:

```rust
import "github.com/martian56/raven-slug" { slugify as to_slug }
import "github.com/martian56/raven-slug" as slug   // then slug.slugify(...)
```

## Releasing new versions

When you change the library, commit, then tag the next version and push the tag.
Follow semantic versioning so consumers know what to expect:

- **Patch** (`v0.1.1`): a bug fix, no API change.
- **Minor** (`v0.2.0`): new functionality, existing code still compiles.
- **Major** (`v1.0.0`): a breaking change, such as renaming or removing a public
  item.

Consumers stay on the version pinned in their `rv.lock` until they run
`rvpm update`, so a new tag never disturbs anyone until they ask for it.

## What makes a good Raven library

The patterns in this tutorial are the same ones the standard packages follow:

- **A one-shot function for the common case, a builder for the rest.** Most
  callers want `slugify(text)`; the few who need options reach for `Slugger`.
- **Underscore-prefix internals.** It keeps the public surface small and the
  generated docs clean.
- **Immutable builders.** Setters that return a new value compose safely and
  read clearly.
- **Tests beside the code.** They guard against regressions and double as
  examples.
- **A README that opens with usage,** a `LICENSE`, and doc comments on every
  public item.

Keep the API small, name things plainly, and let `rvpm fmt`, `rvpm test`, and
`rvpm doc` do the busywork. That is the whole craft.
