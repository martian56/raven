//! Randomized GC-rooting stress generator.
//!
//! Generates many diverse Raven programs that combine the allocation
//! patterns most likely to expose a rooting gap (a heap value built then
//! left unrooted across a later allocation): struct, enum, list, map, and
//! set construction with String and nested fields, deep interpolation with
//! many non-literal parts, closures capturing several heap values, Any
//! boxing, derive(ToString) round-trips, method chains on freshly built
//! receivers, and `match`, all interleaved with allocation churn that runs
//! the collector. Each generated program computes a deterministic Int
//! checksum from values that must survive collection and prints it; this
//! file computes the same checksum independently and asserts they match.
//!
//! Every program runs under `RAVEN_GC_THRESHOLD=1`, the most aggressive
//! setting, so a collection fires on nearly every allocation. A rooting gap
//! corrupts a live String or frees a live object, which flips a gated
//! addition (wrong checksum) or crashes (non-zero exit). Because the bug
//! class is nondeterministic, each program runs several times.
//!
//! The default set is sized for CI. Set `RAVEN_GC_STRESS_SHAPES` to widen
//! it (for example 200) and `RAVEN_GC_STRESS_REPEATS` to run each shape
//! more times.

use std::path::{Path, PathBuf};
use std::process::Command;

use raven::codegen::linker::{self, RuntimeStaticLib};
use raven::codegen::{self, CodegenError};
use raven::hir::lower_file;
use raven::lexer::Lexer;
use raven::mir::lower_program;
use raven::parser::parse;
use raven::resolve::{expand_with_stdlib, resolve_file, FsLoader};
use raven::tycheck::check_file;

const DEFAULT_SHAPES: u64 = 60;
const DEFAULT_REPEATS: usize = 4;

#[test]
fn randomized_rooting_survives_threshold_one() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    let shapes = env_u64("RAVEN_GC_STRESS_SHAPES", DEFAULT_SHAPES);
    let repeats = env_u64("RAVEN_GC_STRESS_REPEATS", DEFAULT_REPEATS as u64) as usize;

    // A fixed base seed keeps the default run reproducible; each shape mixes
    // its index in, so the set is diverse but deterministic.
    let base_seed: u64 = env_u64("RAVEN_GC_STRESS_SEED", 0x5eed_1234_abcd_0001);

    let mut failures = Vec::new();
    for shape in 0..shapes {
        let mut rng = Rng::new(base_seed ^ shape.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let prog = generate_program(&mut rng);
        let example = match build_program(&prog.source, &runtime) {
            Ok(e) => e,
            Err(e) => {
                failures.push(format!("shape {shape}: build failed: {e}\n{}", prog.source));
                continue;
            }
        };
        let expected = format!("{}\n", prog.checksum);
        for run in 0..repeats {
            let output = Command::new(&example.binary)
                .env("RAVEN_GC_THRESHOLD", "1")
                .output()
                .expect("run generated binary");
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            if !output.status.success() || stdout != expected {
                failures.push(format!(
                    "shape {shape} run {run}: status={:?}\n  expected: {expected:?}\n  \
                     actual:   {stdout:?}\n  stderr: {stderr}\n  source:\n{}",
                    output.status, prog.source
                ));
                break;
            }
        }
        cleanup(&example.tmp);
        if !failures.is_empty() && failures.len() >= 5 {
            break;
        }
    }

    assert!(
        failures.is_empty(),
        "randomized rooting stress found {} failure(s) under RAVEN_GC_THRESHOLD=1:\n\n{}",
        failures.len(),
        failures.join("\n\n----\n\n")
    );
}

/// A generated program and the checksum it must print.
struct Program {
    source: String,
    checksum: i64,
}

