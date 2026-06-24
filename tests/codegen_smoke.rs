//! End to end smoke test for the Cranelift back end.
//!
//! Compiles `examples/v2/hello.rv` with the driver, links the
//! resulting object with the `raven-runtime` staticlib using the
//! toolchain-aware linker (MSVC `link.exe` on windows-msvc, `cc`
//! elsewhere), runs the binary, and checks that stdout matches
//! `Hello, Raven!\n`. On a correctly configured host the test links and
//! runs the program for real. It short circuits with a diagnostic on
//! `eprintln!` and a successful exit only in the genuinely unsupported
//! cases: no linker is available at all, or the runtime staticlib has
//! not been built yet.

use std::path::{Path, PathBuf};
use std::process::Command;

use raven::codegen::linker::{self, RuntimeStaticLib};
use raven::codegen::{self, CodegenError};
use raven::hir::lower_file;
use raven::lexer::Lexer;
use raven::mir::lower_program;
use raven::parser::parse_with_macros;
use raven::resolve::{expand_with_stdlib, resolve_file, FsLoader};
use raven::tycheck::check_file;

#[test]
fn hello_world_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    compile_link_run_and_check("hello.rv", "Hello, Raven!\n", &runtime);
}

#[test]
fn result_struct_error_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Regression for #513: a struct error payload in `Result<Bool, _>` carried
    // through `?` and matched. The `Err(e)` binding must be typed as the error
    // struct, not the `Ok` payload, or the program crashes at runtime.
    compile_link_run_and_check("result_struct_error.rv", "10\nnegative code=1\n", &runtime);
}

#[test]
fn declarative_macros_compile_and_run() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // `twice!`, `max2!`, repetition macros, and a hygienic macro expand at the
    // token level before parse. twice!(n + 1) == 8, max2!(10, 7) == 10,
    // max2!(2 * 2, 9) == 9, twice!(twice!(n)) + 1 == 13, sum_all!(1, 2, 3) == 6,
    // sum_all!() == 0, sum_all!(10) == 10, sum_some!(4, 5, 6) == 15, and the
    // hygienic doubled!(tmp) == 200 with the caller's tmp left at 100.
    compile_link_run_and_check(
        "use_macros.rv",
        "8\n10\n9\n13\n6\n0\n10\n15\n200\n100\n",
        &runtime,
    );
}

#[test]
fn mutable_module_global_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A scalar global reassigned from a function (counter 0 -> 3), a global
    // computed from a function call at startup (base == 30), a reassigned
    // String global, and a heap List global read back (6).
    compile_link_run_and_check("module_global.rv", "0\n3\n30\ndone\n6\n", &runtime);
}

#[test]
fn local_const_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A `const` inside a function body is an immutable local with a runtime
    // initializer: LIMIT == 5, DOUBLED == compute() == 42, sum == 47.
    compile_link_run_and_check("local_const.rv", "5\n42\n47\n", &runtime);
}

#[test]
fn item_and_statement_position_macros_compile_and_run() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Macro calls in item position (a struct and two functions) and in
    // statement position (a print and a two-`let` binding). emit!(p.x)/p.y
    // print 1/2, decl_pair!(a, b) gives a + b == 30, f1() + f2() == 3.
    compile_link_run_and_check("macro_item_statement.rv", "1\n2\n30\n3\n", &runtime);
}

#[test]
fn macro_call_in_interpolation_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A macro call inside a `"${...}"` fragment expands against the file's
    // macro table: twice!(n + 1) == 8, the nested twice!(twice!(n)) == 12, and
    // twice!(10) == 20 alongside a plain `${name}`.
    compile_link_run_and_check(
        "interp_macro_call.rv",
        "twice = 8\nnested 12 done\nhi raven, 20\n",
        &runtime,
    );
}

#[test]
fn multifile_local_imports_compile_and_run() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // `main.rv` selectively imports a function and a struct from a sibling
    // `helper.rv` (`./helper`). The expander merges the local module into
    // the program, so the imported `greet` call, the imported `Counter`
    // type, and its `bumped` method all compile and link. Prints the
    // greeting then 42.
    compile_link_run_and_check("multifile/main.rv", "hi raven\n42\n", &runtime);
}

#[test]
fn struct_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Point { x: 3, y: 4 } built on the heap, passed by reference, and
    // summed through field access: prints 7.
    compile_link_run_and_check("point.rv", "7\n", &runtime);
}

#[test]
fn enum_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Option values matched and unwrapped: 5 + 99 prints 104.
    compile_link_run_and_check("option_sum.rv", "104\n", &runtime);
}

#[test]
fn enum_construction_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // User enum variants constructed in expression position and matched.
    // `Color.Green` (unit) prints `green`; `Shape.Circle(2.0)` and
    // `Shape.Square(3.0)` (payload) print 12 and 9.
    compile_link_run_and_check("enum_construct.rv", "green\n12\n9\n", &runtime);
}

#[test]
fn enum_self_method_match_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Regression for issue #186: matching on the `self` receiver of an
    // enum method must read the payload through the underlying enum, not
    // the `SelfTy` wrapper. Scalar prints 42, String prints hello, List
    // payload `.len()` prints 3.
    compile_link_run_and_check("enum_self_method.rv", "42\nhello\n3\n", &runtime);
}

#[test]
fn derive_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // `@derive(Eq, Hash, ToString, Debug)` synthesizes the trait impls. The
    // derived `Eq + Hash` let a struct key a Map and join a Set (lines 5 to
    // 7), the enum derive compares and prints payload variants, and the
    // generic `Pair` derive compares and prints at <Int, String>.
    compile_link_run_and_check(
        "use_derive.rv",
        "true\nfalse\nPoint { x: 1, y: 2 }\nPoint { x: 1, y: 2 }\ntrue\ntrue\nfalse\ntrue\nfalse\nCircle(3)\nDot\ntrue\nfalse\nPair { first: 1, second: hi }\n",
        &runtime,
    );
}

#[test]
fn reflection_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // `type_name<T>()` renders a type's name and `field_names<T>()` yields a
    // struct's fields. The generic `describe<T>` and `introspect<T>` resolve
    // per monomorphization, so the same body prints `Int` then `Point`.
    compile_link_run_and_check(
        "use_reflection.rv",
        "Int\nString\nPoint\n2\nx\ny\nInt\nPoint\nintrospect Point\nfield x\nfield y\n",
        &runtime,
    );
}

#[test]
fn field_types_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // `field_types<T>()` pairs with `field_names<T>()`, rendering each field's
    // type by position. A generic struct (`Box<T>`) renders its concrete field
    // type per instantiation, so `Box<Int>` reads `Int` and `Box<String>`
    // reads `String`.
    compile_link_run_and_check(
        "use_field_types.rv",
        "id: Int\nname: String\nactive: Bool\n--\nInt\nString\n--\nvalue: Int\n",
        &runtime,
    );
}

#[test]
fn variant_reflection_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // `variant_names<T>()` lists an enum's variants and
    // `variant_field_types<T>()` gives each variant's payload types (empty for
    // a unit variant). The generic `describe<T>` resolves per monomorphization,
    // so `Tree<Int>` renders `Int` payloads.
    compile_link_run_and_check(
        "use_variant_reflection.rv",
        "Circle\nRectangle\nDot\n--\nCircle: Float\nRectangle: Float, Float\nDot\n== generic ==\nLeaf: Int\nBranch: Int, Int\nEmpty\n",
        &runtime,
    );
}

