# Compile-time benchmark

A representative Raven v2 corpus and a runner that measures how long the
compiler takes to build it. The benchmark runs in CI on every push and
pull request that touches the compiler or the corpus, so compile-time
regressions surface early.

## Corpus

The corpus lives under `corpus/`: one entry file `corpus/main.rv` that
imports and exercises every module under `corpus/modules/`. It totals
roughly 10k lines (run the generator to see the exact count printed).

Each module carries a varied mix of language constructs so the type
checker and monomorphizer do real work rather than re-checking one
repeated shape:

- Plain structs with methods (vectors, nodes).
- Generic structs (`Pair<T>`, `Wrap<T>`) with generic methods, including
  a method-level type parameter (`mapped<U>`).
- Enums matched in `match` expressions, including a payload-carrying
  `Option<Int>`.
- A trait with multiple impls and a trait-bounded generic function.
- Free generic functions, recursion, `while` and `for` loops, and `if`
  expressions.

Concrete types are keyed off each module index, so the compiler sees
distinct struct layouts, enum tags, and monomorphized instantiations
across the corpus. The corpus only needs to compile; its `main` prints a
single deterministic integer.

### Regenerating

The corpus is deterministic and committed, so CI does not regenerate it.
Re-run the generator after changing the module shape or count, then
commit the regenerated `corpus/`:

```bash
benchmarks/compile-time/generate.sh          # default module count (~10k lines)
benchmarks/compile-time/generate.sh 80       # custom module count
```

## What is measured

The compiler has a single build path today. There is no separate
debug-build and release-build mode for a Raven program, so the benchmark
measures the wall-clock time the release-built `raven` binary spends
compiling the corpus (`raven build corpus/main.rv -o <tmp>`), which is
the meaningful number. The runner takes the best of a few runs to damp
runner noise and prints every run plus the best time.

The issue targets (under 5s and under 30s for the corpus) are tracked
through the printed time and the absolute ceiling below.

## Gating

`run.sh` (Linux, used by CI) and `run.ps1` (local Windows) apply two
checks against the best measured time:

1. Regression: the best time must stay within the stored baseline scaled
   by the threshold percentage.
2. Absolute ceiling: the best time must stay under `RAVEN_BENCH_CEILING_S`
   seconds.

The measured time is always printed so trends are visible even when both
checks pass.

### Baseline

The baseline is `baseline.txt`, a single integer in milliseconds taken
from a Linux CI run. Refresh it after an intentional change that moves
compile time, by reading the time the CI job prints and writing it here.
When `baseline.txt` is absent or empty, the regression check is skipped
and only the ceiling and the printed time apply.

### Environment variables

| Variable | Default | Meaning |
|----------|---------|---------|
| `RAVEN_BENCH_THRESHOLD_PCT` | `25` | Regression allowance over the baseline, in percent. |
| `RAVEN_BENCH_CEILING_S` | `60` | Absolute wall-clock ceiling, in seconds. |
| `RAVEN_BENCH_RUNS` | `3` | Number of timed runs; the best is kept. |
| `RAVEN_BENCH_SKIP_BUILD` | `0` | Set to `1` to reuse an existing release build. |

The ceiling is intentionally generous relative to the under-30s target
so the job reports and tracks compile time without flaking on a noisy
shared CI runner.

## Running locally

```bash
benchmarks/compile-time/run.sh
```

```powershell
benchmarks\compile-time\run.ps1
```

Windows numbers run higher than CI because MSVC linking is slower than
the gcc link on the Linux runner; the baseline tracks the Linux number.