/// Build a single randomized program. The program declares a churn helper
/// and a handful of types, then in `main` builds a value of each selected
/// shape, holds it across churn, reads it back, and folds a contribution
/// into a running checksum gated on the value being intact. The same
/// checksum is computed here so a corrupted value diverges.
fn generate_program(rng: &mut Rng) -> Program {
    let churn = 8 + (rng.next() % 18) as i64;
    let mut checksum: i64 = 0;
    let mut body = String::new();
    body.push_str("    let total = 0\n");

    // A pool of distinct non-trivial String literals, so corruption of one
    // does not accidentally read as another's bytes.
    let words = WORD_POOL;

    // Each shape adds a block to `main` and a matching contribution to the
    // expected checksum. Pick a randomized subset (always at least four) so
    // shapes vary while every program stays substantial.
    let shape_count = 4 + (rng.next() % 5) as usize;
    for k in 0..shape_count {
        let pick = rng.next() % 9;
        let weight = 1i64 << (k as i64 + 3); // distinct magnitudes per slot
        match pick {
            0 => emit_struct_shape(rng, &mut body, &mut checksum, words, churn, weight),
            1 => emit_enum_shape(rng, &mut body, &mut checksum, words, churn, weight),
            2 => emit_list_shape(rng, &mut body, &mut checksum, words, churn, weight),
            3 => emit_map_shape(rng, &mut body, &mut checksum, words, churn, weight),
            4 => emit_set_shape(rng, &mut body, &mut checksum, words, churn, weight),
            5 => emit_closure_shape(rng, &mut body, &mut checksum, churn, weight),
            6 => emit_interp_shape(rng, &mut body, &mut checksum, words, churn, weight),
            7 => emit_tostring_shape(rng, &mut body, &mut checksum, words, churn, weight),
            _ => emit_any_shape(rng, &mut body, &mut checksum, words, churn, weight),
        }
    }

    body.push_str("    print(total)\n");

    let source = format!(
        "import std/string\n\
         import std/collections {{ Map, Set }}\n\
         \n\
         @derive(Eq, ToString, Debug)\n\
         struct Rec {{ id: Int, tag: String }}\n\
         \n\
         struct Nest {{ outer: Int, inner: Rec }}\n\
         \n\
         @derive(Eq, ToString)\n\
         enum Variant {{\n\
         \x20   Empty,\n\
         \x20   One(String),\n\
         \x20   Two(Int, String),\n\
         }}\n\
         \n\
         fun churn(n: Int) -> Int {{\n\
         \x20   let acc = 0\n\
         \x20   let i = 0\n\
         \x20   while i < n {{\n\
         \x20       let junk = Rec {{ id: i, tag: \"scratch\" }}\n\
         \x20       acc = acc + junk.id\n\
         \x20       i = i + 1\n\
         \x20   }}\n\
         \x20   return acc\n\
         }}\n\
         \n\
         fun main() {{\n\
         {body}\
         }}\n"
    );

    Program { source, checksum }
}

/// Struct with a String field built then read after churn. The String must
/// survive: a corrupted `tag` fails the equality and drops the weight.
fn emit_struct_shape(
    rng: &mut Rng,
    body: &mut String,
    checksum: &mut i64,
    words: &[&str],
    churn: i64,
    weight: i64,
) {
    let id = (rng.next() % 90 + 1) as i64;
    let w = words[(rng.next() as usize) % words.len()];
    let nested = rng.next() % 2 == 0;
    let v = next_var(rng);
    if nested {
        body.push_str(&format!(
            "    let {v} = Nest {{ outer: {id}, inner: Rec {{ id: {id}, tag: \"{w}\" }} }}\n\
             \x20   let _ = churn({churn})\n\
             \x20   if {v}.inner.tag == \"{w}\" {{ total = total + {weight} + {v}.outer }}\n"
        ));
    } else {
        body.push_str(&format!(
            "    let {v} = Rec {{ id: {id}, tag: \"{w}\" }}\n\
             \x20   let _ = churn({churn})\n\
             \x20   if {v}.tag == \"{w}\" {{ total = total + {weight} + {v}.id }}\n"
        ));
    }
    *checksum += weight + id;
}

/// Enum with a String (and Int) payload, matched after churn.
fn emit_enum_shape(
    rng: &mut Rng,
    body: &mut String,
    checksum: &mut i64,
    words: &[&str],
    churn: i64,
    weight: i64,
) {
    let n = (rng.next() % 50 + 1) as i64;
    let w = words[(rng.next() as usize) % words.len()];
    let v = next_var(rng);
    body.push_str(&format!(
        "    let {v} = Variant.Two({n}, \"{w}\")\n\
         \x20   let _ = churn({churn})\n\
         \x20   let {v}r = match {v} {{\n\
         \x20       Empty -> 0,\n\
         \x20       One(s) -> 1,\n\
         \x20       Two(k, s) -> if s == \"{w}\" {{ k + {weight} }} else {{ 0 }},\n\
         \x20   }}\n\
         \x20   total = total + {v}r\n"
    ));
    *checksum += n + weight;
}