#[test]
fn derive_json_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // `@derive(ToJson, FromJson)` synthesizes JSON serialization for a
    // struct, a nested struct, a generic `Pair`, and an enum (tuple and unit
    // variants). Each value round-trips `to_json` -> `stringify` -> `parse`
    // -> `from_json` back to an equal value (the `true` lines), and a
    // from_json on an object missing a required field returns the
    // missing-field error.
    compile_link_run_and_check(
        "use_derive_json.rv",
        "{\"x\":1,\"y\":2}\ntrue\n{\"start\":{\"x\":0,\"y\":0},\"end\":{\"x\":3,\"y\":4}}\ntrue\n{\"first\":7,\"second\":true}\ntrue\n{\"tag\":\"Rect\",\"values\":[2,5]}\ntrue\n{\"tag\":\"Dot\",\"values\":[]}\ntrue\nmissing field y\n",
        &runtime,
    );
}

#[test]
fn closure_value_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A non-capturing lambda is allocated as a closure object; the
    // program prints 42 to show the allocation and GC root frame run.
    compile_link_run_and_check("closure_value.rv", "42\n", &runtime);
}

#[test]
fn closure_capture_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // make_adder(10) returns a closure capturing the local `n = 10` by
    // value. Invoking the returned closure value adds the captured amount:
    // add10(5) prints 15 and add10(32) prints 42.
    compile_link_run_and_check("closure_capture.rv", "15\n42\n", &runtime);
}

#[test]
fn concurrency_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // `spawn` starts goroutines that communicate over channels on the
    // cooperative scheduler. Producer/consumer, fan-in, and a cooperative
    // interleave each sum to 15, deterministically.
    compile_link_run_and_check("use_concurrency.rv", "15\n15\n15\n", &runtime);
}

#[test]
fn nested_defer_runs_at_function_return() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A defer inside a nested `if` block runs at the function's return,
    // not at the inner block exit (function-scoped, Go-style). The program
    // prints 1 3 4 first, then the deferred 2 fires at f's return.
    compile_link_run_and_check("defer_nested.rv", "1\n3\n4\n2\n", &runtime);
}

#[test]
fn deeply_nested_expression_does_not_overflow() {
    // Regression for issue #172: lowering recurses with expression
    // nesting, so a deeply nested expression overflowed the default
    // stack until the compiler moved its work onto a large-stack thread.
    // This case must drive the real `raven` binary (not the in-process
    // pipeline used by the other smoke cases) because the large stack is
    // provided by the binary's worker thread, not the test thread.
    let Some(_runtime) = supported_runtime() else {
        return;
    };
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("v2")
        .join("deep_nesting.rv");
    let tmp = workdir();
    let binary = tmp.join(if cfg!(windows) {
        "deep_nesting.exe"
    } else {
        "deep_nesting"
    });
    let build = Command::new(env!("CARGO_BIN_EXE_raven"))
        .arg("build")
        .arg(&source_path)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("run raven build");
    assert!(
        build.status.success(),
        "raven build crashed or failed on deep nesting: status={:?} stderr={}",
        build.status,
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary)
        .output()
        .expect("run deep_nesting binary");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    cleanup(&tmp);
    assert!(run.status.success(), "deep_nesting binary exited non zero");
    assert_eq!(stdout, "250\n", "unexpected stdout: {:?}", stdout);
}

#[test]
fn closure_arg_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A capturing closure passed as a function argument and invoked
    // indirectly: `triple` captures `factor = 3`, and apply(triple, 7)
    // returns 7 * 3, printing 21.
    compile_link_run_and_check("closure_arg.rv", "21\n", &runtime);
}

#[test]
fn closure_generic_call_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A generic function (`identity<T>`) reachable only through a closure
    // body. The lifted closure body's call to `identity` is the sole call
    // site, so the monomorphizer must pick it up from the lifted body to
    // emit the `identity$Int` instantiation. apply(f, 41) computes
    // identity(41) + 1, printing 42.
    compile_link_run_and_check("closure_generic_call.rv", "42\n", &runtime);
}

#[test]
fn dyn_dispatch_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Two concrete types are coerced to `dyn Speak` and dispatched
    // through their vtables: Dog.sound() is 1, Cat.sound() is 2, each on
    // its own line.
    compile_link_run_and_check("dyn_dispatch.rv", "1\n2\n", &runtime);
}

#[test]
fn interpolation_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // String interpolation end to end: a String fragment (`name`) and an
    // arithmetic Int fragment (`a + b`) are converted, concatenated, and
    // printed. Prints `Hello, Raven!` then `sum is 7`.
    compile_link_run_and_check("interpolation.rv", "Hello, Raven!\nsum is 7\n", &runtime);
}

#[test]
fn generic_interpolation_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Interpolating a part whose static type is not a built-in scalar. A
    // generic `${x}` for `T: ToString` and a `${Point}` (user `ToString`
    // impl) are rendered through `to_string`, while the surrounding scalar
    // parts keep the per-type fast path. `show` is monomorphized at Int,
    // Bool, and Point. Prints `n=42`, `ok=true`, then `p=(3, 4)`.
    compile_link_run_and_check("generic_interp.rv", "n=42\nok=true\np=(3, 4)\n", &runtime);
}

#[test]
fn defer_order_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Two defers run in reverse declaration order at the return: the
    // function schedules print(1) then print(2), so the program
    // prints 2 then 1.
    compile_link_run_and_check("defer_order.rv", "2\n1\n", &runtime);
}

#[test]
fn defer_early_return_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Only reached defers run. f(true) takes the early return after the
    // first defer, printing 9. f(false) reaches both defers and runs
    // them LIFO at its return, printing 8 then 9. Combined: 9, 8, 9.
    compile_link_run_and_check("defer_early_return.rv", "9\n8\n9\n", &runtime);
}

#[test]
fn ffi_strlen_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // C FFI end to end: `extern "C" { fun strlen(s: CStr) -> CSize }` is
    // declared as an imported symbol, resolved against the CRT at link
    // time, and called on the C string literal `c"hello"` (a static
    // null-terminated buffer). strlen counts 5 bytes; prints 5.
    compile_link_run_and_check("ffi_strlen.rv", "5\n", &runtime);
}

#[test]
fn ffi_abs_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A non-pointer FFI type: `abs(x: CInt) -> CInt` takes and returns a
    // 32-bit C int. The negative literal -7 is reduced to i32 at the
    // call; abs(-7) is 7.
    compile_link_run_and_check("ffi_abs.rv", "7\n", &runtime);
}

#[test]
fn ffi_cfloat_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A single-precision FFI type: `sqrtf(x: CFloat) -> CFloat` takes and
    // returns a C `float`. The Raven `Float` argument 16.0 is narrowed to
    // f32 at the call (fdemote) and the f32 result is widened back to f64
    // (fpromote) before printing; sqrtf(16.0) is 4.
    compile_link_run_and_check("ffi_cfloat.rv", "4\n", &runtime);
}

