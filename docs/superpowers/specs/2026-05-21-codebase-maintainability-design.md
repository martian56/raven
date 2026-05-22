# Codebase Maintainability Refactor — Design

**Date:** 2026-05-21
**Status:** Approved (pending user review of this doc)
**Scope:** Rust-side restructure only. No language behavior changes. No CLI/REPL surface changes. No stdlib (`lib/*.rv`) changes.

---

## 1. Problem

Four files carry ~80% of Raven's Rust source (12,227 LOC total):

| File | LOC | God-functions |
|---|---|---|
| `src/code_gen.rs` | 2,504 | `eval_expression` (~817), `call_builtin_function` (~788) |
| `src/type_checker.rs` | 1,746 | `check` (~378), `check_expression_with_expected_type` (~566), `check_builtin_function` (~458) |
| `src/parser.rs` | 1,663 | already well-decomposed (24 small `parse_*` fns); only `parse_term` (~280) is borderline |
| `src/format.rs` | 778 | well-decomposed |

Two operational pain points emerge:

1. **Dual builtin maintenance.** Adding a stdlib builtin (e.g., a new `fs.*` function) requires editing two parallel ~500–800 line `match` statements (`call_builtin_function` in `code_gen.rs` and `check_builtin_function` in `type_checker.rs`). The two sides drift easily.
2. **Expression dispatchers are unreadable.** `eval_expression` and `check_expression_with_expected_type` each contain a single match arm exceeding 500 lines (`MethodCall` on the runtime side, similar shape on the type-check side). Reading them requires scrolling, not navigating.

## 2. Goals & non-goals

**Goals:**
- Split the two god-files into directory modules with each file < ~400 LOC.
- Symmetric layout between `code_gen/` and `type_checker/` so the runtime and type-check sides of any builtin or expression variant live in the same filename.
- Preserve language behavior bit-for-bit. Verified by a new golden-output test suite added before the refactor begins.
- Preserve the public API (`Interpreter`, `Value`, `TypeChecker`) so `main.rs`, `repl.rs`, and `src/bin/rvpm.rs` don't change.

**Non-goals (explicit):**
- No new abstractions, traits, visitor patterns, dispatch tables, or IRs.
- No `parser.rs`, `format.rs`, `lexer.rs`, `ast.rs`, `error.rs`, `span.rs`, `repl.rs`, `paths.rs` changes.
- No `lib/*.rv` stdlib changes.
- No CI workflow changes (existing `fmt → clippy → build → test` still passes).
- No builtin registry / unified-builtin-definition (rejected; intentionally kept as two parallel sides for this refactor).

## 3. Target file layout

```
src/
  code_gen/
    mod.rs              # Interpreter struct, ::new, execute(), pub re-exports
    value.rs            # Value enum + impl Display, Default, alloc_tcp_id
    array_ops.rs        # flatten_array_index_chain, assign_array_element_by_path,
                        # assign_array_flat_target
    eval.rs             # eval_expression: dispatch over Expression variants; small
                        # arms inline, BinaryOp delegates to binop.rs, MethodCall to
                        # methods/mod.rs
    binop.rs            # BinaryOp arm body (~165 LOC)
    methods/
      mod.rs            # MethodCall dispatcher: route by receiver Value kind
      string.rs         # methods on Value::String
      array.rs          # methods on Value::Array
      tcp.rs            # methods on TcpListener / TcpStream
      struct_impl.rs    # user-defined struct methods (wraps call_struct_method)
      module.rs         # module.fn() calls
    stmt.rs             # statement execution helpers (currently inline in execute)
    calls.rs            # call_struct_method, call_function_with_module, scope save/restore
    modules.rs          # load_module
    builtins/
      mod.rs            # call_builtin_function: dispatch by name to category fns
      core.rs           # len, type, panic, enum_from_string
      io.rs             # print, input, format
      string.rs         # parse_int, char_code
      time.rs           # sys_time, sys_date, sys_timestamp
      fs.rs             # read_file, write_file, append_file, file_exists,
                        # list_directory, create_directory, remove_file,
                        # remove_directory, get_file_size, is_dir
      net.rs            # tcp_listen, tcp_accept, tcp_read, tcp_write,
                        # tcp_close_stream, tcp_close_listener, dns_lookup, reachable
      http.rs           # http_fetch, http_invoke_dispatch (+ value_from_ureq_response)
    tests.rs            # existing #[cfg(test)] mod, moved verbatim

  type_checker/
    mod.rs              # TypeChecker struct, ::new, top-level check()
    type_repr.rs        # impl Type (fmt_for_user, from_string, from_string_with_context)
    stmt.rs             # statement-level checking (the 378-LOC body of check())
    expr.rs             # check_expression / check_expression_with_expected_type
    binop.rs            # BinaryOp arm body
    methods/
      mod.rs
      string.rs
      array.rs
      tcp.rs
      struct_impl.rs
      module.rs
    modules.rs          # load_module_for_type_checking
    builtins/
      mod.rs            # check_builtin_function dispatcher
      core.rs           # mirrors code_gen/builtins/core.rs
      io.rs
      string.rs
      time.rs
      fs.rs
      net.rs
      http.rs
    tests.rs

  # Untouched:
  parser.rs  lexer.rs  format.rs  ast.rs  error.rs  span.rs
  repl.rs    paths.rs  lib.rs     main.rs
  bin/rvpm.rs  bin/gen_long_rv.rs

tests/
  golden.rs             # NEW — runs deterministic examples/*.rv, compares stdout

examples/
  *.rv                  # existing
  *.rv.out              # NEW — committed golden stdout baselines
```