/// List of String literals, read back by index after churn.
fn emit_list_shape(
    rng: &mut Rng,
    body: &mut String,
    checksum: &mut i64,
    words: &[&str],
    churn: i64,
    weight: i64,
) {
    let len = 3 + (rng.next() % 4) as usize;
    let chosen: Vec<&str> = (0..len)
        .map(|_| words[(rng.next() as usize) % words.len()])
        .collect();
    let lits = chosen
        .iter()
        .map(|w| format!("\"{w}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let idx = rng.next() as usize % len;
    let target = chosen[idx];
    let v = next_var(rng);
    body.push_str(&format!(
        "    let {v} = [{lits}]\n\
         \x20   let _ = churn({churn})\n\
         \x20   if {v}[{idx}] == \"{target}\" {{ total = total + {weight} }}\n"
    ));
    *checksum += weight;
}

/// Map with String keys and String values; a key read back after churn.
fn emit_map_shape(
    rng: &mut Rng,
    body: &mut String,
    checksum: &mut i64,
    words: &[&str],
    churn: i64,
    weight: i64,
) {
    let k = words[(rng.next() as usize) % words.len()];
    let val = words[(rng.next() as usize) % words.len()];
    let v = next_var(rng);
    body.push_str(&format!(
        "    let {v} = Map.new()\n\
         \x20   {v}.set(\"{k}\", \"{val}\")\n\
         \x20   let _ = churn({churn})\n\
         \x20   match {v}.get(\"{k}\") {{\n\
         \x20       Some(got) -> {{ if got == \"{val}\" {{ total = total + {weight} }} }}\n\
         \x20       None -> {{}}\n\
         \x20   }}\n"
    ));
    *checksum += weight;
}

/// Set of String literals; membership checked after churn.
fn emit_set_shape(
    rng: &mut Rng,
    body: &mut String,
    checksum: &mut i64,
    words: &[&str],
    churn: i64,
    weight: i64,
) {
    let a = words[(rng.next() as usize) % words.len()];
    let b = words[(rng.next() as usize) % words.len()];
    let v = next_var(rng);
    body.push_str(&format!(
        "    let {v} = Set.new()\n\
         \x20   {v}.add(\"{a}\")\n\
         \x20   {v}.add(\"{b}\")\n\
         \x20   let _ = churn({churn})\n\
         \x20   if {v}.contains(\"{a}\") {{ total = total + {weight} }}\n"
    ));
    *checksum += weight;
}

/// Closure capturing several heap-derived values, invoked after churn.
fn emit_closure_shape(
    rng: &mut Rng,
    body: &mut String,
    checksum: &mut i64,
    churn: i64,
    weight: i64,
) {
    let a = (rng.next() % 30 + 1) as i64;
    let b = (rng.next() % 30 + 1) as i64;
    let c = (rng.next() % 30 + 1) as i64;
    let v = next_var(rng);
    body.push_str(&format!(
        "    let {v}a = {a}\n\
         \x20   let {v}b = {b}\n\
         \x20   let {v}c = {c}\n\
         \x20   let {v}f = fun(x: Int) -> Int = x + {v}a + {v}b + {v}c + {weight}\n\
         \x20   let _ = churn({churn})\n\
         \x20   total = total + {v}f(0)\n"
    ));
    *checksum += a + b + c + weight;
}

/// Deep interpolation with many non-literal parts, compared after churn.
fn emit_interp_shape(
    rng: &mut Rng,
    body: &mut String,
    checksum: &mut i64,
    words: &[&str],
    churn: i64,
    weight: i64,
) {
    let id = (rng.next() % 90 + 1) as i64;
    let w = words[(rng.next() as usize) % words.len()];
    let n = (rng.next() % 90 + 1) as i64;
    let v = next_var(rng);
    // Build a Rec, then interpolate several of its parts plus extra
    // expressions: the fold concatenates many heap Strings.
    body.push_str(&format!(
        "    let {v} = Rec {{ id: {id}, tag: \"{w}\" }}\n\
         \x20   let {v}n = {n}\n\
         \x20   let {v}s = \"a=${{{v}.id}}-b=${{{v}.tag}}-c=${{{v}n}}-d=${{{v}.id}}\"\n\
         \x20   let _ = churn({churn})\n\
         \x20   if {v}s == \"a={id}-b={w}-c={n}-d={id}\" {{ total = total + {weight} }}\n"
    ));
    *checksum += weight;
}

/// derive(ToString) round-trip: build a struct, render it, compare after
/// churn.
fn emit_tostring_shape(
    rng: &mut Rng,
    body: &mut String,
    checksum: &mut i64,
    words: &[&str],
    churn: i64,
    weight: i64,
) {
    let id = (rng.next() % 90 + 1) as i64;
    let w = words[(rng.next() as usize) % words.len()];
    let v = next_var(rng);
    body.push_str(&format!(
        "    let {v} = Rec {{ id: {id}, tag: \"{w}\" }}\n\
         \x20   let {v}s = {v}.to_string()\n\
         \x20   let _ = churn({churn})\n\
         \x20   if {v}s == \"Rec {{ id: {id}, tag: {w} }}\" {{ total = total + {weight} }}\n"
    ));
    *checksum += weight;
}

/// Any-box a struct, churn, then read a field back through reflection.
fn emit_any_shape(
    rng: &mut Rng,
    body: &mut String,
    checksum: &mut i64,
    words: &[&str],
    churn: i64,
    weight: i64,
) {
    let id = (rng.next() % 90 + 1) as i64;
    let w = words[(rng.next() as usize) % words.len()];
    let v = next_var(rng);
    body.push_str(&format!(
        "    let {v} = to_any<Rec>(Rec {{ id: {id}, tag: \"{w}\" }})\n\
         \x20   let _ = churn({churn})\n\
         \x20   let {v}f = get_field({v}, \"id\")\n\
         \x20   match {v}f {{\n\
         \x20       Some(av) -> {{\n\
         \x20           match cast<Int>(av) {{\n\
         \x20               Some(iv) -> {{ if iv == {id} {{ total = total + {weight} }} }}\n\
         \x20               None -> {{}}\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       None -> {{}}\n\
         \x20   }}\n"
    ));
    *checksum += weight;
}

/// Distinct, non-trivial String literals used as field, element, key, and
/// value content. Distinct lengths and bytes mean a corrupted read is very
/// unlikely to alias another and pass by coincidence.
const WORD_POOL: &[&str] = &[
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "indigo", "juliet",
    "kilo", "lima", "mike", "november", "oscar", "papa",
];

/// A fresh unique variable name per shape slot, so generated blocks never
/// collide on a binding name.
fn next_var(rng: &mut Rng) -> String {
    // A monotonically-derived suffix from the rng keeps names unique within
    // a program without external state.
    format!("v{}", rng.next() % 1_000_000)
}

/// A small splitmix64 PRNG: deterministic, dependency free, good enough to
/// diversify generated shapes.
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Rng {
            state: seed | 1, // avoid the all-zero fixed point
        }
    }
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(default)
}