#[test]
fn ffi_callback_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A non-capturing top-level function passed as a `CFnPtr`: the C
    // library `qsort` (through the runtime helper) calls back into the
    // Raven `compare` for each pair, sorting the buffer 30,10,50,20,40
    // into ascending order. Proves the Raven function compiled under the
    // platform C ABI is invocable directly by C.
    compile_link_run_and_check("ffi_callback.rv", "10\n20\n30\n40\n50\n", &runtime);
}

#[test]
fn ffi_struct_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A small C struct crosses the FFI by value. `Point { x: CInt, y: CInt }`
    // is eight bytes, so both System V AMD64 and Windows x64 carry it in one
    // integer register. `raven_ffi_point_sum({3, 4})` is 7 (by-value
    // argument), and `raven_ffi_translate({3, 4}, 1, 2)` returns {4, 6}
    // (by-value argument and by-value return). Prints 7, 4, 6.
    compile_link_run_and_check("ffi_struct.rv", "7\n4\n6\n", &runtime);
}

#[test]
fn use_ffi_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/ffi converts a runtime Raven String to a C string. strlen on
    // "hello" is 5, strcmp on equal strings is 0 and on "abc"/"abd" is a
    // negative value (-1 on the supported libc), and from_cstr round-trips
    // a CStr back to the String "roundtrip".
    compile_link_run_and_check("use_ffi.rv", "5\n0\n-1\nroundtrip\n", &runtime);
}

#[test]
fn std_io_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // The first stdlib module end to end: `import std/io { println }`
    // merges the bundled `std/io` source into the program, namespaced as
    // `std.io.*`. The selector binds to that function, which calls the
    // internal `__io_*` intrinsics wired to the runtime. The second line
    // prints an interpolated integer. Prints the greeting then 42.
    compile_link_run_and_check("use_io.rv", "Hello from std/io!\n42\n", &runtime);
}

#[test]
fn std_string_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // The std/string module end to end: `import std/string { ... }` merges
    // the bundled source (which builds its utilities in pure Raven on top
    // of the `__str_*` byte intrinsics) into the program, namespaced as
    // `std.string.*`. Exercises case mapping, trim, repeat, replace,
    // substring, contains, and index_of, with the last two observed
    // through println of interpolated integers.
    let expected = "HELLO\n\
                    world\n\
                    spaced\n\
                    ababab\n\
                    a+b+c\n\
                    ave\n\
                    contains: yes\n\
                    2\n\
                    -1\n";
    compile_link_run_and_check("use_string.rv", expected, &runtime);
}

#[test]
fn builtin_method_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A method declared on the built in `Int` type via `impl Int { fun
    // doubled(self) -> Int = self * 2 }`. The call `21.doubled()`
    // resolves to the impl method and dispatches statically to the per
    // type symbol `Int$doubled`, printing 42.
    compile_link_run_and_check("builtin_method.rv", "42\n", &runtime);
}

#[test]
fn assoc_fn_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Associated functions (Type.func()): `Counter.new()` and
    // `Counter.with(41)` call the receiverless functions declared in the
    // impl block, dispatched statically to `Counter$new` and
    // `Counter$with` with no receiver argument. Prints 0 then 41.
    compile_link_run_and_check("assoc_fn.rv", "0\n41\n", &runtime);
}

#[test]
fn collections_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/collections hash-backed Set<T: Eq + Hash> and Map<K: Eq + Hash,
    // V>: dedup add, contains, remove on the set; insert, overwrite, get,
    // has on the map. Prints 2, true, false, 2, 99, false.
    compile_link_run_and_check(
        "use_collections.rv",
        "2\ntrue\nfalse\n2\n99\nfalse\n",
        &runtime,
    );
}

#[test]
fn hash_collections_resize_and_collisions() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // The hash-backed Set and Map grow past their initial 8 buckets (load
    // factor 0.75), rehashing every entry. 60 unique ints survive dedup;
    // removing the 30 evens leaves 30. A Map of 50 int keys answers get
    // correctly across resizes, overwrite keeps len, remove updates has.
    // A String-keyed map dedups an overwritten key. Prints the sequence
    // below.
    compile_link_run_and_check(
        "use_hash_collections.rv",
        "60\ntrue\ntrue\nfalse\n30\nfalse\ntrue\n50\n0\n490\n-1\n50\n7777\ntrue\nfalse\nfalse\n3\n11\n-1\n",
        &runtime,
    );
}

#[test]
fn collection_literals_compile_and_run() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Set and map literal syntax (issue #156). `{1, 2, 2, 3}` dedups to a
    // 3-element Set; `contains` reports membership. An un-annotated set
    // infers its element type. `["a": 1, "b": 2]` builds a 2-entry Map;
    // `get` returns Some/None. Map values infer (String to Bool). The
    // empty-map form `[:]` builds an empty Map. Prints
    // 3, true, false, 3, 2, 1, -1, 2, true, 0.
    compile_link_run_and_check(
        "use_collection_literals.rv",
        "3\ntrue\nfalse\n3\n2\n1\n-1\n2\ntrue\n0\n",
        &runtime,
    );
}

#[test]
fn selective_bundled_type_import_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A selective import of the bundled types `Map` and `Set`
    // (`import std/collections { Map, Set }`) must bind each name to the
    // merged type without declaring it twice (issue #184). The set dedups
    // to 2; the map has one key whose value was overwritten to 99. Prints
    // 2, 1, 99.
    compile_link_run_and_check("use_collections_selective.rv", "2\n1\n99\n", &runtime);
}

#[test]
fn error_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/error end to end: the `Error` value type, its `ToString` impl
    // (so `print(e)` renders the message), `with_context` chaining, and the
    // generic free helpers over the built-in `Result<T, E>` (`is_ok`,
    // `is_err`, `unwrap_or`, `ok`) matched through `match r { Ok(v) ->
    // ..., Err(e) -> ... }`. divide(10, 2) prints 5; is_ok and is_err on
    // an Err print false and true; unwrap_or of an Err returns the default
    // -1; the Err's message is "divide by zero"; with_context prefixes
    // "save config: disk full"; and ok(divide(8, 4)) is Some(2).
    compile_link_run_and_check(
        "use_error.rv",
        "5\nfalse\ntrue\n-1\ndivide by zero\nsave config: disk full\n2\n",
        &runtime,
    );
}

#[test]
fn test_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/test assertions in a passing program: assert, assert_msg,
    // assert_true, assert_false, assert_eq_int, and assert_eq_str all hold,
    // so no assertion calls the runtime panic. The program reaches the
    // final line and exits zero, printing `all passed`.
    compile_link_run_and_check("use_test.rv", "all passed\n", &runtime);
}

#[test]
fn hash_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/hash non-cryptographic hashes: fnv1a, djb2, hash_int, and combine
    // are deterministic (equal inputs hash equal) and input-sensitive
    // (different inputs differ; combine is order-sensitive). The booleans
    // print true, true, false, true, true, false, true, false.
    compile_link_run_and_check(
        "use_hash.rv",
        "true\ntrue\nfalse\ntrue\ntrue\nfalse\ntrue\nfalse\n",
        &runtime,
    );
}

#[test]
fn string_eq_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // `==`/`!=` on String compare contents, not object identity: a
    // concatenated "foo" equals the literal "foo", and unequal literals
    // differ. Int and Bool `==` are value compares and stay unaffected.
    // Prints true, false, false, true, true, false, true.
    compile_link_run_and_check(
        "string_eq.rv",
        "true\nfalse\nfalse\ntrue\ntrue\nfalse\ntrue\n",
        &runtime,
    );
}

