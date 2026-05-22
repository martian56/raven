# Codebase Maintainability Refactor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split `src/code_gen.rs` (2,504 LOC) and `src/type_checker.rs` (1,746 LOC) into symmetric directory modules so each file stays under ~400 LOC and the runtime + type-check sides of any builtin or expression variant live in the same filename.

**Architecture:** Rust-only restructure. No language behavior changes. `Interpreter` impl is spread across multiple files inside `src/code_gen/` (Rust allows split impls); `TypeChecker` likewise across `src/type_checker/`. Symmetric per-category builtin files. A new golden-output test suite in `tests/golden.rs` runs deterministic `examples/*.rv` and diffs stdout against committed `.out` baselines — established **before** any moves begin so regressions fail CI immediately.

**Tech Stack:** Rust 2021, `cargo`, existing crates (`clap`, `chrono`, `toml`, `ureq`). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-05-21-codebase-maintainability-design.md`

---

## Pre-flight & Conventions

### Working directory
All commands assume CWD = repo root (`C:\Users\martian\projects\rust\raven` or equivalent). Shell is PowerShell on Windows; Bash on Linux. Commands are POSIX where possible; PowerShell-specific where called out.

### End-of-task verification (the recipe)

**Every task ends with this sequence.** Each task references it as "Step: Verify & commit." Do not skip any command. If any fails, stop and debug — do not commit a red task.

```bash
cargo build
cargo test --lib
cargo test --test golden        # only after Task A1 lands; skip earlier
cargo clippy -- -D warnings
cargo fmt --check
```

Then commit. Commit messages use the conventions in `scripts/commit-conventions.md`:
- `refactor:` prefix for code moves with no behavior change
- `test:` for the golden suite
- `docs:` for `CLAUDE.md` updates

**Do NOT add `Co-Authored-By: Claude` or any AI attribution trailer.** This is repo policy (see `CLAUDE.md`).

### Splitting `impl Interpreter` across files

Rust allows multiple `impl Interpreter { … }` blocks across different files in the same module tree. When moving a method out of `code_gen/mod.rs`, the recipe is:

1. Create the new file with `use super::Interpreter;` (or `use crate::code_gen::Interpreter;`) and any other types needed.
2. Open an `impl Interpreter { … }` block in the new file.
3. Paste the moved method inside.
4. Delete the method from `code_gen/mod.rs`.
5. If the method was `pub` keep it `pub`; if it was private, keep it private — visibility is unchanged.
6. If the moved method accesses private fields of `Interpreter`, no change needed — same module privacy rules apply across `impl` blocks in the same module.

Same recipe for `impl TypeChecker` on the type_checker side.

### File-move guidance

For "move N lines from file X to file Y":
- Use the exact line ranges quoted in each task — they reference the original `code_gen.rs` / `type_checker.rs` snapshot.
- After moving, **re-run `cargo build` immediately** to catch missing imports. The compiler errors guide you to add `use` statements at the top of the new file.

---

# Phase A — Safety Net

Adds golden-output tests **before** any moves so behavior regressions are caught on the very next commit.

## Task A1: Golden test harness

**Files:**
- Create: `tests/golden.rs`

- [ ] **Step 1: Create `tests/golden.rs`**

```rust
//! Golden-output test suite for examples/*.rv.
//!
//! For each examples/*.rv (skipping files whose first 5 lines contain
//! `// golden:skip`), run the `raven` binary and compare stdout to the
//! committed examples/<name>.out file.
//!
//! Set RAVEN_UPDATE_GOLDEN=1 to overwrite .out baselines.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const EXAMPLES_DIR: &str = "examples";

fn raven_binary() -> PathBuf {
    // CARGO_BIN_EXE_<name> is set by cargo for the `raven` binary in integration tests.
    PathBuf::from(env!("CARGO_BIN_EXE_raven"))
}