### Invariants

- `code_gen/mod.rs` re-exports `Interpreter` and `Value` so `use raven::code_gen::Interpreter;` still works from `main.rs`/`repl.rs`/tests.
- `type_checker/mod.rs` re-exports `TypeChecker` and `Type` likewise.
- No function in any new module is `pub` unless it was `pub` before. Cross-module access inside `code_gen/` uses `pub(super)` / `pub(crate)`.
- No new dependencies in `Cargo.toml`.

## 4. Builtin bucketing

Pulled directly from the existing `call_builtin_function` match in `code_gen.rs`:

| File | Builtins (Rust-backed) |
|---|---|
| `core.rs` | `len`, `type`, `panic`, `enum_from_string` |
| `io.rs` | `print`, `input`, `format` |
| `string.rs` | `parse_int`, `char_code` |
| `time.rs` | `sys_time`, `sys_date`, `sys_timestamp` |
| `fs.rs` | `read_file`, `write_file`, `append_file`, `file_exists`, `list_directory`, `create_directory`, `remove_file`, `remove_directory`, `get_file_size`, `is_dir` |
| `net.rs` | `tcp_listen`, `tcp_accept`, `tcp_read`, `tcp_write`, `tcp_close_stream`, `tcp_close_listener`, `dns_lookup`, `reachable` |
| `http.rs` | `http_fetch`, `http_invoke_dispatch` |

`builtins/mod.rs` is a flat `match name { … }` that dispatches to per-category functions (e.g., `io::call(interp, name, args)`). Each category file owns a smaller `match name` over only its builtins.

`type_checker/builtins/` uses the **same filenames and same function names**, returning `Type` instead of `Value`. Mirror is intentional: adding a new builtin is "open the same filename on both sides."

**Note:** `math`, `collections`, and `json` have **zero** Rust-backed builtins — those are pure stdlib `.rv` files. No `builtins/math.rs` etc. is created. (Earlier drafts of this design listed them; corrected here.)

**Out of scope but adjacent:** `default_value_for_type_str` in `code_gen.rs` (line 543) is a small `match` over type-name strings. It is not a builtin; it stays as a private method on `Interpreter` inside `code_gen/mod.rs`.

## 5. Expression dispatcher slicing

### Runtime side (`code_gen/eval.rs`)

Measured arm sizes in current `eval_expression`:

| Arm | LOC |
|---|---|
| `Integer`, `Float`, `Boolean`, `StringLiteral` | ~1 each |
| `Identifier` | ~7 |
| `UnaryOp` | ~13 |
| **`BinaryOp`** | **~165** → `binop.rs` |
| `FunctionCall` | ~12 |
| `ArrayLiteral` | ~7 |
| `ArrayIndex` | ~37 |
| **`MethodCall`** | **~499** → `methods/` directory |
| `StructInstantiation` | ~24 |
| `FieldAccess` | ~16 |
| `EnumVariant` | ~16 |

Result: `eval.rs` itself shrinks to ~150 LOC of dispatch + small arms. The two big arms move out.

### `methods/` is the only nested directory inside `code_gen/`

500 LOC of one match arm, naturally bucketing by receiver type, is exactly the case where a folder pays for itself. Submodules: `string.rs`, `array.rs`, `tcp.rs`, `struct_impl.rs`, `module.rs`. Every other split is a flat file.

### Type-checker side (`type_checker/expr.rs`)

Same shape, mirror filenames. The type-checker's expression function is ~566 LOC (vs. ~817 on the runtime side) because of extra inference work, but the same two arms dominate.

### Statement-level (`type_checker/stmt.rs`)

`check()`'s match is 378 LOC. Big arms are `FunctionDecl`, `VariableDeclTyped`, `Assignment`, `ImplBlock`, `Import`. Each becomes a `check_<variant>` helper in `stmt.rs`; small arms stay inline. No nested directory needed.

## 6. Migration order

Each row below is one commit. Every commit compiles, passes `cargo test`, passes the new golden suite, passes `cargo clippy`, passes `cargo fmt --check`.