#[test]
fn path_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/path POSIX `/` manipulation built on std/string's `String` methods
    // (a bundled module importing another): join collapses a trailing separator, basename/dirname split
    // on the last `/` (dirname of a bare name is "."), extension/stem split on
    // the last `.` of the basename (a name with no dot has an empty
    // extension), and is_absolute tests a leading `/`. Prints a/b/c.txt twice,
    // c.txt, a/b, ".", txt, c, an empty line, then absolute.
    compile_link_run_and_check(
        "use_path.rv",
        "a/b/c.txt\na/b/c.txt\nc.txt\na/b\n.\ntxt\nc\n\nabsolute\n",
        &runtime,
    );
}

#[test]
fn cmp_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/cmp ordering and sorting over the prelude `Ord` trait: min, max,
    // clamp, a selection sort that returns a new ascending list, and the
    // max_of/min_of reductions returning Option. Prints 3, 7, 10, then the
    // sorted first 1 and last 9, then max_of 9 and min_of 1.
    compile_link_run_and_check("use_cmp.rv", "3\n7\n10\n1\n9\n9\n1\n", &runtime);
}

#[test]
fn math_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/math end to end. The transcendental and rounding functions bind
    // to the C runtime math library through `extern "C"` (libm via the CRT),
    // returning C `double` as Raven `Float`; the integer helpers and the
    // `ln`/`abs` wrappers are pure Raven. Inputs are chosen so every Float
    // result lands on a whole number, and the exp(ln(x)) round trip is
    // asserted within a tolerance so the printed output is stable. Prints
    // abs_int 5, min_int 3, max_int 7, clamp_int 10, pow_int 1024, then
    // sqrt 4, pow 1024, abs 4, floor 3, ceil 4, round 3, trunc 3, the
    // tolerance check true, sin 0, cos 1, and the two constant checks true.
    compile_link_run_and_check(
        "use_math.rv",
        "5\n3\n7\n10\n1024\n4\n1024\n4\n3\n4\n3\n3\ntrue\n0\n1\ntrue\ntrue\n",
        &runtime,
    );
}

#[test]
fn stdlib_string_method_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A method declared on the built in `String` type inside the bundled
    // std/string module: `impl String { fun shout(self) -> String }`.
    // Importing the module merges the `impl` block into the program; the
    // method is resolved by the receiver's type, not by an imported name.
    // `shout` calls the module's sibling `to_upper`, so "hi".shout()
    // prints HI.
    compile_link_run_and_check("stdlib_string_method.rv", "HI\n", &runtime);
}

#[test]
fn core_trait_prelude_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // The auto-imported std/core prelude in action: a generic function
    // bounded by `ToString` dispatches to the built-in impls for Int and
    // Bool, and a user struct implements `ToString` itself. No
    // `import std/core` line is written; the prelude is always in scope.
    compile_link_run_and_check("trait_tostring.rv", "42\ntrue\n(3, 4)\n", &runtime);
}

#[test]
fn generic_struct_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Generic struct monomorphization end to end. `Box<T>` is instantiated
    // at Int and String, exercising the per-instantiation struct layout and
    // GC pointer descriptor (the String slot is a traced pointer, the Int
    // slot is opaque). The concrete `impl Box<Int>` method `get` and the
    // generic `impl<T> Box<T>` method `unwrap`, specialized at T = Int and
    // T = String, both resolve to per-instantiation symbols. Prints 42, 42,
    // 7, then raven.
    compile_link_run_and_check("generic_struct.rv", "42\n42\n7\nraven\n", &runtime);
}

#[test]
fn method_generics_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Method-level generic monomorphization end to end. The method
    // `mapped<U>` on `impl<T> Box<T>` introduces a type parameter `U` that
    // does not appear in the implementing type. Two calls on the same
    // `Box<Int>` at distinct `U` (Int for the doubling closure, Bool for
    // the comparison) must resolve to distinct per-instantiation symbols
    // (`Box_Int$mapped$Int` and `Box_Int$mapped$Bool`) rather than colliding
    // on the receiver-derived `Box_Int$mapped`. Each body substitution binds
    // both the impl's `T` and the method's own `U`. Prints 42 then true.
    compile_link_run_and_check("method_generics.rv", "42\ntrue\n", &runtime);
}

#[test]
fn iter_pipeline_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // The lazy iterator pipeline end to end: `xs.iter().map(f).filter(g)`
    // chained adapters driven by the generic consumers `collect`, `fold`,
    // and `count`. Each consumer is generic over `S: Iterator<T>` where the
    // element type `T` appears only in the bound and the return type, so the
    // monomorphizer recovers it by matching the declared return type against
    // the concrete call result. Prints 4, then 180, then 3.
    compile_link_run_and_check("iter_pipeline.rv", "4\n180\n3\n", &runtime);
}

#[test]
fn list_int_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // List literals, indexing, and the built-in methods over scalar
    // elements: `[10, 20, 30]` is allocated, `len` reads 3, `xs[1]` reads
    // 20, `push(40)` mutates the shared heap object, then `len` reads 4
    // and `xs[3]` reads the appended 40.
    compile_link_run_and_check("list_ops.rv", "3\n20\n4\n40\n", &runtime);
}

#[test]
fn list_string_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A list of heap String values, exercising the GC-pointer element
    // path: each element slot holds a traced pointer the collector
    // follows, so the strings stay reachable through the list. Pushing
    // "bird" then indexing words[0] and words[2] prints raven and bird,
    // and `len` reads 3.
    compile_link_run_and_check("list_strings.rv", "raven\nbird\n3\n", &runtime);
}

#[test]
fn for_loop_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // For loops over a range and a list, lowered to counter and index
    // loops. The range loop sums 0..5 to 10, the list loop sums [3, 5, 7]
    // to 15, and the third loop exercises break and continue: continue
    // still advances the counter, so 0..10 minus the i == 5 skip and the
    // i == 8 break counts 7 iterations. Prints 10, 15, 7.
    compile_link_run_and_check("for_loops.rv", "10\n15\n7\n", &runtime);
}

#[test]
fn mutation_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Field and index assignment end to end. `self.n = self.n + 1` inside
    // a method mutates the heap struct, observed after the call: two bumps
    // plus `c.n = c.n + 5` make 7. `xs[1] = 99` overwrites a list element,
    // read back as 99. Prints 7 then 99.
    compile_link_run_and_check("mutation.rv", "7\n99\n", &runtime);
}

#[test]
fn encoding_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/encoding hex and base64 over String bytes. The known vectors:
    // hex("abc")=616263, base64 of "abc"/"Man"/"Ma"/"M" =
    // YWJj/TWFu/TWE=/TQ== (the last two show one and two `=` padding).
    // Then three round-trip equalities print true.
    compile_link_run_and_check(
        "use_encoding.rv",
        "616263\nYWJj\nTWFu\nTWE=\nTQ==\ntrue\ntrue\ntrue\n",
        &runtime,
    );
}