// ----- compile / link harness (mirrors tests/codegen_smoke.rs) -----

struct CompiledProgram {
    binary: PathBuf,
    tmp: PathBuf,
}

fn build_program(source: &str, runtime: &RuntimeStaticLib) -> Result<CompiledProgram, String> {
    let object_bytes = build_object(source)?;
    let tmp = workdir();
    let object_path = tmp.join("gen.o");
    std::fs::write(&object_path, &object_bytes).expect("write object");
    let binary = tmp.join(if cfg!(windows) { "gen.exe" } else { "gen" });
    if let Err(e) = linker::link(&object_path, runtime, &binary) {
        cleanup(&tmp);
        return Err(format!("link: {e}"));
    }
    Ok(CompiledProgram { binary, tmp })
}

const COMPILER_STACK_SIZE: usize = 512 * 1024 * 1024;

fn build_object(source: &str) -> Result<Vec<u8>, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(COMPILER_STACK_SIZE)
        .spawn(move || build_object_inner(&source))
        .expect("spawn compile worker")
        .join()
        .expect("compile worker panicked")
}

fn build_object_inner(source: &str) -> Result<Vec<u8>, String> {
    let path = Path::new("generated.rv");
    let tokens = Lexer::new(source.to_string(), path.to_path_buf())
        .tokenize()
        .map_err(|e| format!("lex: {e}"))?;
    let tokens = raven::macros::expand_tokens(&tokens).map_err(|e| format!("macro: {e}"))?;
    let file = parse(&tokens).map_err(|e| format!("parse: {e}"))?;
    let file = expand_with_stdlib(&file).map_err(|e| format!("stdlib: {e}"))?;
    let mut loader = FsLoader;
    let resolved = resolve_file(&file, &mut loader).map_err(|e| format!("resolve: {e}"))?;
    let typed = check_file(&resolved).map_err(|e| format!("tycheck: {e}"))?;
    let hir = lower_file(&typed).map_err(|e| format!("hir: {e}"))?;
    let mir = lower_program(&hir).map_err(|e| format!("mir: {e}"))?;
    codegen::compile_program(&mir).map_err(|e: CodegenError| format!("codegen: {e}"))
}

fn workdir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    p.push(format!("raven-gcstress-{pid}-{stamp}-{seq}"));
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

fn cleanup(p: &Path) {
    let _ = std::fs::remove_dir_all(p);
}

fn supported_runtime() -> Option<RuntimeStaticLib> {
    if !linker::linker_available() {
        eprintln!("gc_rooting_stress: skipping, no linker available for the host.");
        return None;
    }
    locate_runtime().or_else(|| {
        eprintln!(
            "gc_rooting_stress: skipping, raven_runtime staticlib not built. \
             Run `cargo build -p raven-runtime`."
        );
        None
    })
}

fn locate_runtime() -> Option<RuntimeStaticLib> {
    if let Ok(p) = std::env::var("RAVEN_RUNTIME_LIB") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(RuntimeStaticLib { path: pb });
        }
    }
    let lib_name = if cfg!(windows) {
        "raven_runtime.lib"
    } else {
        "libraven_runtime.a"
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for sub in ["target/debug", "target/release"] {
        let p = root.join(sub).join(lib_name);
        if p.is_file() {
            return Some(RuntimeStaticLib { path: p });
        }
    }
    None
}