| # | Step | Notes |
|---|---|---|
| 0 | Add `tests/golden.rs` + capture `examples/*.rv.out` baselines | Safety net first. CI catches drift in every later commit. |
| 1 | `git mv src/code_gen.rs src/code_gen/mod.rs` | Literal move, zero logic edits. |
| 2 | Extract `code_gen/value.rs`, `code_gen/array_ops.rs` | Smallest, lowest-risk lifts. |
| 3 | Extract `code_gen/builtins/mod.rs` with `call_builtin_function` whole | Giant out of `mod.rs`; still one match, but contained. |
| 4 | Split `builtins/mod.rs` into category files | 7 sub-commits, one per file: `core`, `io`, `string`, `time`, `fs`, `net`, `http`. |
| 5 | Extract `code_gen/methods/` (mod + 5 receiver files) | Biggest single logical change. |
| 6 | Extract `code_gen/stmt.rs` — the `execute()` statement dispatcher (~317 LOC at current line 183) | Big arms (`FunctionDecl`, `VariableDeclTyped`, `Assignment`, `ImplBlock`, `Import`) become `execute_<variant>` helpers; small arms stay inline. |
| 7 | Extract `code_gen/binop.rs` | Independent, small. |
| 8 | Extract `code_gen/calls.rs` and `code_gen/modules.rs` | Cleans up remainder of `mod.rs`. |
| 9 | Move tests → `code_gen/tests.rs` | Pure file move. |
| 10–18 | Repeat steps 1–9 for `type_checker.rs` | Same recipe, mirror filenames. |
| 19 | Update `CLAUDE.md` architecture table | Docs follow code. |

**End-state check per step:**
```
cargo build
cargo test --lib
cargo test --test golden
cargo clippy
cargo fmt --check
```

## 7. Testing strategy

### Golden harness (`tests/golden.rs`)

A single integration test:

1. Walks `examples/*.rv`.
2. Skips files whose first 5 lines contain `// golden:skip`.
3. For each remaining file: invokes `cargo run --quiet -- examples/<name>.rv`, captures stdout + stderr + exit code.
4. Compares stdout to `examples/<name>.out`. Compares exit code to `examples/<name>.exit` if present (default expectation: 0).
5. On missing baseline: fails with "run `RAVEN_UPDATE_GOLDEN=1 cargo test --test golden` to capture".
6. On mismatch: fails with a unified diff.

**Invocation: shell out, not in-process.** The interpreter uses `println!` directly, which is hard to capture without source surgery. Shelling out keeps the test harness trivially correct during the refactor. Speed is acceptable for ~12 files in CI.

### Non-determinism handling

Examples that call `sys_time`/`sys_date`/`sys_timestamp`, network/HTTP builtins, or `input()` cannot have stable `.out` files. The `// golden:skip` marker (with a reason) excludes them. Skipped files print a `skipped: <reason>` line in test output so they remain visible.

Likely initial skip list (to be confirmed when baselines are captured):
- `network_http_smoke.rv` (network)
- `tcp_listen_smoke.rv` (network, blocks)
- Any file calling `sys_*`

### Stdlib coverage caveat

`lib/*.rv` files are not standalone-executable; they are modules imported by examples. Golden coverage of stdlib happens **indirectly** via examples that `import` them (`standard_library_demo.rv`, `comprehensive.rv`, `test_suite.rv`). After capturing baselines, identify any stdlib module unexercised by any example; for each gap, either add a one-line driver example or accept the gap with a note in the spec. **No new test infrastructure for running `.rv` libraries standalone** — that is scope creep.

### Update flow

`RAVEN_UPDATE_GOLDEN=1` overwrites `.out` baselines instead of comparing. CI never sets this. Locally it's how an intentional behavior change refreshes baselines.

## 8. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Behavior regression hidden by 18 unit tests | Step 0 lands golden suite before any moves. |
| Visibility / `pub(crate)` mistakes break callers | Each commit must `cargo build`; CI gates catch it. |
| Symmetric naming drift between code_gen/ and type_checker/ | Recipe doc (this spec, §3) is the canonical name list. PR review checks parity. |
| Step 5 (`methods/` split) too big for one commit | If it exceeds ~600 LOC of diff, split per receiver file (string, array, tcp, struct, module). |
| Skipped example silently undermines safety net | Skip-list size is reported in test output. If > ~30% of examples are skipped, stop and add deterministic equivalents before continuing. |
| `cargo run` per example slow in CI | Acceptable for ~12 files. If it becomes painful later, threading an output writer through `Interpreter` is a follow-up — not part of this refactor. |

## 9. Acceptance criteria

- `cargo build`, `cargo test`, `cargo clippy`, `cargo fmt --check` all pass at every commit.
- Every file in the new `code_gen/` and `type_checker/` trees is < ~400 LOC.
- `code_gen/builtins/` and `type_checker/builtins/` have identical filenames and identical per-file function names.
- `tests/golden.rs` covers ≥ 70% of `examples/*.rv` (rough target; exact figure set after baseline capture).
- `CLAUDE.md` architecture table reflects the new directories.
- Public API unchanged: `main.rs`, `repl.rs`, `src/bin/rvpm.rs` need zero edits (verified by `git diff`).

## 10. Out of scope (revisited)

- `parser.rs` decomposition (already well-structured; `parse_term`'s 280 LOC is a known but acceptable spot).
- `format.rs` decomposition.
- Any change to `lib/*.rv`.
- Builtin registry / unified-definition pattern.
- Visitor traits, IR, bytecode VM, dispatch tables.
- CI workflow changes.
- Performance work.
- New stdlib functions or language features.