#[test]
fn random_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/random seeded splitmix64 PRNG. Every printed line is a
    // structural check that is deterministic given the fixed seeds: two
    // Rng.new(42) agree on their first two draws, gen_range(0, 10) stays
    // in range across 100 draws, next_float lands in [0.0, 1.0), choice
    // returns an element of a small list and None on an empty list, and
    // shuffle preserves the length and element sum. Prints nine `true`.
    compile_link_run_and_check(
        "use_random.rv",
        "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n",
        &runtime,
    );
}

#[test]
fn fmt_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/fmt string and integer formatting plus the Debug trait. repeat,
    // pad_left, pad_right, and center build fixed-width fields (widths count
    // bytes); to_binary/to_octal/to_hex/to_radix render integers in a base,
    // with to_radix(-42, 16) showing the leading '-'; pad_int zero-pads with
    // the sign kept leftmost; join inserts the separator between elements;
    // and .debug() delegates to ToString for Int/Bool and wraps Char and
    // String in quotes. Prints ababab, 007, 7--, **hi**, 1010, 100, ff, -2a,
    // 00042, -007, "a, b, c", 42, true, 'x', and "hi".
    let expected = "ababab\n\
                    007\n\
                    7--\n\
                    **hi**\n\
                    1010\n\
                    100\n\
                    ff\n\
                    -2a\n\
                    00042\n\
                    -007\n\
                    a, b, c\n\
                    42\n\
                    true\n\
                    'x'\n\
                    \"hi\"\n";
    compile_link_run_and_check("use_fmt.rv", expected, &runtime);
}

#[test]
fn env_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/env environment, args, and platform info. The asserted output is
    // driven only by the env var this test sets plus structural booleans, so
    // it is identical on every host: get_env_or returns the set value, a set
    // and an unset variable report true/false, get_env_or falls back when
    // unset, arg_count is at least one, and os_name is one of the known set.
    let name = "use_env.rv";
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("v2")
        .join(name);
    let source =
        std::fs::read_to_string(&source_path).unwrap_or_else(|e| panic!("read {}: {}", name, e));
    let object_bytes = match build_object(&source, &source_path) {
        Ok(b) => b,
        Err(e) => panic!("frontend or codegen failed for {}: {}", name, e),
    };
    let tmp = workdir();
    let object_path = tmp.join("use_env.o");
    std::fs::write(&object_path, &object_bytes).expect("write object");
    let binary = tmp.join(if cfg!(windows) {
        "use_env.exe"
    } else {
        "use_env"
    });
    if let Err(e) = linker::link(
        &object_path,
        &runtime,
        &linker::NativeLink::default(),
        &binary,
    ) {
        cleanup(&tmp);
        panic!("linker failed to produce an executable for {}: {}", name, e);
    }
    let output = Command::new(&binary)
        .env("RAVEN_ENV_TEST", "hello-env")
        .env_remove("RAVEN_DEFINITELY_UNSET_XYZ")
        .output()
        .unwrap_or_else(|e| panic!("run {} binary: {}", name, e));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    cleanup(&tmp);
    assert!(
        output.status.success(),
        "use_env binary exited non zero: status={:?} stderr={}",
        output.status,
        stderr
    );
    assert_eq!(
        stdout, "hello-env\ntrue\nfalse\nfallback\ntrue\ntrue\n",
        "unexpected stdout for use_env: {:?}",
        stdout
    );
}

#[test]
fn fs_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/fs filesystem ops end to end. The program writes a uniquely
    // named file in its working directory, queries it, appends, sizes it,
    // removes it, and finally reads a missing file to exercise the Err
    // path. Every printed line is deterministic: wrote, then exists/is_file
    // true and is_dir false, the contents "hello", appended, the appended
    // contents "hello world", the size 11, removed, exists false, and the
    // missing-file read reporting "read failed". The binary runs from a
    // fresh temp dir so its own temp file (which it removes) leaves no
    // residue.
    let name = "use_fs.rv";
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("v2")
        .join(name);
    let source =
        std::fs::read_to_string(&source_path).unwrap_or_else(|e| panic!("read {}: {}", name, e));
    let object_bytes = match build_object(&source, &source_path) {
        Ok(b) => b,
        Err(e) => panic!("frontend or codegen failed for {}: {}", name, e),
    };
    let tmp = workdir();
    let object_path = tmp.join("use_fs.o");
    std::fs::write(&object_path, &object_bytes).expect("write object");
    let binary = tmp.join(if cfg!(windows) {
        "use_fs.exe"
    } else {
        "use_fs"
    });
    if let Err(e) = linker::link(
        &object_path,
        &runtime,
        &linker::NativeLink::default(),
        &binary,
    ) {
        cleanup(&tmp);
        panic!("linker failed to produce an executable for {}: {}", name, e);
    }
    let output = Command::new(&binary)
        .current_dir(&tmp)
        .output()
        .unwrap_or_else(|e| panic!("run {} binary: {}", name, e));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    cleanup(&tmp);
    assert!(
        output.status.success(),
        "use_fs binary exited non zero: status={:?} stderr={}",
        output.status,
        stderr
    );
    assert_eq!(
        stdout,
        "wrote\ntrue\ntrue\nfalse\nhello\nappended\nhello world\n11\nremoved\nfalse\nread failed\n",
        "unexpected stdout for use_fs: {:?}",
        stdout
    );
}

#[test]
fn fs_binary_roundtrip_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // fs.read returns raw bytes, so a file written with non-UTF-8 bytes reads
    // back byte-for-byte. The program writes a small binary file in its working
    // directory, reads it back, prints the byte values, and removes it.
    let example = build_example_binary("fs_binary_roundtrip.rv", &runtime);
    let output = Command::new(&example.binary)
        .current_dir(&example.tmp)
        .output()
        .expect("run fs_binary_roundtrip binary");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    cleanup(&example.tmp);
    assert!(
        output.status.success(),
        "fs_binary_roundtrip exited non zero: status={:?} stderr={}",
        output.status,
        stderr
    );
    assert_eq!(
        stdout, "wrote 5\nread 5\n0 255 104 105 254\n",
        "unexpected stdout for fs_binary_roundtrip: {:?}",
        stdout
    );
}

#[test]
fn net_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/net TCP client end to end against a loopback echo server. Raven
    // v2 has no threads, so a single program cannot be both server and
    // client: the test side runs the server on a std::thread and the
    // compiled Raven program is the client. The server binds an ephemeral
    // loopback port, the test passes its address to the client through
    // RAVEN_NET_ADDR, the client connects, writes "ping", reads the echo,
    // and prints it. A read timeout on both ends keeps a failure from
    // hanging CI.
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
    let addr = listener
        .local_addr()
        .expect("listener local addr")
        .to_string();

    let server = std::thread::spawn(move || {
        // Accept one connection, echo back exactly what is read once, then
        // drop. A read timeout means a misbehaving client cannot wedge the
        // thread.
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
            let mut buf = [0u8; 256];
            if let Ok(n) = stream.read(&mut buf) {
                let _ = stream.write_all(&buf[..n]);
                let _ = stream.flush();
            }
        }
    });

    let name = "use_net.rv";
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("v2")
        .join(name);
    let source =
        std::fs::read_to_string(&source_path).unwrap_or_else(|e| panic!("read {}: {}", name, e));
    let object_bytes = match build_object(&source, &source_path) {
        Ok(b) => b,
        Err(e) => panic!("frontend or codegen failed for {}: {}", name, e),
    };
    let tmp = workdir();
    let object_path = tmp.join("use_net.o");
    std::fs::write(&object_path, &object_bytes).expect("write object");
    let binary = tmp.join(if cfg!(windows) {
        "use_net.exe"
    } else {
        "use_net"
    });
    if let Err(e) = linker::link(
        &object_path,
        &runtime,
        &linker::NativeLink::default(),
        &binary,
    ) {
        cleanup(&tmp);
        panic!("linker failed to produce an executable for {}: {}", name, e);
    }
    let output = Command::new(&binary)
        .env("RAVEN_NET_ADDR", &addr)
        .output()
        .unwrap_or_else(|e| panic!("run {} binary: {}", name, e));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = server.join();
    cleanup(&tmp);
    assert!(
        output.status.success(),
        "use_net binary exited non zero: status={:?} stderr={}",
        output.status,
        stderr
    );
    assert_eq!(
        stdout, "ping\n",
        "unexpected stdout for use_net: {:?}",
        stdout
    );
}