fn first_5_lines(path: &std::path::Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .take(5)
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize(s: &str) -> String {
    // Normalize CRLF to LF so Windows and Linux baselines match.
    s.replace("\r\n", "\n")
}

#[test]
fn golden_examples() {
    let update = std::env::var("RAVEN_UPDATE_GOLDEN").ok().as_deref() == Some("1");

    let entries: Vec<_> = fs::read_dir(EXAMPLES_DIR)
        .expect("examples/ dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rv"))
        .map(|e| e.path())
        .collect();

    assert!(!entries.is_empty(), "no examples found");

    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    let mut ran = 0;

    for src in entries {
        let name = src.file_stem().unwrap().to_string_lossy().into_owned();
        let header = first_5_lines(&src);

        if header.contains("golden:skip") {
            let reason = header
                .lines()
                .find(|l| l.contains("golden:skip"))
                .unwrap_or("")
                .trim_start_matches("//")
                .trim()
                .to_string();
            skipped.push(format!("{} ({})", name, reason));
            continue;
        }

        let output = Command::new(raven_binary())
            .arg(&src)
            .output()
            .expect("invoke raven");

        let stdout = normalize(&String::from_utf8_lossy(&output.stdout));
        let baseline_path = src.with_extension("rv.out");

        if update {
            fs::write(&baseline_path, &stdout).expect("write baseline");
            continue;
        }

        let expected = match fs::read_to_string(&baseline_path) {
            Ok(s) => normalize(&s),
            Err(_) => {
                failures.push(format!(
                    "{}: no baseline at {} — run with RAVEN_UPDATE_GOLDEN=1 to capture",
                    name,
                    baseline_path.display()
                ));
                continue;
            }
        };

        if stdout != expected {
            failures.push(format!(
                "{}: stdout mismatch\n--- expected ---\n{}\n--- actual ---\n{}",
                name, expected, stdout
            ));
        }
        ran += 1;
    }

    eprintln!("golden: ran {} examples, skipped {}", ran, skipped.len());
    for s in &skipped {
        eprintln!("  skipped: {}", s);
    }

    if !failures.is_empty() {
        panic!(
            "{} golden failure(s):\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}
```

- [ ] **Step 2: Build & confirm test compiles (will fail without baselines)**

```bash
cargo test --test golden -- --nocapture
```

Expected: failures listing every `.out` file as missing.

- [ ] **Step 3: Mark known-non-deterministic examples with `// golden:skip`**

Edit the top of each file below, inserting a comment line as the first line:

- `examples/network_http_smoke.rv` → `// golden:skip — uses network`
- `examples/tcp_listen_smoke.rv` → `// golden:skip — uses network and blocks`

For any other example that calls `sys_time` / `sys_timestamp` / `sys_date` / `input` / `http_*` / `tcp_*` / `dns_lookup` / `reachable`, also add a skip marker with a one-word reason.

To find them:

```bash
grep -lE "sys_time|sys_date|sys_timestamp|http_|tcp_|dns_lookup|reachable|\\binput\\(" examples/*.rv
```

- [ ] **Step 4: Capture baselines**

PowerShell:
```powershell
$env:RAVEN_UPDATE_GOLDEN = "1"
cargo test --test golden -- --nocapture
Remove-Item Env:\RAVEN_UPDATE_GOLDEN
```

Bash:
```bash
RAVEN_UPDATE_GOLDEN=1 cargo test --test golden -- --nocapture
```

- [ ] **Step 5: Inspect baselines manually**

```bash
ls examples/*.rv.out
```

For each `.out`, open it and confirm it looks like sensible program output. If any contains a stack trace, runtime error, or empty content where output was expected, **stop** — that example has a pre-existing bug or non-determinism. Either add `golden:skip` to it or fix the bug as a separate, prior commit.

- [ ] **Step 6: Run the test in compare mode**

```bash
cargo test --test golden
```

Expected: PASS.

- [ ] **Step 7: Verify & commit**

Run the [end-of-task verification recipe](#end-of-task-verification-the-recipe). For this task `cargo test --test golden` is the new thing to confirm.

Stage:
```bash
git add tests/golden.rs examples/*.rv examples/*.rv.out
```

Commit:
```bash
git commit -m "test: add golden-output suite for examples/*.rv"
```

---

# Phase B — `code_gen.rs` Refactor

Each task is one commit. The recipe is the same throughout: move/extract, build, test (incl. golden), clippy, fmt, commit. After every task: golden tests MUST pass.

## Task B1: Convert `code_gen.rs` to directory module

**Files:**
- Move: `src/code_gen.rs` → `src/code_gen/mod.rs`

- [ ] **Step 1: Create directory and move file**

```bash
mkdir -p src/code_gen
git mv src/code_gen.rs src/code_gen/mod.rs
```

- [ ] **Step 2: Verify & commit**

Run end-of-task verification. The file is byte-identical, just relocated. Build should pass without edits.

```bash
git commit -m "refactor(code_gen): convert to directory module"
```

## Task B2: Extract `value.rs`

**Files:**
- Modify: `src/code_gen/mod.rs` (delete lines covering `Value` enum + `Display` impl)
- Create: `src/code_gen/value.rs`

- [ ] **Step 1: Add module declaration to `mod.rs`**

At the top of `src/code_gen/mod.rs`, immediately after the existing `use` block, add:

```rust
mod value;
pub use value::Value;
```

- [ ] **Step 2: Create `src/code_gen/value.rs`**

Cut from `src/code_gen/mod.rs` the `Value` enum definition (originally `code_gen.rs` lines 74–87) and its `impl std::fmt::Display for Value` block (originally lines 89–125). Paste into a new `src/code_gen/value.rs`. Prepend:

```rust
use std::collections::HashMap;
```

The file should compile standalone (the only external reference is `HashMap`).

- [ ] **Step 3: Remove the original `Value` from `mod.rs`**

Delete the cut text from `mod.rs`. The `pub use value::Value;` line keeps every existing `use crate::code_gen::Value` import working.

- [ ] **Step 4: Verify & commit**

```bash
git commit -m "refactor(code_gen): extract Value enum into value.rs"
```

## Task B3: Extract `array_ops.rs`

**Files:**
- Modify: `src/code_gen/mod.rs`
- Create: `src/code_gen/array_ops.rs`

- [ ] **Step 1: Add module declaration**

In `src/code_gen/mod.rs`, after `mod value;`, add:

```rust
mod array_ops;
```

(No `pub use`; the two helpers are crate-internal.)

- [ ] **Step 2: Move helpers**

Cut from `mod.rs`:
- `fn flatten_array_index_chain` (originally lines 10–26)
- `fn assign_array_element_by_path` (originally lines 28–72)
- The method `fn assign_array_flat_target` on `Interpreter` (currently around line 500 in the post-B1 file; find with `grep -n "fn assign_array_flat_target" src/code_gen/mod.rs`)

Paste into a new `src/code_gen/array_ops.rs`. Prepend:

```rust
use super::Value;
use crate::ast::Expression;
use super::Interpreter;
```

The two top-level helpers stay as `pub(super) fn`. The method moves into an `impl Interpreter { … }` block in this file. Make it `pub(super) fn` if it was private (no callers outside `code_gen/`).

- [ ] **Step 3: Update call sites if needed**

Search for usage of the moved helpers from elsewhere in `mod.rs`. Calls to top-level helpers need the `array_ops::` prefix or a `use super::array_ops::{flatten_array_index_chain, assign_array_element_by_path};` at the top of `mod.rs`.

- [ ] **Step 4: Verify & commit**

```bash
git commit -m "refactor(code_gen): extract array helpers into array_ops.rs"
```

## Task B4: Extract `builtins/mod.rs` (move whole `call_builtin_function`)

**Files:**
- Modify: `src/code_gen/mod.rs`
- Create: `src/code_gen/builtins/mod.rs`

- [ ] **Step 1: Add module declaration**

In `src/code_gen/mod.rs`, add:

```rust
mod builtins;
```

- [ ] **Step 2: Create the new file**

Create `src/code_gen/builtins/mod.rs`:

```rust
//! Built-in functions called from `eval_expression`'s FunctionCall arm.
//!
//! Each category lives in its own submodule (see Task B5–B11). This file
//! is currently a single match dispatching to per-name implementations;
//! it will shrink to a router in subsequent tasks.

use super::Interpreter;
use super::Value;
use crate::ast::Expression;

impl Interpreter {
    // <PASTE call_builtin_function method HERE>
}
```

Cut the entire `fn call_builtin_function` method from `mod.rs` (find with `grep -n "fn call_builtin_function" src/code_gen/mod.rs`). Paste inside the `impl Interpreter` block in `builtins/mod.rs`.

Also move the helper method `fn value_from_ureq_response` (it's only called from `http_fetch` inside `call_builtin_function`). Find with `grep -n "fn value_from_ureq_response" src/code_gen/mod.rs`.

- [ ] **Step 3: Verify & commit**

After build, check that nothing in `mod.rs` still references private fields used only by these moved methods. Build will fail with clear messages if so.

```bash
git commit -m "refactor(code_gen): move call_builtin_function into builtins/"
```

## Task B5: Split `core` builtins

**Files:**
- Modify: `src/code_gen/builtins/mod.rs`
- Create: `src/code_gen/builtins/core.rs`

Builtins this task owns: `len`, `type`, `panic`, `enum_from_string`.

- [ ] **Step 1: Add submodule**

In `src/code_gen/builtins/mod.rs`, add at the top of the file (above the `impl` block):

```rust
mod core;
```

- [ ] **Step 2: Create `src/code_gen/builtins/core.rs`**

```rust
use super::super::Interpreter;
use super::super::Value;
use crate::ast::Expression;

pub(super) fn call(
    interp: &mut Interpreter,
    name: &str,
    args: &[Expression],
) -> Result<Option<Value>, String> {
    match name {
        "len" => {
            // <PASTE len arm body from builtins/mod.rs HERE>
            // The body becomes Ok(Some(...)) or Err(...) directly.
        }
        "type" => {
            // <PASTE type arm body>
        }
        "panic" => {
            // <PASTE panic arm body>
        }
        "enum_from_string" => {
            // <PASTE enum_from_string arm body>
        }
        _ => Ok(None),
    }
}
```

The arm bodies in `builtins/mod.rs` already evaluate their args via `self.eval_expression(...)`. In the new file, replace `self` with `interp` and the rest is identical.

- [ ] **Step 3: Route from the dispatcher**

In `builtins/mod.rs`, replace the `"len"`, `"type"`, `"panic"`, `"enum_from_string"` arms with a single delegation at the **top** of the match (before any other arm), or restructure to fall through. Cleanest: change `call_builtin_function` to first try `core::call(self, name, args)?` and return early if it returns `Some(_)`. The remaining arms stay in `mod.rs` for now.

Sketch:

```rust
pub fn call_builtin_function(
    &mut self,
    name: &str,
    args: &[Expression],
) -> Result<Option<Value>, String> {
    if let Some(v) = core::call(self, name, args)? {
        return Ok(Some(v));
    }
    match name {
        // ... remaining un-extracted arms ...
        _ => Ok(None),
    }
}
```

Each subsequent task (B6–B11) adds another `if let Some(v) = io::call(...)` line.

- [ ] **Step 4: Verify & commit**

```bash
git commit -m "refactor(code_gen): extract core builtins into builtins/core.rs"
```

## Task B6: Split `io` builtins

Builtins: `print`, `input`, `format`.

- [ ] **Step 1: Create `src/code_gen/builtins/io.rs`** with the same `pub(super) fn call(...)` shape as `core.rs`. Move arm bodies for `print`, `input`, `format` from `builtins/mod.rs`.

- [ ] **Step 2: Add `mod io;` and `if let Some(v) = io::call(self, name, args)? { return Ok(Some(v)); }` to the dispatcher.**

- [ ] **Step 3: Verify & commit**

```bash
git commit -m "refactor(code_gen): extract io builtins into builtins/io.rs"
```

## Task B7: Split `string` builtins

Builtins: `parse_int`, `char_code`.

Same pattern as B6. Create `src/code_gen/builtins/string.rs`, route from dispatcher.

- [ ] **Step 1–3: Same shape as Task B6.**

- [ ] **Step 4: Verify & commit**

```bash
git commit -m "refactor(code_gen): extract string builtins into builtins/string.rs"
```

## Task B8: Split `time` builtins

Builtins: `sys_time`, `sys_date`, `sys_timestamp`.

Same pattern. `chrono::Local` import needed at top of the new file.

```bash
git commit -m "refactor(code_gen): extract time builtins into builtins/time.rs"
```

## Task B9: Split `fs` builtins

Builtins: `read_file`, `write_file`, `append_file`, `file_exists`, `list_directory`, `create_directory`, `remove_file`, `remove_directory`, `get_file_size`, `is_dir`.

Same pattern. `std::fs` and `std::path::Path` imports needed.

```bash
git commit -m "refactor(code_gen): extract fs builtins into builtins/fs.rs"
```

## Task B10: Split `net` builtins

Builtins: `tcp_listen`, `tcp_accept`, `tcp_read`, `tcp_write`, `tcp_close_stream`, `tcp_close_listener`, `dns_lookup`, `reachable`.

Same pattern. `std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs}` and `std::io::{Read, Write}` imports needed. The `Interpreter` fields `tcp_listeners`, `tcp_streams`, `next_tcp_id`, and the helper `alloc_tcp_id` are used here.

**Watch out:** `alloc_tcp_id` is a method on `Interpreter`, so `interp.alloc_tcp_id()` still works inside `net.rs`. No changes to the method itself.

```bash
git commit -m "refactor(code_gen): extract net builtins into builtins/net.rs"
```

## Task B11: Split `http` builtins

Builtins: `http_fetch`, `http_invoke_dispatch`. The helper `value_from_ureq_response` moves into this file too.

Imports needed: `ureq`, `std::time::Duration`. Since `value_from_ureq_response` is a method on `Interpreter`, when you move it into `http.rs`, open a fresh `impl Interpreter { ... }` block for it. Alternatively, make it a free function `fn value_from_ureq_response(resp: ureq::Response) -> Result<Value, String>` since it does not access `self`. **Free function is preferred** — it removes one method from the `Interpreter` impl surface.

```bash
git commit -m "refactor(code_gen): extract http builtins into builtins/http.rs"
```

## Task B12: Audit `builtins/mod.rs` is now a router only

**Files:**
- Modify: `src/code_gen/builtins/mod.rs`

- [ ] **Step 1: Confirm the dispatcher is short**

After Tasks B5–B11 the file should look like:

```rust
mod core;
mod io;
mod string;
mod time;
mod fs;
mod net;
mod http;

use super::Interpreter;
use super::Value;
use crate::ast::Expression;

impl Interpreter {
    pub fn call_builtin_function(
        &mut self,
        name: &str,
        args: &[Expression],
    ) -> Result<Option<Value>, String> {
        if let Some(v) = core::call(self, name, args)? { return Ok(Some(v)); }
        if let Some(v) = io::call(self, name, args)? { return Ok(Some(v)); }
        if let Some(v) = string::call(self, name, args)? { return Ok(Some(v)); }
        if let Some(v) = time::call(self, name, args)? { return Ok(Some(v)); }
        if let Some(v) = fs::call(self, name, args)? { return Ok(Some(v)); }
        if let Some(v) = net::call(self, name, args)? { return Ok(Some(v)); }
        if let Some(v) = http::call(self, name, args)? { return Ok(Some(v)); }
        Ok(None)
    }
}
```

If anything else remains (forgotten arms), move it into the appropriate category file. There should be **no `match name`** left in `builtins/mod.rs`.

- [ ] **Step 2: Verify & commit (only if changes were needed)**

If clean, skip. Otherwise:

```bash
git commit -m "refactor(code_gen): finalize builtins router"
```

## Task B13: Extract `methods/string.rs`

The MethodCall arm in `eval_expression` is ~499 LOC and dispatches by receiver value kind. We'll extract one receiver-kind file per task (B13–B17).

**Files:**
- Modify: `src/code_gen/mod.rs` (the MethodCall arm of `eval_expression`)
- Create: `src/code_gen/methods/mod.rs`
- Create: `src/code_gen/methods/string.rs`

- [ ] **Step 1: Locate the MethodCall arm**

```bash
grep -n "Expression::MethodCall" src/code_gen/mod.rs
```

The arm spans roughly from that line to ~1337 in the original file. Inside it, the receiver value is `eval_expression(object_expr)`, and the rest of the arm is a `match` over `Value::String(...)`, `Value::Array(...)`, `Value::TcpListener(...)`, `Value::TcpStream(...)`, `Value::Struct(...)`, `Value::Module(...)`.

- [ ] **Step 2: Create `src/code_gen/methods/mod.rs` skeleton**

```rust
mod string;

use super::Interpreter;
use super::Value;
use crate::ast::Expression;

impl Interpreter {
    pub(super) fn eval_method_call(
        &mut self,
        object_expr: &Expression,
        method_name: &str,
        args: &[Expression],
    ) -> Result<Value, String> {
        let receiver = self.eval_expression(object_expr)?;
        match &receiver {
            Value::String(_) => string::call(self, receiver, method_name, args),
            // other receiver kinds added in B14–B17
            _ => Err(format!(
                "Method '{}' not implemented for receiver",
                method_name
            )),
        }
    }
}
```

- [ ] **Step 3: Create `src/code_gen/methods/string.rs`**

```rust
use super::super::Interpreter;
use super::super::Value;
use crate::ast::Expression;

pub(super) fn call(
    interp: &mut Interpreter,
    receiver: Value,
    method_name: &str,
    args: &[Expression],
) -> Result<Value, String> {
    let Value::String(s) = receiver else {
        return Err("method receiver must be string".to_string());
    };
    match method_name {
        // <PASTE all Value::String method arms from the original MethodCall arm HERE>
        // Each existing arm of the form
        //   "foo" => { ... return Ok(Value::String(...)); }
        // becomes
        //   "foo" => { ... Ok(Value::String(...)) }
        _ => Err(format!("Unknown string method: {}", method_name)),
    }
}
```

- [ ] **Step 4: Replace the original MethodCall arm in `mod.rs`**

Replace the entire MethodCall arm with:

```rust
Expression::MethodCall(object_expr, method_name, args) => {
    self.eval_method_call(object_expr, method_name, args)
}
```

(For now, only the string branch is implemented in `methods/mod.rs`; the other receiver kinds still need to be moved. Build will fail until B14–B17 are done. **To keep each task green**, copy the array, tcp, struct, module arms into `methods/mod.rs` temporarily as inline branches before extracting them to their own files. This makes B13 individually committable.)

Concretely, `methods/mod.rs` after B13 looks like:

```rust
mod string;

use super::Interpreter;
use super::Value;
use crate::ast::Expression;

impl Interpreter {
    pub(super) fn eval_method_call(
        &mut self,
        object_expr: &Expression,
        method_name: &str,
        args: &[Expression],
    ) -> Result<Value, String> {
        let receiver = self.eval_expression(object_expr)?;
        match &receiver {
            Value::String(_) => string::call(self, receiver, method_name, args),
            Value::Array(_) => {
                // <inline: paste the Value::Array methods here, will move in B14>
            }
            Value::TcpListener(_) | Value::TcpStream(_) => {
                // <inline: paste tcp methods, will move in B15>
            }
            Value::Struct(_, _) => {
                // <inline: paste struct method dispatch, will move in B16>
            }
            Value::Module(_) => {
                // <inline: paste module method dispatch, will move in B17>
            }
            _ => Err(format!("Method '{}' not implemented", method_name)),
        }
    }
}
```

- [ ] **Step 5: Verify & commit**

```bash
git commit -m "refactor(code_gen): extract methods/ with string receiver split"
```

## Task B14: Extract `methods/array.rs`

Same pattern as B13:

1. Create `src/code_gen/methods/array.rs` with `pub(super) fn call(interp, receiver, method_name, args)` and move all `Value::Array(...)` method arms into it.
2. Add `mod array;` to `methods/mod.rs`.
3. Replace the inline Array branch in `eval_method_call` with `array::call(self, receiver, method_name, args)`.

```bash
git commit -m "refactor(code_gen): extract methods/array.rs"
```

## Task B15: Extract `methods/tcp.rs`

Same pattern. Receivers: `Value::TcpListener`, `Value::TcpStream`. Move methods like `accept`, `read`, `write`, `close` (whichever exist as methods rather than free builtins) to this file.

```bash
git commit -m "refactor(code_gen): extract methods/tcp.rs"
```

## Task B16: Extract `methods/struct_impl.rs`

Same pattern. Receiver: `Value::Struct(_, _)`. This branch typically delegates to `Interpreter::call_struct_method`. Keep the delegation; the new file's `call` function is a thin wrapper.

```bash
git commit -m "refactor(code_gen): extract methods/struct_impl.rs"
```

## Task B17: Extract `methods/module.rs`

Same pattern. Receiver: `Value::Module(_)`. Module method calls dispatch through `Interpreter::call_function_with_module` — the new file's `call` is a thin wrapper.

```bash
git commit -m "refactor(code_gen): extract methods/module.rs"
```

## Task B18: Extract `stmt.rs` (the `execute()` body)

**Files:**
- Modify: `src/code_gen/mod.rs`
- Create: `src/code_gen/stmt.rs`

The `pub fn execute(&mut self, node: &ASTNode)` method (originally line 183, ~317 LOC) dispatches over `ASTNode` variants. Move the whole method into `stmt.rs`. Inside, big arms become helper methods (`execute_function_decl`, `execute_variable_decl_typed`, `execute_assignment`, `execute_impl_block`, `execute_import`); small arms stay inline.

- [ ] **Step 1: Create `src/code_gen/stmt.rs` skeleton**

```rust
use super::Interpreter;
use super::Value;
use crate::ast::ASTNode;

impl Interpreter {
    pub fn execute(&mut self, node: &ASTNode) -> Result<Value, String> {
        if self.return_value.is_some() {
            return Ok(self.return_value.clone().unwrap());
        }
        match node {
            ASTNode::VariableDecl(name, expr) => {
                let value = self.eval_expression(expr)?;
                self.variables.insert(name.clone(), value);
                Ok(Value::Void)
            }
            ASTNode::VariableDeclTyped(name, type_str, expr) => {
                self.execute_variable_decl_typed(name, type_str, expr)
            }
            ASTNode::FunctionDecl(name, ret_type, params, body) => {
                self.execute_function_decl(name, ret_type, params, body)
            }
            // ... one arm per ASTNode variant; small ones inline, big ones delegate ...
        }
    }

    fn execute_variable_decl_typed(
        &mut self,
        name: &str,
        type_str: &str,
        expr: &Expression,
    ) -> Result<Value, String> {
        // <PASTE original VariableDeclTyped arm body HERE>
    }

    fn execute_function_decl(
        &mut self,
        name: &str,
        ret_type: &str,
        params: &[Parameter],
        body: &ASTNode,
    ) -> Result<Value, String> {
        // <PASTE original FunctionDecl arm body HERE>
    }

    // ... and so on for Assignment, ImplBlock, Import ...
}
```

- [ ] **Step 2: Add `mod stmt;` to `mod.rs` and delete the original `execute` method.**

- [ ] **Step 3: Verify & commit**

The hardest part of this task is the `Interpreter::variables` and `Interpreter::structs` etc. field access — they remain accessible because we're still in `impl Interpreter` blocks inside the same module tree.

```bash
git commit -m "refactor(code_gen): extract execute() into stmt.rs"
```

## Task B19: Extract `binop.rs`

**Files:**
- Modify: `src/code_gen/mod.rs` (BinaryOp arm in `eval_expression`)
- Create: `src/code_gen/binop.rs`

The BinaryOp arm is ~165 LOC and has nested `match (op, l_val, r_val)` logic.

- [ ] **Step 1: Create `src/code_gen/binop.rs`**

```rust
use super::Interpreter;
use super::Value;
use crate::ast::{Expression, Operator};

impl Interpreter {
    pub(super) fn eval_binop(
        &mut self,
        left: &Expression,
        op: &Operator,
        right: &Expression,
    ) -> Result<Value, String> {
        // <PASTE the BinaryOp arm body HERE — the part that does the work,
        // not the outer `Expression::BinaryOp(l, op, r) => { … }` wrapper>
    }
}
```

- [ ] **Step 2: Replace BinaryOp arm in `eval_expression`**

```rust
Expression::BinaryOp(left, op, right) => self.eval_binop(left, op, right),
```

- [ ] **Step 3: Add `mod binop;` to `mod.rs` and verify & commit**

```bash
git commit -m "refactor(code_gen): extract BinaryOp into binop.rs"
```

## Task B20: Extract `calls.rs`

**Files:**
- Modify: `src/code_gen/mod.rs`
- Create: `src/code_gen/calls.rs`

Move: `call_struct_method`, `call_function_with_module`. These are full methods, not match arms, so it's a straight cut-and-paste into an `impl Interpreter { … }` block in `calls.rs`.

```rust
use super::Interpreter;
use super::Value;
use crate::ast::{ASTNode, Expression, Parameter};
use std::collections::HashMap;

impl Interpreter {
    // <PASTE call_struct_method HERE>
    // <PASTE call_function_with_module HERE>
}
```

Add `mod calls;`. Verify & commit:

```bash
git commit -m "refactor(code_gen): extract call_* methods into calls.rs"
```

## Task B21: Extract `modules.rs`

**Files:**
- Modify: `src/code_gen/mod.rs`
- Create: `src/code_gen/modules.rs`

Move: `load_module`. Same pattern as B20.

```rust
use super::Interpreter;
use crate::ast::ASTNode;
// ... whatever other imports load_module needs
```

```bash
git commit -m "refactor(code_gen): extract load_module into modules.rs"
```

## Task B22: Move tests

**Files:**
- Modify: `src/code_gen/mod.rs`
- Create: `src/code_gen/tests.rs`

- [ ] **Step 1: Locate the test module**

```bash
grep -n "^#\\[cfg(test)\\]" src/code_gen/mod.rs
```

- [ ] **Step 2: Move into `tests.rs`**

Cut the entire `#[cfg(test)] mod tests { … }` block from `mod.rs`. Paste into a new `src/code_gen/tests.rs`. The `mod tests { … }` wrapper stays.

Then in `mod.rs`, replace with:

```rust
#[cfg(test)]
mod tests;
```

- [ ] **Step 3: Fix imports in `tests.rs`**

The test code originally used `super::*` — that referred to items in `code_gen.rs`. Now `super` is `code_gen`, so `use super::*;` still works. Build will confirm.

- [ ] **Step 4: Verify & commit**

```bash
git commit -m "refactor(code_gen): move tests into tests.rs"
```

## Task B23: Audit `code_gen/mod.rs` is now small

- [ ] **Step 1: Check size**

```bash
wc -l src/code_gen/mod.rs
```

Expected: ~200–300 LOC. It should contain only:
- top-level `use` statements
- `mod` declarations for submodules
- `pub use` re-exports
- `Function`, `Module` structs
- `Interpreter` struct definition + `Default` + `new` + `alloc_tcp_id` + `default_value_for_type_str` + `assign_array_flat_target` if not yet moved

If anything else lingers, this is the audit step to clean it up. Otherwise no commit needed.

# Phase C — `type_checker.rs` Refactor

Identical recipe, mirror filenames. Each task name and commit message changes `code_gen` → `type_checker` and `Interpreter` → `TypeChecker`.

## Task C1: Convert `type_checker.rs` to directory module

```bash
mkdir -p src/type_checker
git mv src/type_checker.rs src/type_checker/mod.rs
git commit -m "refactor(type_checker): convert to directory module"
```

## Task C2: Extract `type_repr.rs`

Move `impl Type { fmt_for_user, from_string, from_string_with_context }` into `src/type_checker/type_repr.rs`. `pub use type_repr::Type;` from `mod.rs` to preserve the existing `use crate::type_checker::Type;` imports.

```bash
git commit -m "refactor(type_checker): extract Type impl into type_repr.rs"
```

## Task C3: Extract `builtins/mod.rs` (whole `check_builtin_function`)

Mirror Task B4. Move the entire `check_builtin_function` method into `src/type_checker/builtins/mod.rs` inside an `impl TypeChecker { ... }` block.

```bash
git commit -m "refactor(type_checker): move check_builtin_function into builtins/"
```

## Tasks C4–C10: Split builtins by category

One commit per file, mirroring B5–B11. File names and builtin names **must match** the code_gen side exactly:

- C4: `core.rs` → `len`, `type`, `panic`, `enum_from_string`
- C5: `io.rs` → `print`, `input`, `format`
- C6: `string.rs` → `parse_int`, `char_code`
- C7: `time.rs` → `sys_time`, `sys_date`, `sys_timestamp`
- C8: `fs.rs` → file system builtins (same list as B9)
- C9: `net.rs` → TCP / DNS builtins (same list as B10)
- C10: `http.rs` → `http_fetch`, `http_invoke_dispatch`

Each file exposes `pub(super) fn check(tc: &mut TypeChecker, name: &str, args: &[Expression]) -> Result<Option<Type>, String>` returning the type produced by that builtin. The dispatcher in `builtins/mod.rs` chains `if let Some(t) = core::check(tc, name, args)? { return Ok(Some(t)); }` calls.

Commit messages: `refactor(type_checker): extract <category> builtins into builtins/<category>.rs`

## Task C11: Audit `type_checker/builtins/mod.rs` is a router only

Same audit as Task B12. The file should contain only `mod` declarations and a chain of `if let Some(...)` lines.

## Tasks C12–C16: Extract `methods/`

Mirror B13–B17. Files: `methods/{mod.rs, string.rs, array.rs, tcp.rs, struct_impl.rs, module.rs}`. The dispatcher signature is `pub(super) fn check_method_call(&mut self, object_expr: &Expression, method_name: &str, args: &[Expression]) -> Result<Type, String>`.

The MethodCall handling on the type-checker side is inside `check_expression_with_expected_type`. Locate with:

```bash
grep -n "Expression::MethodCall" src/type_checker/mod.rs
```

Same staged-extraction recipe as B13: first create `methods/mod.rs` with inline branches for non-string receivers, extract `string.rs`, commit; then extract array, tcp, struct, module across C13–C16.

Commit messages: `refactor(type_checker): extract methods/<file>`

## Task C17: Extract `stmt.rs` (the body of `check()`)

`check()` is the 378-LOC statement-level dispatcher in `type_checker/mod.rs`. Mirror Task B18 — move whole method into `src/type_checker/stmt.rs`, break big arms into `check_<variant>` helpers.

```bash
git commit -m "refactor(type_checker): extract check() into stmt.rs"
```

## Task C18: Extract `binop.rs`

Mirror B19. Move the BinaryOp arm body from `check_expression_with_expected_type` into `src/type_checker/binop.rs` as `TypeChecker::check_binop`.

```bash
git commit -m "refactor(type_checker): extract BinaryOp into binop.rs"
```

## Task C19: Extract `expr.rs`

The remaining body of `check_expression_with_expected_type` (after BinaryOp and MethodCall have been extracted) moves to `src/type_checker/expr.rs`. This is the type-checker analogue of `code_gen/eval.rs`.

```bash
git commit -m "refactor(type_checker): extract check_expression into expr.rs"
```

## Task C20: Extract `modules.rs`

Mirror B21. Move `load_module_for_type_checking` into `src/type_checker/modules.rs`.

```bash
git commit -m "refactor(type_checker): extract load_module into modules.rs"
```

## Task C21: Move tests

Mirror B22.

```bash
git commit -m "refactor(type_checker): move tests into tests.rs"
```

## Task C22: Audit `type_checker/mod.rs` is now small

Mirror B23. Expected: ~200 LOC of struct definition, `new`, and `pub use` re-exports.

# Phase D — Documentation

## Task D1: Update `CLAUDE.md` architecture table

**Files:**
- Modify: `CLAUDE.md` (gitignored, but tracked locally as Claude guidance — update anyway)

- [ ] **Step 1: Update the architecture table**

Open `CLAUDE.md`. In the "Architecture" section, replace the rows for Interpreter and Type Checker:

```
| Type Checker | `src/type_checker/` | Static type validation; statement dispatcher in `stmt.rs`, expression dispatcher in `expr.rs`, builtins in `builtins/<category>.rs` mirroring code_gen |
| Interpreter | `src/code_gen/` | AST-walking execution; statement dispatcher in `stmt.rs`, expression dispatcher in `eval.rs`, method-call dispatch in `methods/`, builtins in `builtins/<category>.rs` |
```

Add a one-sentence note after the table:

> The two directories are intentionally symmetric: builtin filenames (`core`, `io`, `string`, `time`, `fs`, `net`, `http`) and method receiver-kind filenames (`string`, `array`, `tcp`, `struct_impl`, `module`) match on both sides. Adding a new builtin or method means editing the same filename on both sides.

- [ ] **Step 2: This file is gitignored — no commit needed.**

Verify with `git status`; CLAUDE.md should not appear.

---

## Final sweep

- [ ] Re-run the full verification recipe one more time from a clean state:

```bash
cargo clean
cargo build --release
cargo test --lib
cargo test --test golden
cargo clippy -- -D warnings
cargo fmt --check
```

- [ ] **Spot-check file sizes:**

```bash
wc -l src/code_gen/**/*.rs src/type_checker/**/*.rs | sort -rn | head -20
```

Acceptance: no file > ~400 LOC. If any exceeds, identify the violator and split per the same recipe (move a few arms into a new submodule).

- [ ] **Confirm public API unchanged:**

```bash
git diff main -- src/main.rs src/repl.rs src/bin/rvpm.rs
```

Acceptance: empty diff.

---

## Self-Review (this section was filled out during plan authoring)

**1. Spec coverage:**

- §3 file tree → Tasks B1–B22 (code_gen) and C1–C22 (type_checker)
- §4 builtin bucketing → Tasks B5–B11 (code_gen), C4–C10 (type_checker), plus B12/C11 router audits
- §5 expression slicing → B13–B17 (methods), B18 (stmt), B19 (binop) on code_gen; C12–C19 on type_checker
- §6 migration order → reflected directly in task ordering
- §7 testing strategy → Task A1 (harness, baselines, skip markers, update flow)
- §8 risks → mitigated by per-task verification recipe and the audit tasks (B12, B23, C11, C22)
- §9 acceptance criteria → final sweep section
- §10 out of scope → not touched (parser.rs, format.rs, lib/*.rv, bin/*)

**2. Placeholder scan:** No TBDs. The `<PASTE … HERE>` markers explicitly point at existing code that's being relocated, with `grep` commands shown to locate it. Acceptable per the writing-plans rubric since the source is the existing file, not unspecified new code.

**3. Type consistency:**

- All builtin-category `call` functions have signature `pub(super) fn call(interp: &mut Interpreter, name: &str, args: &[Expression]) -> Result<Option<Value>, String>` (code_gen) / `pub(super) fn check(tc: &mut TypeChecker, ...) -> Result<Option<Type>, String>` (type_checker). Consistent across B5–B11 and C4–C10.
- All method-receiver `call` functions have signature `pub(super) fn call(interp: &mut Interpreter, receiver: Value, method_name: &str, args: &[Expression]) -> Result<Value, String>` on code_gen side.
- `eval_method_call`, `eval_binop`, `check_method_call`, `check_binop` are the new dispatcher method names, used consistently throughout.

No issues found.