#[test]
fn net_read_rejects_bad_limits() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // TcpStream.read validates its limit: a negative size is an error (not a
    // zero-length Ok that mimics EOF), and a size too large to allocate fails
    // gracefully instead of aborting. The server only has to accept the
    // connection; both reads fail before any bytes move.
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
    let addr = listener
        .local_addr()
        .expect("listener local addr")
        .to_string();
    let server = std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            // Hold the connection briefly so the client can run its reads.
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(stream);
        }
    });

    let example = build_example_binary("net_read_limits.rv", &runtime);
    let output = Command::new(&example.binary)
        .env("RAVEN_NET_ADDR", &addr)
        .output()
        .expect("run net_read_limits binary");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = server.join();
    cleanup(&example.tmp);
    assert!(
        output.status.success(),
        "net_read_limits exited non zero: status={:?} stderr={}",
        output.status,
        stderr
    );
    assert_eq!(
        stdout, "neg err\nhuge err\nalive\n",
        "unexpected stdout for net_read_limits: {:?}",
        stdout
    );
}

#[test]
fn http_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/http client end to end against a loopback mock server. CI has no
    // external network and Raven v2 has no threads, so the test runs a tiny
    // HTTP/1.1 server on a std::thread that accepts one connection, reads the
    // request headers, and writes a fixed 200 response. The compiled Raven
    // program GETs that URL (passed through RAVEN_HTTP_URL) and prints the
    // status code then the body. Read timeouts on both ends keep a failure
    // from hanging CI.
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
    let addr = listener
        .local_addr()
        .expect("listener local addr")
        .to_string();
    let url = format!("http://{addr}/");

    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
            // Read until the end of the request headers; the GET has no body.
            let mut buf = Vec::new();
            let mut chunk = [0u8; 256];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let resp = "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let name = "use_http.rv";
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("v2")
        .join(name);
    let source =
        std::fs::read_to_string(&source_path).unwrap_or_else(|e| panic!("read {}: {}", name, e));
    let object_bytes = match build_object(&source, &source_path) {
        Ok(b) => b,
        Err(e) => panic!("frontend or codegen failed for {}: {}", name, e),
    };
    let tmp = workdir();
    let object_path = tmp.join("use_http.o");
    std::fs::write(&object_path, &object_bytes).expect("write object");
    let binary = tmp.join(if cfg!(windows) {
        "use_http.exe"
    } else {
        "use_http"
    });
    if let Err(e) = linker::link(
        &object_path,
        &runtime,
        &linker::NativeLink::default(),
        &binary,
    ) {
        cleanup(&tmp);
        panic!("linker failed to produce an executable for {}: {}", name, e);
    }
    let output = Command::new(&binary)
        .env("RAVEN_HTTP_URL", &url)
        .output()
        .unwrap_or_else(|e| panic!("run {} binary: {}", name, e));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = server.join();
    cleanup(&tmp);
    assert!(
        output.status.success(),
        "use_http binary exited non zero: status={:?} stderr={}",
        output.status,
        stderr
    );
    assert_eq!(
        stdout, "200\nhello\n",
        "unexpected stdout for use_http: {:?}",
        stdout
    );
}

#[test]
fn time_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/time date and time end to end, driven by fixed UTC timestamps so
    // every printed line is identical on every host: the epoch formats to
    // 1970-01-01 00:00:00, 86400 seconds decomposes to 1970-1-2, 1700000000
    // formats to 2023-11-14, parsing "2000-01-01 00:00:00" yields the Unix
    // timestamp 946684800, an unparseable string takes the Err path printing
    // "parse failed", and now() is past Nov 2023 so the final line is true.
    let name = "use_time.rv";
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("v2")
        .join(name);
    let source =
        std::fs::read_to_string(&source_path).unwrap_or_else(|e| panic!("read {}: {}", name, e));
    let object_bytes = match build_object(&source, &source_path) {
        Ok(b) => b,
        Err(e) => panic!("frontend or codegen failed for {}: {}", name, e),
    };
    let tmp = workdir();
    let object_path = tmp.join("use_time.o");
    std::fs::write(&object_path, &object_bytes).expect("write object");
    let binary = tmp.join(if cfg!(windows) {
        "use_time.exe"
    } else {
        "use_time"
    });
    if let Err(e) = linker::link(
        &object_path,
        &runtime,
        &linker::NativeLink::default(),
        &binary,
    ) {
        cleanup(&tmp);
        panic!("linker failed to produce an executable for {}: {}", name, e);
    }
    let output = Command::new(&binary)
        .output()
        .unwrap_or_else(|e| panic!("run {} binary: {}", name, e));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    cleanup(&tmp);
    assert!(
        output.status.success(),
        "use_time binary exited non zero: status={:?} stderr={}",
        output.status,
        stderr
    );
    assert_eq!(
        stdout, "1970-01-01 00:00:00\n1970-1-2\n2023-11-14\n946684800\nparse failed\ntrue\n",
        "unexpected stdout for use_time: {:?}",
        stdout
    );
}

#[test]
fn json_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/json parse and stringify end to end. A nested object with an
    // array value round trips through parse then compact stringify,
    // rendering the whole-number Float 1 as `1`. Object key order follows
    // the hash-bucket layout. A string with a `\n` escape and a `A` unicode escape
    // decodes (the escape becomes the literal A) and re-serializes with the
    // newline re-escaped. A malformed object takes the Err path. Prints the
    // compact object, the re-escaped string, then `parse failed`.
    let expected = "{\"a\":1,\"b\":[true,null,\"hi\"]}\n\
                    \"x\\ny A\"\n\
                    parse failed\n";
    compile_link_run_and_check("use_json.rv", expected, &runtime);
}

#[test]
fn regex_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/regex end to end, backed by the Rust regex crate through the
    // runtime handle registry. Every line is deterministic: `a+` matches
    // "xaaay" (true) and finds "aaa", a non-match returns None (none),
    // and splitting "xaaayaz" yields 3 pieces x/y/z. The `(\d+)-(\d+)`
    // pattern finds 2 matches (12-345, 67-8), captures 3 groups of the
    // first match (12-345, 12, 345), and replace_all with "$2-$1" swaps
    // the groups to "a 345-12 b". An invalid pattern "(" takes the Err
    // path, printing "compile failed".
    let expected = "true\n\
                    aaa\n\
                    none\n\
                    3\n\
                    x\n\
                    y\n\
                    z\n\
                    2\n\
                    12-345\n\
                    67-8\n\
                    3\n\
                    12-345\n\
                    12\n\
                    345\n\
                    a 345-12 b\n\
                    compile failed\n";
    compile_link_run_and_check("use_regex.rv", expected, &runtime);
}

#[test]
fn process_module_is_bundled() {
    // The std/process module source is embedded in the compiler and visible
    // to the resolver under its `std/` path.
    assert!(raven::resolve::stdlib::bundled_source("process").is_some());
}

#[test]
fn process_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/process subprocesses end to end. A portable subprocess test
    // cannot rely on host commands (echo/cmd differ across platforms), so
    // the child is a SECOND compiled Raven program with identical output
    // everywhere: proc_child.rv prints `7\n`. The parent (use_process.rv)
    // reads the child's path from RAVEN_PROC_CHILD, runs it with no args,
    // and prints the exit code (0) then the captured stdout (`7\n`), then
    // takes the spawn-failure Err path on a nonexistent program, printing
    // `run failed`. Total stdout is `0\n7\nrun failed\n`.
    let child = build_example_binary("proc_child.rv", &runtime);
    let parent = build_example_binary("use_process.rv", &runtime);

    let output = Command::new(&parent.binary)
        .env("RAVEN_PROC_CHILD", &child.binary)
        .output()
        .expect("run use_process binary");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    cleanup(&child.tmp);
    cleanup(&parent.tmp);
    assert!(
        output.status.success(),
        "use_process binary exited non zero: status={:?} stderr={}",
        output.status,
        stderr
    );
    assert_eq!(
        stdout, "0\n7\nrun failed\n",
        "unexpected stdout for use_process: {:?}",
        stdout
    );
}

#[test]
fn proc_argv_preserves_empty_arg() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A single empty argument must reach the child, not vanish in the
    // NUL-encoded argv. The child prints its argument count (argv[0] included);
    // the parent runs it with no args (1), one empty arg (2), and "a" then ""
    // (3).
    let child = build_example_binary("proc_argv_child.rv", &runtime);
    let parent = build_example_binary("proc_argv_parent.rv", &runtime);

    let output = Command::new(&parent.binary)
        .env("RAVEN_PROC_CHILD", &child.binary)
        .output()
        .expect("run proc_argv_parent binary");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    cleanup(&child.tmp);
    cleanup(&parent.tmp);
    assert!(
        output.status.success(),
        "proc_argv_parent exited non zero: status={:?} stderr={}",
        output.status,
        stderr
    );
    assert_eq!(
        stdout, "1\n2\n3\n",
        "unexpected stdout for proc_argv_parent: {:?}",
        stdout
    );
}

#[test]
fn proc_binary_io_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // std/process carries arbitrary bytes both ways. The child writes the
    // bytes 0x00 0xFF 0x41 (plus print's newline) to stdout; the parent feeds
    // it non-UTF-8 stdin (which must be accepted, not rejected) and then prints
    // the byte values of the child's captured stdout, which must come back
    // verbatim rather than mangled into U+FFFD.
    let child = build_example_binary("proc_binary_child.rv", &runtime);
    let parent = build_example_binary("proc_binary_parent.rv", &runtime);

    let output = Command::new(&parent.binary)
        .env("RAVEN_PROC_CHILD", &child.binary)
        .output()
        .expect("run proc_binary_parent binary");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    cleanup(&child.tmp);
    cleanup(&parent.tmp);
    assert!(
        output.status.success(),
        "proc_binary_parent exited non zero: status={:?} stderr={}",
        output.status,
        stderr
    );
    assert_eq!(
        stdout, "stdin ok\n4\n0\n255\n65\n10\n",
        "unexpected stdout for proc_binary_parent: {:?}",
        stdout
    );
}

#[test]
fn gc_stress_survives_repeated_collections() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // The GC stress program holds a live root of every heap object kind
    // (struct with a String field, nested struct, list of strings, hash Map
    // and Set, a closure capturing a heap-derived value, an Any-boxed heap
    // value read back through reflection, a defer-captured value, and a
    // goroutine that parks heap state on a channel) across heavy allocation,
    // then reads each back. Driven under a very low RAVEN_GC_THRESHOLD so a
    // collection fires every few allocations, and run many times: the bug
    // class this guards against (a live object freed mid-collection) was
    // nondeterministic, so a single pass could pass by luck.
    let example = build_example_binary("gc_stress.rv", &runtime);
    let expected = "struct 7 ada\nnested 99 deep\nlist 0 a\nlist 2 c\nmap x\nset 2\n\
                    closure 142\nany 7 ada\ndefer 55\ngo 8\n";
    // A tight threshold (256 bytes) forces a collection after only a handful
    // of allocations, so every run exercises the frame-based root paths hard.
    for run in 0..12 {
        let output = Command::new(&example.binary)
            .env("RAVEN_GC_THRESHOLD", "256")
            .output()
            .expect("run gc_stress binary");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() || stdout != expected {
            cleanup(&example.tmp);
            panic!(
                "gc_stress run {run} diverged under collection pressure: \
                 status={:?}\n  expected: {expected:?}\n  actual:   {stdout:?}\n  stderr: {stderr}",
                output.status
            );
        }
    }
    cleanup(&example.tmp);
}

#[test]
fn gc_call_args_survive_repeated_collections() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Regression for the call-site rooting gap: a String-literal argument is
    // promoted to a fresh heap String at the call site and must stay rooted
    // across the allocation of later arguments and the call. Before the fix
    // the first argument was freed mid-call under collection pressure and
    // read back as garbage or the wrong word. Driven under a low threshold
    // and repeated, since the freed-live-object bug is nondeterministic.
    let example = build_example_binary("gc_call_args.rv", &runtime);
    let expected = "first alpha\njoined ab\npick omega\n";
    for run in 0..12 {
        let output = Command::new(&example.binary)
            .env("RAVEN_GC_THRESHOLD", "1")
            .output()
            .expect("run gc_call_args binary");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() || stdout != expected {
            cleanup(&example.tmp);
            panic!(
                "gc_call_args run {run} diverged under collection pressure: \
                 status={:?}\n  expected: {expected:?}\n  actual:   {stdout:?}\n  stderr: {stderr}",
                output.status
            );
        }
    }
    cleanup(&example.tmp);
}

#[test]
fn list_get_rejects_out_of_range_index() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A list index outside the u32 range used to be truncated before the
    // bounds check, so a multiple of 2^32 aliased a real element. It must now
    // be rejected as out of bounds. The program prints a valid get first to
    // show normal access is unaffected, then aborts on the 2^32 index.
    let example = build_example_binary("list_get_index_oob.rv", &runtime);
    let output = Command::new(&example.binary)
        .output()
        .expect("run list_get_index_oob binary");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    cleanup(&example.tmp);
    assert!(
        !output.status.success(),
        "out-of-range get should abort, but the binary exited zero: stdout={:?}",
        stdout
    );
    assert_eq!(
        stdout, "20\n",
        "the valid get should print before the abort: {:?}",
        stdout
    );
    assert!(
        stderr.contains("list index out of bounds"),
        "expected an out-of-bounds abort, got stderr: {:?}",
        stderr
    );
}

#[test]
fn read_line_preserves_non_utf8() {
    use std::io::Write;
    use std::process::Stdio;
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // read_line returns the line's raw bytes, so a non-UTF-8 byte is preserved
    // rather than dropped. The program reads one line and prints its length and
    // each byte value; the input is `A` `0xFF` `B` then a newline.
    let example = build_example_binary("read_line_binary.rv", &runtime);
    let mut child = Command::new(&example.binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn read_line_binary");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(&[0x41, 0xff, 0x42, b'\n'])
        .expect("write stdin");
    let output = child
        .wait_with_output()
        .expect("wait read_line_binary binary");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    cleanup(&example.tmp);
    assert!(
        output.status.success(),
        "read_line_binary exited non zero: status={:?} stderr={}",
        output.status,
        stderr
    );
    assert_eq!(
        stdout, "3\n65\n255\n66\n",
        "unexpected stdout for read_line_binary: {:?}",
        stdout
    );
}

/// A compiled example binary plus the temp dir holding it, so the caller
/// can run the binary and then clean up.
struct ExampleBinary {
    binary: PathBuf,
    tmp: PathBuf,
}

/// Compile `examples/v2/<name>` to a fresh temp executable and return its
/// path. Panics on any failure on a supported host.
fn build_example_binary(name: &str, runtime: &RuntimeStaticLib) -> ExampleBinary {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("v2")
        .join(name);
    let source =
        std::fs::read_to_string(&source_path).unwrap_or_else(|e| panic!("read {}: {}", name, e));
    let object_bytes = match build_object(&source, &source_path) {
        Ok(b) => b,
        Err(e) => panic!("frontend or codegen failed for {}: {}", name, e),
    };
    let tmp = workdir();
    let stem = Path::new(name).file_stem().unwrap().to_string_lossy();
    let object_path = tmp.join(format!("{}.o", stem));
    std::fs::write(&object_path, &object_bytes).expect("write object");
    let binary = tmp.join(if cfg!(windows) {
        format!("{}.exe", stem)
    } else {
        stem.to_string()
    });
    if let Err(e) = linker::link(
        &object_path,
        runtime,
        &linker::NativeLink::default(),
        &binary,
    ) {
        cleanup(&tmp);
        panic!("linker failed to produce an executable for {}: {}", name, e);
    }
    ExampleBinary { binary, tmp }
}

/// Return the runtime staticlib when a linker and the staticlib are both
/// present, or skip with a diagnostic. Shared by every smoke case so the
/// skip behavior stays identical.
fn supported_runtime() -> Option<RuntimeStaticLib> {
    if !linker::linker_available() {
        eprintln!(
            "codegen_smoke: skipping, no linker available for the host. \
             Install the MSVC C++ build tools on windows-msvc, a 64-bit \
             MinGW-w64 on windows-gnu, or a `cc` driver on Unix."
        );
        return None;
    }
    match locate_runtime() {
        Some(r) => Some(r),
        None => {
            eprintln!(
                "codegen_smoke: skipping, raven_runtime staticlib not built. \
                 Run `cargo build -p raven-runtime` to produce it."
            );
            None
        }
    }
}

/// Compile `examples/v2/<name>`, link it with the runtime, run it, and
/// assert its stdout equals `expected`. Panics on any failure on a
/// supported host so a regression is loud.
fn compile_link_run_and_check(name: &str, expected: &str, runtime: &RuntimeStaticLib) {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("v2")
        .join(name);
    let source =
        std::fs::read_to_string(&source_path).unwrap_or_else(|e| panic!("read {}: {}", name, e));

    let object_bytes = match build_object(&source, &source_path) {
        Ok(b) => b,
        Err(e) => panic!("frontend or codegen failed for {}: {}", name, e),
    };

    let tmp = workdir();
    let stem = Path::new(name).file_stem().unwrap().to_string_lossy();
    let object_path = tmp.join(format!("{}.o", stem));
    std::fs::write(&object_path, &object_bytes).expect("write object");
    let binary = tmp.join(if cfg!(windows) {
        format!("{}.exe", stem)
    } else {
        stem.to_string()
    });

    if let Err(e) = linker::link(
        &object_path,
        runtime,
        &linker::NativeLink::default(),
        &binary,
    ) {
        cleanup(&tmp);
        panic!("linker failed to produce an executable for {}: {}", name, e);
    }

    let output = Command::new(&binary)
        .output()
        .unwrap_or_else(|e| panic!("run {} binary: {}", name, e));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    cleanup(&tmp);
    assert!(
        output.status.success(),
        "{} binary exited non zero: status={:?} stderr={}",
        name,
        output.status,
        stderr
    );
    assert_eq!(
        stdout, expected,
        "unexpected stdout for {}: {:?}",
        name, stdout
    );
}

/// Stack for the compile worker thread. Matches the `raven` binary's
/// large-stack worker (issue #172): lowering recurses with source nesting,
/// so compiling in process on the default test-thread stack can overflow.
const COMPILER_STACK_SIZE: usize = 512 * 1024 * 1024;

fn build_object(source: &str, path: &Path) -> Result<Vec<u8>, String> {
    let source = source.to_string();
    let path = path.to_path_buf();
    std::thread::Builder::new()
        .stack_size(COMPILER_STACK_SIZE)
        .spawn(move || build_object_inner(&source, &path))
        .expect("spawn compile worker")
        .join()
        .expect("compile worker panicked")
}

fn build_object_inner(source: &str, path: &Path) -> Result<Vec<u8>, String> {
    let tokens = Lexer::new(source.to_string(), path.to_path_buf())
        .tokenize()
        .map_err(|e| format!("lex: {}", e))?;
    let macro_table =
        raven::macros::collect_macro_table(&tokens).map_err(|e| format!("macro: {}", e))?;
    let tokens = raven::macros::expand_tokens(&tokens).map_err(|e| format!("macro: {}", e))?;
    let file = parse_with_macros(&tokens, macro_table).map_err(|e| format!("parse: {}", e))?;
    // Mirror the driver: merge any imported bundled stdlib modules before
    // resolving. A program with no `std/` imports is unchanged.
    let file = expand_with_stdlib(&file).map_err(|e| format!("stdlib: {}", e))?;
    let mut loader = FsLoader;
    let resolved = resolve_file(&file, &mut loader).map_err(|e| format!("resolve: {}", e))?;
    let typed = check_file(&resolved).map_err(|e| format!("tycheck: {}", e))?;
    let hir = lower_file(&typed).map_err(|e| format!("hir: {}", e))?;
    let mir = lower_program(&hir).map_err(|e| format!("mir: {}", e))?;
    codegen::compile_program(&mir).map_err(|e: CodegenError| format!("codegen: {}", e))
}

fn workdir() -> PathBuf {
    // A process-wide atomic counter makes each tempdir unique even when
    // several smoke tests run in parallel and start within the same
    // nanosecond, so one test's cleanup never deletes another's binary.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    p.push(format!("raven-smoke-{}-{}-{}", pid, stamp, seq));
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

fn cleanup(p: &Path) {
    let _ = std::fs::remove_dir_all(p);
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
