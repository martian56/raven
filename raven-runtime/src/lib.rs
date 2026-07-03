//! Runtime support crate for compiled Raven v2 programs.
//!
//! Compiled v2 binaries link against this crate as a `staticlib`. The
//! exported `extern "C"` symbols below form the entire ABI surface the
//! back-end is allowed to call. See `docs/v2/specs/runtime.md` for the
//! full contract and what is intentionally deferred.

#![deny(unsafe_op_in_unsafe_fn)]
// The ABI surface is `extern "C"` and is called from machine code
// emitted by the back-end. The safety contract for each pointer
// argument is documented on the function itself; the symbols are not
// marked `unsafe` because the back-end emits direct call instructions
// and an `unsafe` qualifier would only matter for Rust callers.
#![allow(clippy::not_unsafe_ptr_arg_deref)]
#![allow(clippy::missing_safety_doc)]

pub mod ffi_test;
pub mod gc;
pub mod object;
pub mod reflect;
pub mod roots;
pub mod sched;
pub mod stw;
pub mod tls;

pub use gc::{
    raven_defer_enter_frame, raven_defer_push, raven_defer_run_frame, raven_gc_alloc,
    raven_gc_bytes_allocated, raven_gc_collect, raven_gc_enter_frame, raven_gc_leave_frame,
    raven_gc_live_objects, raven_gc_pop_roots, raven_gc_push_root, raven_struct_register,
};
pub use object::{
    raven_bool_to_string, raven_box_new, raven_box_payload, raven_char_to_string,
    raven_closure_captures, raven_closure_fn_ptr, raven_closure_new, raven_float_to_string,
    raven_int_to_string, raven_list_elements, raven_list_len, raven_list_new, raven_list_push,
    raven_map_bucket_count, raven_map_buckets, raven_map_new, raven_set_bucket_count,
    raven_set_buckets, raven_set_new, raven_string_byte_at, raven_string_bytes, raven_string_cmp,
    raven_string_concat, raven_string_eq, raven_string_from_byte, raven_string_from_bytes,
    raven_string_len, raven_string_new, raven_string_substring, raven_struct_fields,
    raven_struct_new, Box as RavenBox, Closure as RavenClosure, List as RavenList, Map as RavenMap,
    MapEntry, ObjectHeader, Set as RavenSet, SetEntry, String as RavenString, OBJECT_ALIGN,
    TAG_BOX, TAG_CLOSURE, TAG_LIST, TAG_MAP, TAG_SET, TAG_STRING, TAG_STRUCT,
};
pub use reflect::{
    raven_any_field_names, raven_any_get_field, raven_any_new, raven_any_payload,
    raven_any_set_field, raven_any_type_id, raven_any_type_name, raven_type_register,
};
pub use sched::{
    raven_channel_new, raven_channel_new_buffered, raven_channel_recv, raven_channel_send,
    raven_go_spawn, raven_go_yield,
};

use std::alloc::{self, Layout};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, BufRead, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::process;
use std::slice;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Ignore `SIGPIPE` once, process-wide, on Unix. Writing to a pipe or socket
/// whose read end has closed raises `SIGPIPE`, whose default disposition
/// terminates the process. A compiled Raven program does not go through Rust's
/// std startup (its `main` is a Cranelift-emitted C shim), so the signal is at
/// its default. Ignoring it makes such a write return an ordinary `EPIPE` error
/// the caller already handles, instead of killing the program. Called lazily at
/// the start of the operations that can hit a broken pipe.
fn ignore_sigpipe() {
    #[cfg(unix)]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        });
    }
}

thread_local! {
    // Message of the most recent fallible fs op: empty on success, the OS
    // error text on failure. The Raven wrapper reads it through
    // `raven_fs_last_error` after each call to decide Ok vs Err.
    static FS_LAST_ERROR: RefCell<std::string::String> = const { RefCell::new(std::string::String::new()) };
}

fn fs_set_error(msg: std::string::String) {
    FS_LAST_ERROR.with(|e| *e.borrow_mut() = msg);
}

fn fs_clear_error() {
    FS_LAST_ERROR.with(|e| e.borrow_mut().clear());
}

/// Record the result of a fallible fs op: clear the last error on success,
/// store the OS message on failure.
fn fs_record<T>(r: io::Result<T>) -> Option<T> {
    match r {
        Ok(v) => {
            fs_clear_error();
            Some(v)
        }
        Err(e) => {
            fs_set_error(e.to_string());
            None
        }
    }
}

thread_local! {
    // Message of the most recent fallible time op (parsing): empty on
    // success, the parse error text on failure. The std/time wrapper reads
    // it through `raven_time_last_error` to decide Ok vs Err.
    static TIME_LAST_ERROR: RefCell<std::string::String> = const { RefCell::new(std::string::String::new()) };
}

fn time_set_error(msg: std::string::String) {
    TIME_LAST_ERROR.with(|e| *e.borrow_mut() = msg);
}

fn time_clear_error() {
    TIME_LAST_ERROR.with(|e| e.borrow_mut().clear());
}

thread_local! {
    // Message of the most recent fallible net op: empty on success, the OS
    // error text on failure. The std/net wrapper reads it through
    // `raven_net_last_error` to decide Ok vs Err.
    static NET_LAST_ERROR: RefCell<std::string::String> = const { RefCell::new(std::string::String::new()) };
}

fn net_set_error(msg: std::string::String) {
    NET_LAST_ERROR.with(|e| *e.borrow_mut() = msg);
}

fn net_clear_error() {
    NET_LAST_ERROR.with(|e| e.borrow_mut().clear());
}

thread_local! {
    // Message of the most recent fallible http op: empty on success, the
    // transport error text on failure. The std/http wrapper reads it
    // through `raven_http_last_error` to decide Ok vs Err. A non-2xx HTTP
    // status is NOT a failure here; only transport, DNS, connect, or
    // timeout errors set this slot.
    static HTTP_LAST_ERROR: RefCell<std::string::String> = const { RefCell::new(std::string::String::new()) };
}

fn http_set_error(msg: std::string::String) {
    HTTP_LAST_ERROR.with(|e| *e.borrow_mut() = msg);
}

fn http_clear_error() {
    HTTP_LAST_ERROR.with(|e| e.borrow_mut().clear());
}

thread_local! {
    // Message of the most recent failed regex compile: empty on success,
    // the syntax error text on failure. The std/regex wrapper reads it
    // through `raven_regex_last_error` to decide Ok vs Err.
    static REGEX_LAST_ERROR: RefCell<std::string::String> = const { RefCell::new(std::string::String::new()) };
}

fn regex_set_error(msg: std::string::String) {
    REGEX_LAST_ERROR.with(|e| *e.borrow_mut() = msg);
}

fn regex_clear_error() {
    REGEX_LAST_ERROR.with(|e| e.borrow_mut().clear());
}

thread_local! {
    // Message of the most recent failed subprocess spawn: empty on success,
    // the OS error text on failure. The std/process wrapper reads it through
    // `raven_process_last_error` to decide Ok vs Err. A child that runs but
    // exits non-zero is NOT a failure here; only a spawn error sets this.
    static PROCESS_LAST_ERROR: RefCell<std::string::String> = const { RefCell::new(std::string::String::new()) };
}

fn process_set_error(msg: std::string::String) {
    PROCESS_LAST_ERROR.with(|e| *e.borrow_mut() = msg);
}

fn process_clear_error() {
    PROCESS_LAST_ERROR.with(|e| e.borrow_mut().clear());
}

/// A finished child's captured output. The child runs to completion in one
/// call and its exit code, stdout, and stderr are stored here keyed by an
/// id so only an opaque integer crosses the FFI.
struct ProcResult {
    code: i64,
    // Raw bytes, not text: a child can write arbitrary binary to stdout/stderr
    // and a Raven String is a byte buffer, so the output is captured verbatim.
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// The process-wide child-result registry keyed by an incrementing id. Ids
/// start at 1; 0 is the spawn-failure sentinel that pairs with a set
/// last-error.
fn process_registry() -> &'static Mutex<HashMap<i64, ProcResult>> {
    static REGISTRY: OnceLock<Mutex<HashMap<i64, ProcResult>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Issue the next child-result id.
fn process_next_id() -> i64 {
    static NEXT_ID: AtomicI64 = AtomicI64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// The process-wide compiled-regex registry keyed by an incrementing id.
/// Ids start at 1; 0 is the failure sentinel that pairs with a set
/// last-error.
fn regex_registry() -> &'static Mutex<HashMap<i64, regex::bytes::Regex>> {
    static REGISTRY: OnceLock<Mutex<HashMap<i64, regex::bytes::Regex>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Issue the next compiled-regex id.
fn regex_next_id() -> i64 {
    static NEXT_ID: AtomicI64 = AtomicI64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// A response captured eagerly into owned data. ureq consumes the
/// response when its body is read, so the whole request runs in one call
/// and the status, headers, and body are stored here keyed by an id.
struct HttpResp {
    status: i64,
    status_text: std::string::String,
    // Response headers as `Key: Value` lines joined by `\n`.
    headers: std::string::String,
    // The raw response body bytes. A Raven String is a byte buffer, so the body
    // is kept as raw bytes rather than a lossily UTF-8-decoded `String`, which
    // would corrupt a binary or non-UTF-8 response.
    body: Vec<u8>,
}

/// The process-wide HTTP response registry keyed by an incrementing id.
/// Ids start at 1; 0 is the failure sentinel that pairs with a set
/// last-error.
fn http_registry() -> &'static Mutex<HashMap<i64, HttpResp>> {
    static REGISTRY: OnceLock<Mutex<HashMap<i64, HttpResp>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Issue the next HTTP response id. Shared by the buffered-response and
/// streaming registries, so an id is unique across both.
fn http_next_id() -> i64 {
    static NEXT_ID: AtomicI64 = AtomicI64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// A live streaming response: the status line and headers captured at open,
/// the body left unread behind a reader that `raven_http_stream_read` pulls
/// chunk by chunk. The reader is `take`n out of the entry for the duration
/// of a read so the registry lock is never held across a blocking read; it
/// is `None` while a read is in flight, after end of stream, and for a
/// bodiless (HEAD) response, all of which read as end of stream.
struct HttpStreamEntry {
    status: i64,
    status_text: std::string::String,
    // Response headers as `Key: Value` lines joined by `\n`.
    headers: std::string::String,
    reader: Option<Box<dyn Read + Send + Sync + 'static>>,
}

/// The process-wide streaming-response registry, ids from `http_next_id`.
fn http_stream_registry() -> &'static Mutex<HashMap<i64, HttpStreamEntry>> {
    static REGISTRY: OnceLock<Mutex<HashMap<i64, HttpStreamEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// An entry in the socket registry. A listener or a stream is kept
/// runtime-side so only an opaque integer id crosses the FFI.
enum Socket {
    Listener(TcpListener),
    Stream(TcpStream),
}

/// The process-wide socket registry keyed by an incrementing id. Ids start
/// at 1; 0 (or any non-positive value) is the failure sentinel that pairs
/// with a set last-error.
fn net_registry() -> &'static Mutex<HashMap<i64, Socket>> {
    static REGISTRY: OnceLock<Mutex<HashMap<i64, Socket>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Issue the next socket id.
fn net_next_id() -> i64 {
    static NEXT_ID: AtomicI64 = AtomicI64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Insert a socket and return its id, clearing the last error.
fn net_insert(sock: Socket) -> i64 {
    let id = net_next_id();
    net_registry().lock().unwrap().insert(id, sock);
    net_clear_error();
    id
}

/// Remove a connected TCP stream from the registry and return ownership of the
/// socket, so another subsystem can take it over (std/tls upgrade for STARTTLS).
/// Returns None if the id is unknown or names a listener, leaving the registry
/// unchanged in that case.
pub(crate) fn net_take_stream(id: i64) -> Option<TcpStream> {
    let mut reg = net_registry().lock().unwrap();
    if matches!(reg.get(&id), Some(Socket::Stream(_))) {
        if let Some(Socket::Stream(stream)) = reg.remove(&id) {
            return Some(stream);
        }
    }
    None
}

/// Allocate `size` bytes aligned to `align`.
///
/// Returns null on allocation failure or invalid layout. The current
/// implementation is a thin `std::alloc::alloc` passthrough; the real
/// allocator lands with issue #64.
///
/// # Safety
///
/// The caller must pair every non-null return with exactly one
/// `raven_dealloc` using the same `size` and `align`.
#[no_mangle]
pub extern "C" fn raven_alloc(size: usize, align: usize) -> *mut u8 {
    let Ok(layout) = Layout::from_size_align(size, align) else {
        return std::ptr::null_mut();
    };
    if layout.size() == 0 {
        // A zero-sized allocation is well defined to return a non-null
        // dangling pointer. `std::alloc::alloc` is UB at size zero.
        return align as *mut u8;
    }
    // SAFETY: layout has a non-zero size, validated above.
    unsafe { alloc::alloc(layout) }
}

/// Free an allocation previously returned by `raven_alloc`.
///
/// A null pointer or zero-sized allocation is a no-op.
///
/// # Safety
///
/// `ptr` must come from a matching `raven_alloc(size, align)` call,
/// and must not have been freed already.
#[no_mangle]
pub extern "C" fn raven_dealloc(ptr: *mut u8, size: usize, align: usize) {
    if ptr.is_null() {
        return;
    }
    let Ok(layout) = Layout::from_size_align(size, align) else {
        return;
    };
    if layout.size() == 0 {
        return;
    }
    // SAFETY: per the contract, `ptr` matches a previous allocation
    // with this exact layout.
    unsafe { alloc::dealloc(ptr, layout) };
}

/// Report a fatal Raven panic and terminate the process.
///
/// Writes `raven panic: <msg>\n` to standard error and exits with
/// status 101 (the same code Rust uses for unwinding panics that hit
/// `abort`). Does not unwind.
///
/// # Safety
///
/// `msg_ptr` must point to `msg_len` initialized bytes of valid UTF-8.
/// `msg_len` may be zero, in which case `msg_ptr` is not dereferenced.
#[no_mangle]
pub extern "C" fn raven_panic(msg_ptr: *const u8, msg_len: usize) -> ! {
    let msg = if msg_len == 0 || msg_ptr.is_null() {
        ""
    } else {
        // SAFETY: caller guarantees the slice is initialized UTF-8.
        let bytes = unsafe { slice::from_raw_parts(msg_ptr, msg_len) };
        std::str::from_utf8(bytes).unwrap_or("<invalid utf-8>")
    };
    let stderr = io::stderr();
    let mut handle = stderr.lock();
    // Best-effort write; we are about to exit either way.
    let _ = writeln!(handle, "raven panic: {msg}");
    let _ = handle.flush();
    process::exit(101);
}

/// Write a UTF-8 byte slice to standard output without a trailing
/// newline.
///
/// # Safety
///
/// `ptr` must point to `len` initialized UTF-8 bytes, or `len` must be
/// zero.
#[no_mangle]
pub extern "C" fn raven_print_str(ptr: *const u8, len: usize) {
    if len == 0 || ptr.is_null() {
        return;
    }
    // Writing to stdout after its pipe reader has exited would raise SIGPIPE and
    // kill the program; ignore it so the write returns an ignored EPIPE instead.
    ignore_sigpipe();
    // SAFETY: caller guarantees the slice is initialized.
    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let _ = handle.write_all(bytes);
}

/// Write a UTF-8 byte slice to standard output followed by a single
/// `\n`.
///
/// # Safety
///
/// `ptr` must point to `len` initialized UTF-8 bytes, or `len` must be
/// zero.
#[no_mangle]
pub extern "C" fn raven_println_str(ptr: *const u8, len: usize) {
    // See `raven_print_str`: ignore SIGPIPE so a closed stdout reader cannot
    // terminate the program mid-write.
    ignore_sigpipe();
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if len > 0 && !ptr.is_null() {
        // SAFETY: caller guarantees the slice is initialized.
        let bytes = unsafe { slice::from_raw_parts(ptr, len) };
        let _ = handle.write_all(bytes);
    }
    let _ = handle.write_all(b"\n");
}

/// Read one line from standard input and return it as a heap `String`.
///
/// The trailing line terminator is stripped: a final `\n` is dropped,
/// and a preceding `\r` (Windows `\r\n`) is dropped with it. At end of
/// input (no bytes read) an empty `String` is returned, so a caller can
/// always treat the result as a valid `String` pointer.
///
/// Returns null only when the underlying `String` allocation fails.
#[no_mangle]
pub extern "C" fn raven_read_line() -> *mut object::String {
    // Read raw bytes, not UTF-8 text: a Raven String is a byte buffer, so a line
    // with a non-UTF-8 byte must be preserved rather than discarded. `read_line`
    // requires valid UTF-8 and drops the line on a bad byte; `read_until` reads
    // the bytes up to the newline regardless.
    let mut line: Vec<u8> = Vec::new();
    let stdin = io::stdin();
    // A read error or clean EOF both leave `line` as the bytes gathered
    // so far (empty at a clean EOF); either way we hand back a String.
    // Reading a line blocks on input, so run it outside the collector's
    // running set (a goroutine waiting on stdin must not freeze a collection).
    crate::gc::blocking(|| {
        let _ = stdin.lock().read_until(b'\n', &mut line);
    });
    // Strip the trailing newline and an optional preceding carriage
    // return so callers see the line content without the terminator.
    if line.last() == Some(&b'\n') {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
    }
    object::raven_string_from_bytes(line.as_ptr(), line.len())
}

/// Look up an environment variable and return its value as a heap
/// `String`. Returns an empty `String` when the variable is unset or its
/// value is not valid UTF-8, so the caller always gets a usable pointer.
/// Pair with `raven_env_has` to tell unset apart from an empty value.
///
/// # Safety
///
/// `name` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_env_get(name: *const object::String) -> *mut object::String {
    let value = env_name(name)
        .and_then(std::env::var_os)
        .and_then(|v| v.into_string().ok())
        .unwrap_or_default();
    object::raven_string_from_bytes(value.as_ptr(), value.len())
}

/// Report whether an environment variable is set, regardless of value.
///
/// # Safety
///
/// `name` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_env_has(name: *const object::String) -> bool {
    env_name(name).is_some_and(|n| std::env::var_os(n).is_some())
}

/// Number of process arguments, including the program path at index 0.
#[no_mangle]
pub extern "C" fn raven_env_arg_count() -> i64 {
    std::env::args_os().count() as i64
}

/// The process argument at `index` as a heap `String`. Index 0 is the
/// program path. Returns an empty `String` when `index` is out of range
/// or the argument is not valid UTF-8.
#[no_mangle]
pub extern "C" fn raven_env_arg_at(index: i64) -> *mut object::String {
    let value = usize::try_from(index)
        .ok()
        .and_then(|i| std::env::args_os().nth(i))
        .and_then(|a| a.into_string().ok())
        .unwrap_or_default();
    object::raven_string_from_bytes(value.as_ptr(), value.len())
}

/// Terminate the process with `code`. Does not return.
#[no_mangle]
pub extern "C" fn raven_env_exit(code: i64) -> ! {
    process::exit(code as i32);
}

/// The target operating system name: "windows", "linux", or "macos".
/// Falls back to "unknown" on other targets.
#[no_mangle]
pub extern "C" fn raven_env_os_name() -> *mut object::String {
    let name = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    };
    object::raven_string_from_bytes(name.as_ptr(), name.len())
}

/// The target CPU architecture name, for example "x86_64" or "aarch64".
#[no_mangle]
pub extern "C" fn raven_env_arch() -> *mut object::String {
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else if cfg!(target_arch = "arm") {
        "arm"
    } else {
        "unknown"
    };
    object::raven_string_from_bytes(arch.as_ptr(), arch.len())
}

/// Borrow a Raven `String` argument as a `&str` for an env lookup.
///
/// # Safety
///
/// `name` must be a valid `raven_string_from_bytes`-built `String` or
/// null.
fn env_name<'a>(name: *const object::String) -> Option<&'a str> {
    if name.is_null() {
        return None;
    }
    let ptr = object::raven_string_bytes(name);
    let len = object::raven_string_len(name) as usize;
    if ptr.is_null() {
        return Some("");
    }
    // SAFETY: a Raven String holds `len` initialized UTF-8 bytes.
    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(bytes).ok()
}

/// Borrow the raw bytes of a Raven `String`. Unlike [`env_name`], this does not
/// require valid UTF-8: a `String` is a byte buffer, so callers that handle
/// arbitrary bytes (the regex engine) use this. The slice borrows the
/// `String`'s storage, so it is valid for the duration of the FFI call.
///
/// # Safety
///
/// `s` must be a valid `raven_string_from_bytes`-built `String` or null.
fn string_bytes<'a>(s: *const object::String) -> &'a [u8] {
    if s.is_null() {
        return &[];
    }
    let ptr = object::raven_string_bytes(s);
    let len = object::raven_string_len(s) as usize;
    if ptr.is_null() || len == 0 {
        return &[];
    }
    // SAFETY: a Raven String holds `len` initialized bytes.
    unsafe { slice::from_raw_parts(ptr, len) }
}

/// Reinterpret a signed 64-bit integer as an `f64` by value conversion.
///
/// The v2 surface language has no Int-to-Float cast, so `std/random`
/// binds this symbol through `extern "C"` to build a `Float` in
/// `[0.0, 1.0)` from generated bits.
#[no_mangle]
pub extern "C" fn raven_int_to_float(value: i64) -> f64 {
    value as f64
}

/// Truncate an `f64` toward zero to a signed 64-bit integer.
///
/// The v2 surface language has no Float-to-Int cast, so `std/json` binds
/// this symbol through `extern "C"` to recover an `Int` field from a JSON
/// number (which always parses to `Float`). A value outside the `i64`
/// range saturates to `i64::MIN`/`i64::MAX`, and `NaN` maps to `0`, the
/// usual `as` cast behavior.
#[no_mangle]
pub extern "C" fn raven_float_to_int(value: f64) -> i64 {
    value as i64
}

/// Copy a Raven `String` into a freshly allocated, null-terminated byte
/// buffer and return a `*const c_char` (`CStr`) to its first byte.
///
/// The returned buffer is a standalone copy, not the String's own bytes,
/// so embedded NUL bytes in the String are preserved up to the first one
/// a C reader will stop at. The buffer is allocated outside the GC heap
/// and is intentionally not reclaimed: it is valid for the rest of the
/// program run. Use this for short-lived calls into C (for example
/// `strlen`). Backs `std/ffi`'s `to_cstr`.
///
/// # Safety
///
/// `s` must be a valid `raven_string_from_bytes`-built `String` or null.
#[no_mangle]
pub extern "C" fn raven_string_to_cstr(s: *const object::String) -> *const u8 {
    let len = object::raven_string_len(s) as usize;
    // One extra byte for the terminating NUL. The buffer is `malloc`-ed (not
    // GC- or `raven_alloc`-managed) so the caller can release it with
    // `raven_cstr_free` (std/ffi's `free_cstr`), which is plain `free`.
    // SAFETY: `malloc` is the C allocator; a null return is handled below.
    let buf = unsafe { malloc(len + 1) } as *mut u8;
    if buf.is_null() {
        return std::ptr::null();
    }
    let src = object::raven_string_bytes(s);
    // SAFETY: `buf` has `len + 1` writable bytes; `src` holds `len`
    // initialized bytes (or is null when `len` is zero, guarded here).
    unsafe {
        if len > 0 && !src.is_null() {
            std::ptr::copy_nonoverlapping(src, buf, len);
        }
        *buf.add(len) = 0;
    }
    buf as *const u8
}

/// Free a buffer returned by `raven_string_to_cstr` (std/ffi's `to_cstr`).
///
/// A null pointer is a no-op. Only a `to_cstr` result may be passed: a
/// `c"..."` literal is static and a `CStr` from another source has a
/// different owner.
///
/// # Safety
///
/// `p` must be a live pointer returned by `raven_string_to_cstr` and not
/// already freed.
#[no_mangle]
pub extern "C" fn raven_cstr_free(p: *const u8) {
    if p.is_null() {
        return;
    }
    // SAFETY: `to_cstr` allocates the buffer with `malloc`, so `free` matches.
    unsafe { free(p as *mut std::ffi::c_void) }
}

/// Read the null-terminated bytes at `p` and build a Raven `String`.
///
/// The length is computed up to the first NUL byte; the terminator is
/// not included in the result. A null `p` yields an empty `String`.
/// Backs `std/ffi`'s `from_cstr`.
///
/// # Safety
///
/// `p` must point to a null-terminated byte sequence or be null.
#[no_mangle]
pub extern "C" fn raven_cstr_to_string(p: *const u8) -> *mut object::String {
    if p.is_null() {
        return object::raven_string_from_bytes(std::ptr::null(), 0);
    }
    let mut len = 0usize;
    // SAFETY: the caller guarantees a NUL terminator, so the scan stops
    // inside the buffer.
    unsafe {
        while *p.add(len) != 0 {
            len += 1;
        }
    }
    object::raven_string_from_bytes(p, len)
}

extern "C" {
    fn malloc(size: usize) -> *mut std::ffi::c_void;
    fn free(ptr: *mut std::ffi::c_void);
}

/// Allocate `bytes` of uninitialized, writable memory outside the GC heap
/// and return its address as a pointer-width integer.
///
/// This is a thin `malloc` wrapper backing `std/ffi`'s `alloc`. The memory
/// is not tracked by the collector and is never reclaimed automatically:
/// the caller must hand the returned pointer to `raven_ffi_free` when done.
/// A request of zero bytes, or an allocation failure, returns 0 (a null
/// pointer).
#[no_mangle]
pub extern "C" fn raven_ffi_alloc(bytes: usize) -> usize {
    if bytes == 0 {
        return 0;
    }
    // SAFETY: `malloc` is the C allocator; a null return is reported as 0.
    let p = unsafe { malloc(bytes) };
    p as usize
}

/// Free a buffer previously returned by `raven_ffi_alloc`.
///
/// A null pointer (0) is a no-op, matching C `free`.
///
/// # Safety
///
/// `p` must be a live pointer returned by `raven_ffi_alloc` and not already
/// freed.
#[no_mangle]
pub extern "C" fn raven_ffi_free(p: usize) {
    if p == 0 {
        return;
    }
    // SAFETY: the caller guarantees `p` came from `raven_ffi_alloc` and is
    // not freed twice.
    unsafe { free(p as *mut std::ffi::c_void) }
}

/// Test helper: write `val` into the first `n` 32-bit slots at `p`.
///
/// Proves a pointer Raven hands to a C function refers to the same memory
/// the C side writes through. Used by `examples/v2/ffi_pointers.rv`.
///
/// # Safety
///
/// `p` must point to at least `n` writable `i32` slots.
#[no_mangle]
pub extern "C" fn raven_ffi_fill_i32(p: *mut i32, n: i32, val: i32) {
    if p.is_null() || n <= 0 {
        return;
    }
    // SAFETY: the caller guarantees `n` writable i32 slots at `p`.
    unsafe {
        for i in 0..n as isize {
            *p.offset(i) = val;
        }
    }
}

/// Test helper: sort the first `n` 32-bit slots at `p` with the C library
/// `qsort`, using `cmp` as the comparator.
///
/// Proves a non-capturing top-level Raven function passed as a `CFnPtr`
/// is invoked correctly by C: the C `qsort` calls back into the Raven
/// comparator for each pair. Used by `examples/v2/ffi_callback.rv`.
///
/// # Safety
///
/// `p` must point to at least `n` writable `i32` slots, and `cmp` must be
/// a valid comparator that reads two `*const i32` arguments.
#[no_mangle]
pub extern "C" fn raven_ffi_qsort_i32(
    p: *mut i32,
    n: usize,
    cmp: extern "C" fn(*const i32, *const i32) -> i32,
) {
    extern "C" {
        fn qsort(
            base: *mut core::ffi::c_void,
            nmemb: usize,
            size: usize,
            compar: extern "C" fn(*const i32, *const i32) -> i32,
        );
    }
    if p.is_null() || n == 0 {
        return;
    }
    // SAFETY: the caller guarantees `n` writable i32 slots at `p` and a
    // valid comparator; `qsort` reads each pair through `cmp`.
    unsafe {
        qsort(
            p as *mut core::ffi::c_void,
            n,
            core::mem::size_of::<i32>(),
            cmp,
        );
    }
}

/// A C-layout point passed and returned by value across the FFI.
///
/// Two `int` fields, eight bytes, the by-value struct shape Raven's
/// `@repr(C)` slice supports. `repr(C)` here matches the C ABI both sides
/// agree on. Used by `examples/v2/ffi_struct.rv`.
#[repr(C)]
pub struct RavenFfiPoint {
    pub x: i32,
    pub y: i32,
}

/// Test helper: translate a point by `(dx, dy)`, taking and returning the
/// point by value. Proves a small C struct crosses the FFI in both
/// directions with its fields intact.
#[no_mangle]
pub extern "C" fn raven_ffi_translate(p: RavenFfiPoint, dx: i32, dy: i32) -> RavenFfiPoint {
    RavenFfiPoint {
        x: p.x + dx,
        y: p.y + dy,
    }
}

/// Test helper: sum a point's fields, taking the point by value. Proves a
/// by-value struct argument reaches C with its fields in the right slots.
#[no_mangle]
pub extern "C" fn raven_ffi_point_sum(p: RavenFfiPoint) -> i32 {
    p.x + p.y
}

/// Return a non-deterministic 64-bit seed for entropy seeding.
///
/// Mixes a high-resolution timestamp, the process id, and a process-global
/// call counter through a splitmix64 finalizer. The counter guarantees
/// successive calls differ even within one clock tick: the finalizer is a
/// bijection, so distinct mixed inputs map to distinct outputs. This is a
/// seed source, not a cryptographic random generator.
#[no_mangle]
pub extern "C" fn raven_random_entropy() -> i64 {
    static SEQ: AtomicI64 = AtomicI64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed) as u64;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut z = nanos ^ (u64::from(process::id()) << 32) ^ seq.wrapping_mul(0x9E3779B97F4A7C15);
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z = z ^ (z >> 31);
    z as i64
}

/// The message of the most recent fallible fs op, empty when it
/// succeeded. The Raven wrapper reads this to build an `Err` only when it
/// is non-empty.
#[no_mangle]
pub extern "C" fn raven_fs_last_error() -> *mut object::String {
    FS_LAST_ERROR.with(|e| {
        let msg = e.borrow();
        object::raven_string_from_bytes(msg.as_ptr(), msg.len())
    })
}

/// Read the whole file at `path` as a `String`. On failure stores the OS
/// message in the last-error slot and returns an empty `String`.
///
/// # Safety
///
/// `path` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_fs_read(path: *const object::String) -> *mut object::String {
    // Read the raw bytes, not UTF-8 text: a Raven String is a byte buffer, and
    // `raven_fs_write` already writes arbitrary bytes, so a binary file written
    // by one program must read back through the same API. Only the path needs
    // to be valid UTF-8.
    let contents = crate::gc::blocking(|| {
        env_name(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not valid UTF-8"))
            .and_then(std::fs::read)
    });
    let value = fs_record(contents).unwrap_or_default();
    object::raven_string_from_bytes(value.as_ptr(), value.len())
}

/// Create or truncate `path` and write `contents` to it.
///
/// # Safety
///
/// Both arguments must be valid `raven_string_from_bytes`-built `String`s.
#[no_mangle]
pub extern "C" fn raven_fs_write(
    path: *const object::String,
    contents: *const object::String,
) -> bool {
    // The contents are written as raw bytes: a Raven String is a byte buffer
    // that may hold arbitrary (non-UTF-8) data, for example binary read back
    // from another file. Only the path must be valid UTF-8.
    let ptr = object::raven_string_bytes(contents);
    let len = object::raven_string_len(contents) as usize;
    let bytes: &[u8] = if ptr.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: a Raven String holds `len` initialized bytes.
        unsafe { slice::from_raw_parts(ptr, len) }
    };
    let result = crate::gc::blocking(|| match env_name(path) {
        Some(p) => std::fs::write(p, bytes),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is not valid UTF-8",
        )),
    });
    fs_record(result).is_some()
}

/// Append `contents` to `path`, creating it when absent.
///
/// # Safety
///
/// Both arguments must be valid `raven_string_from_bytes`-built `String`s.
#[no_mangle]
pub extern "C" fn raven_fs_append(
    path: *const object::String,
    contents: *const object::String,
) -> bool {
    // Append raw bytes for the same reason as `raven_fs_write`: a Raven String
    // may hold non-UTF-8 data. Only the path must be valid UTF-8.
    let ptr = object::raven_string_bytes(contents);
    let len = object::raven_string_len(contents) as usize;
    let bytes: &[u8] = if ptr.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: a Raven String holds `len` initialized bytes.
        unsafe { slice::from_raw_parts(ptr, len) }
    };
    let result = crate::gc::blocking(|| match env_name(path) {
        Some(p) => std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .and_then(|mut f| f.write_all(bytes)),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is not valid UTF-8",
        )),
    });
    fs_record(result).is_some()
}

/// Remove the file at `path`.
///
/// # Safety
///
/// `path` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_fs_remove_file(path: *const object::String) -> bool {
    let result = crate::gc::blocking(|| {
        env_name(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not valid UTF-8"))
            .and_then(std::fs::remove_file)
    });
    fs_record(result).is_some()
}

/// Create the directory at `path`, including any missing parents.
///
/// # Safety
///
/// `path` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_fs_create_dir(path: *const object::String) -> bool {
    let result = crate::gc::blocking(|| {
        env_name(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not valid UTF-8"))
            .and_then(std::fs::create_dir_all)
    });
    fs_record(result).is_some()
}

/// Remove the directory at `path` and all of its contents.
///
/// # Safety
///
/// `path` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_fs_remove_dir(path: *const object::String) -> bool {
    let result = crate::gc::blocking(|| {
        env_name(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not valid UTF-8"))
            .and_then(std::fs::remove_dir_all)
    });
    fs_record(result).is_some()
}

/// The raw bytes of a directory entry name. On Unix a filename is an arbitrary
/// non-NUL byte string, so the exact bytes are preserved (a non-UTF-8 name would
/// otherwise be mangled by a lossy decode). On other platforms a name is Unicode,
/// rendered as its UTF-8 bytes.
#[cfg(unix)]
fn os_name_bytes(name: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    name.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_name_bytes(name: &std::ffi::OsStr) -> Vec<u8> {
    name.to_string_lossy().into_owned().into_bytes()
}

/// List the entry names of the directory at `path`, joined by a NUL byte. A NUL
/// cannot occur in a filename on any platform, so it separates names without
/// risk of splitting a name that contains a newline; the raw filename bytes are
/// preserved so a non-UTF-8 Unix name round-trips. An empty directory yields an
/// empty `String`. On failure stores the OS message and returns an empty
/// `String`.
///
/// # Safety
///
/// `path` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_fs_list(path: *const object::String) -> *mut object::String {
    let result = crate::gc::blocking(|| {
        env_name(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not valid UTF-8"))
            .and_then(|p| {
                let mut out: Vec<u8> = Vec::new();
                for entry in std::fs::read_dir(p)? {
                    let entry = entry?;
                    if !out.is_empty() {
                        out.push(0);
                    }
                    out.extend_from_slice(&os_name_bytes(&entry.file_name()));
                }
                Ok(out)
            })
    });
    let value = fs_record(result).unwrap_or_default();
    object::raven_string_from_bytes(value.as_ptr(), value.len())
}

/// File size at `path` in bytes. On failure stores the OS message and
/// returns 0.
///
/// # Safety
///
/// `path` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_fs_size(path: *const object::String) -> i64 {
    let result = crate::gc::blocking(|| {
        env_name(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not valid UTF-8"))
            .and_then(std::fs::metadata)
            .map(|m| m.len() as i64)
    });
    fs_record(result).unwrap_or(0)
}

/// Whether anything exists at `path`. Not fallible: a missing path is a
/// normal `false`.
///
/// # Safety
///
/// `path` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_fs_exists(path: *const object::String) -> bool {
    env_name(path).is_some_and(|p| std::path::Path::new(p).exists())
}

/// Whether `path` is a regular file. A missing path is `false`.
///
/// # Safety
///
/// `path` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_fs_is_file(path: *const object::String) -> bool {
    env_name(path).is_some_and(|p| std::path::Path::new(p).is_file())
}

/// Whether `path` is a directory. A missing path is `false`.
///
/// # Safety
///
/// `path` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_fs_is_dir(path: *const object::String) -> bool {
    env_name(path).is_some_and(|p| std::path::Path::new(p).is_dir())
}

/// Whether `path` itself is a symbolic link. Unlike `is_dir`, this does not
/// follow the link: it reads the link's own metadata. A missing path is
/// `false`. Used by `fs.walk` to avoid descending through a symlinked
/// directory, which could form a cycle.
///
/// # Safety
///
/// `path` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_fs_is_symlink(path: *const object::String) -> bool {
    env_name(path)
        .is_some_and(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()))
}

/// The message of the most recent fallible time op (parsing), empty when
/// it succeeded. The std/time wrapper reads this to build an `Err` only
/// when it is non-empty.
#[no_mangle]
pub extern "C" fn raven_time_last_error() -> *mut object::String {
    TIME_LAST_ERROR.with(|e| {
        let msg = e.borrow();
        object::raven_string_from_bytes(msg.as_ptr(), msg.len())
    })
}

/// Current Unix timestamp in whole seconds (UTC).
#[no_mangle]
pub extern "C" fn raven_time_now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Current Unix time in milliseconds (UTC).
#[no_mangle]
pub extern "C" fn raven_time_now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The UTC datetime for a Unix timestamp in seconds, or the epoch when the
/// timestamp is out of chrono's representable range.
fn time_from_ts(ts: i64) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    match chrono::Utc.timestamp_opt(ts, 0) {
        chrono::LocalResult::Single(dt) => dt,
        _ => chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap(),
    }
}

/// Calendar year of `ts` (UTC).
#[no_mangle]
pub extern "C" fn raven_time_year(ts: i64) -> i64 {
    use chrono::Datelike;
    time_from_ts(ts).year() as i64
}

/// Month of `ts` (UTC), 1 through 12.
#[no_mangle]
pub extern "C" fn raven_time_month(ts: i64) -> i64 {
    use chrono::Datelike;
    time_from_ts(ts).month() as i64
}

/// Day of month of `ts` (UTC), 1 through 31.
#[no_mangle]
pub extern "C" fn raven_time_day(ts: i64) -> i64 {
    use chrono::Datelike;
    time_from_ts(ts).day() as i64
}

/// Hour of `ts` (UTC), 0 through 23.
#[no_mangle]
pub extern "C" fn raven_time_hour(ts: i64) -> i64 {
    use chrono::Timelike;
    time_from_ts(ts).hour() as i64
}

/// Minute of `ts` (UTC), 0 through 59.
#[no_mangle]
pub extern "C" fn raven_time_minute(ts: i64) -> i64 {
    use chrono::Timelike;
    time_from_ts(ts).minute() as i64
}

/// Second of `ts` (UTC), 0 through 59 (60 on a leap second).
#[no_mangle]
pub extern "C" fn raven_time_second(ts: i64) -> i64 {
    use chrono::Timelike;
    time_from_ts(ts).second() as i64
}

/// Weekday of `ts` (UTC) as 0 for Sunday through 6 for Saturday.
#[no_mangle]
pub extern "C" fn raven_time_weekday(ts: i64) -> i64 {
    use chrono::Datelike;
    time_from_ts(ts).weekday().num_days_from_sunday() as i64
}

/// Format the UTC datetime of `ts` with a chrono strftime `pattern`.
///
/// # Safety
///
/// `pattern` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_time_format(
    ts: i64,
    pattern: *const object::String,
) -> *mut object::String {
    let dt = time_from_ts(ts);
    let value = match env_name(pattern) {
        // chrono's formatter reports an invalid directive through its `Display`
        // error. `to_string()` panics on that, which cannot unwind across the C
        // ABI and aborts the process, so format fallibly and return the empty
        // string for a bad pattern instead.
        Some(p) => {
            use std::fmt::Write;
            let mut out = std::string::String::new();
            if write!(out, "{}", dt.format(p)).is_err() {
                out.clear();
            }
            out
        }
        None => std::string::String::new(),
    };
    object::raven_string_from_bytes(value.as_ptr(), value.len())
}

/// Parse `text` as a UTC datetime by the chrono strftime `pattern`,
/// returning the Unix timestamp in seconds. On failure stores the parse
/// error in the last-error slot and returns 0.
///
/// # Safety
///
/// Both arguments must be valid `raven_string_from_bytes`-built `String`s.
#[no_mangle]
pub extern "C" fn raven_time_parse(
    text: *const object::String,
    pattern: *const object::String,
) -> i64 {
    let parsed = match (env_name(text), env_name(pattern)) {
        (Some(t), Some(p)) => chrono::NaiveDateTime::parse_from_str(t, p)
            .map_err(|e| e.to_string())
            .map(|dt| dt.and_utc().timestamp()),
        _ => Err("text or pattern is not valid UTF-8".to_string()),
    };
    match parsed {
        Ok(ts) => {
            time_clear_error();
            ts
        }
        Err(msg) => {
            time_set_error(msg);
            0
        }
    }
}

/// Sleep the current thread for `ms` milliseconds. A negative value is
/// treated as zero.
#[no_mangle]
pub extern "C" fn raven_time_sleep_millis(ms: i64) {
    let ms = ms.max(0) as u64;
    // Sleeping blocks the thread for the whole duration; leave the collector's
    // running set so a collection on another thread is not stalled until it
    // wakes.
    crate::gc::blocking(|| std::thread::sleep(std::time::Duration::from_millis(ms)));
}

/// The message of the most recent fallible net op, empty when it
/// succeeded. The std/net wrapper reads this to build an `Err` only when it
/// is non-empty.
#[no_mangle]
pub extern "C" fn raven_net_last_error() -> *mut object::String {
    NET_LAST_ERROR.with(|e| {
        let msg = e.borrow();
        object::raven_string_from_bytes(msg.as_ptr(), msg.len())
    })
}

/// Connect a TCP stream to `addr` ("host:port") and register it. Returns
/// the stream id, or 0 on failure with the last-error slot set.
///
/// # Safety
///
/// `addr` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_net_connect(addr: *const object::String) -> i64 {
    // The connect (DNS + TCP handshake) can block for seconds; run it outside
    // the collector's running set so a concurrent collection is not frozen.
    let result = crate::gc::blocking(|| {
        env_name(addr)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "addr is not valid UTF-8"))
            .and_then(TcpStream::connect)
    });
    match result {
        Ok(stream) => net_insert(Socket::Stream(stream)),
        Err(e) => {
            net_set_error(e.to_string());
            0
        }
    }
}

/// Bind a TCP listener to `addr` ("host:port") and register it. Returns the
/// listener id, or 0 on failure with the last-error slot set.
///
/// # Safety
///
/// `addr` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_net_listen(addr: *const object::String) -> i64 {
    let result = env_name(addr)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "addr is not valid UTF-8"))
        .and_then(TcpListener::bind);
    match result {
        Ok(listener) => net_insert(Socket::Listener(listener)),
        Err(e) => {
            net_set_error(e.to_string());
            0
        }
    }
}

/// Block until a connection arrives on the listener `listener_id`, register
/// the accepted stream, and return its id. Returns 0 on failure (unknown
/// id, the id is not a listener, or an accept error) with the last-error
/// slot set.
#[no_mangle]
pub extern "C" fn raven_net_accept(listener_id: i64) -> i64 {
    // accept blocks until a connection arrives. Clone the listener handle under
    // the registry lock, then accept without holding it, so a parked accept does
    // not serialize every other goroutine's net operation on the shared
    // registry. Run outside the running set so a slow accept never freezes a
    // collection.
    let listener = {
        let registry = net_registry().lock().unwrap();
        match registry.get(&listener_id) {
            Some(Socket::Listener(l)) => l.try_clone(),
            Some(Socket::Stream(_)) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "id is not a listener",
            )),
            None => Err(io::Error::new(io::ErrorKind::NotFound, "unknown socket id")),
        }
    };
    let accepted = match listener {
        Ok(l) => crate::gc::blocking(|| l.accept().map(|(stream, _)| stream)),
        Err(e) => Err(e),
    };
    match accepted {
        Ok(stream) => net_insert(Socket::Stream(stream)),
        Err(e) => {
            net_set_error(e.to_string());
            0
        }
    }
}

/// Read up to `max` bytes from the stream `stream_id` and return them as a
/// `String` (treated as a byte buffer). A clean EOF returns an empty
/// `String` with the last error cleared, so the wrapper can return Ok("").
/// On error sets the last error and returns an empty `String`.
#[no_mangle]
pub extern "C" fn raven_net_read(stream_id: i64, max: i64) -> *mut object::String {
    // A negative limit is a caller error, not a zero-length read: returning
    // Ok("") would be indistinguishable from a clean EOF.
    if max < 0 {
        net_set_error("read size must be non-negative".to_string());
        return object::raven_string_from_bytes(std::ptr::null(), 0);
    }
    let cap = max as usize;
    // Clone the stream handle under the registry lock, then read without holding
    // it, so a blocked read does not serialize other goroutines' net operations
    // on the shared registry. Block outside the collector's running set so a slow
    // read never freezes a concurrent collection waiting for this worker to reach
    // a safepoint.
    let stream = {
        let registry = net_registry().lock().unwrap();
        match registry.get(&stream_id) {
            Some(Socket::Stream(s)) => s.try_clone(),
            Some(Socket::Listener(_)) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "id is not a stream",
            )),
            None => Err(io::Error::new(io::ErrorKind::NotFound, "unknown socket id")),
        }
    };
    let result = match stream {
        Ok(mut s) => crate::gc::blocking(|| {
            // A very large `max` must fail gracefully: a plain `vec![0; cap]`
            // aborts the process when the allocation fails. Reserve fallibly so
            // an oversized request becomes an error instead.
            let mut buf: Vec<u8> = Vec::new();
            buf.try_reserve_exact(cap).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "read size too large to allocate",
                )
            })?;
            buf.resize(cap, 0);
            let n = s.read(&mut buf)?;
            buf.truncate(n);
            Ok(buf)
        }),
        Err(e) => Err(e),
    };
    match result {
        Ok(bytes) => {
            net_clear_error();
            object::raven_string_from_bytes(bytes.as_ptr(), bytes.len())
        }
        Err(e) => {
            net_set_error(e.to_string());
            object::raven_string_from_bytes(std::ptr::null(), 0)
        }
    }
}

/// Write all bytes of `data` to the stream `stream_id`. Returns the number
/// of bytes written, or -1 on failure with the last error set.
///
/// # Safety
///
/// `data` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_net_write(stream_id: i64, data: *const object::String) -> i64 {
    // Writing to a peer that has closed the connection would raise SIGPIPE and
    // kill the program; ignore it so the write surfaces as an error instead.
    ignore_sigpipe();
    let ptr = object::raven_string_bytes(data);
    let len = object::raven_string_len(data) as usize;
    let bytes: &[u8] = if ptr.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: a Raven String holds `len` initialized bytes.
        unsafe { slice::from_raw_parts(ptr, len) }
    };
    // Clone the stream handle under the registry lock, then write without
    // holding it, so a blocked write does not serialize other goroutines' net
    // operations on the shared registry.
    let stream = {
        let registry = net_registry().lock().unwrap();
        match registry.get(&stream_id) {
            Some(Socket::Stream(s)) => s.try_clone(),
            Some(Socket::Listener(_)) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "id is not a stream",
            )),
            None => Err(io::Error::new(io::ErrorKind::NotFound, "unknown socket id")),
        }
    };
    let result = match stream {
        Ok(mut s) => crate::gc::blocking(|| s.write_all(bytes).map(|()| bytes.len() as i64)),
        Err(e) => Err(e),
    };
    match result {
        Ok(n) => {
            net_clear_error();
            n
        }
        Err(e) => {
            net_set_error(e.to_string());
            -1
        }
    }
}

/// Remove `stream_id` from the registry; dropping the socket closes it. An
/// unknown id is a no-op.
#[no_mangle]
pub extern "C" fn raven_net_close(stream_id: i64) {
    let sock = net_registry().lock().unwrap().remove(&stream_id);
    // Accept/read clone the handle for their blocking syscall, so dropping the
    // registry's copy alone leaves the underlying socket open and a parked
    // reader hung. Shut a stream down so that read returns at once. (A listener
    // has no portable shutdown; dropping its registry copy is the best we can
    // do here.)
    if let Some(Socket::Stream(stream)) = sock {
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }
}

/// Set the read timeout of the stream `stream_id` to `ms` milliseconds. A
/// value of 0 clears the timeout (blocking reads). Errors are stored in the
/// last-error slot.
#[no_mangle]
pub extern "C" fn raven_net_set_read_timeout_ms(stream_id: i64, ms: i64) {
    let timeout = if ms <= 0 {
        None
    } else {
        Some(Duration::from_millis(ms as u64))
    };
    let registry = net_registry().lock().unwrap();
    match registry.get(&stream_id) {
        Some(Socket::Stream(s)) => match s.set_read_timeout(timeout) {
            Ok(()) => net_clear_error(),
            Err(e) => net_set_error(e.to_string()),
        },
        _ => net_set_error("unknown socket id".to_string()),
    }
}

/// Set the write timeout of the stream `stream_id` to `ms` milliseconds. A
/// value of 0 clears the timeout (blocking writes). Errors are stored in the
/// last-error slot.
#[no_mangle]
pub extern "C" fn raven_net_set_write_timeout_ms(stream_id: i64, ms: i64) {
    let timeout = if ms <= 0 {
        None
    } else {
        Some(Duration::from_millis(ms as u64))
    };
    let registry = net_registry().lock().unwrap();
    match registry.get(&stream_id) {
        Some(Socket::Stream(s)) => match s.set_write_timeout(timeout) {
            Ok(()) => net_clear_error(),
            Err(e) => net_set_error(e.to_string()),
        },
        _ => net_set_error("unknown socket id".to_string()),
    }
}

/// Resolve `host` to its IP addresses, newline-joined. On failure stores
/// the error and returns an empty `String`.
///
/// # Safety
///
/// `host` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_dns_lookup(host: *const object::String) -> *mut object::String {
    // DNS resolution blocks on the network, so run it outside the running set.
    let result = crate::gc::blocking(|| {
        env_name(host)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "host is not valid UTF-8"))
            .and_then(|h| {
                let addrs = (h, 0u16).to_socket_addrs()?;
                let ips: Vec<std::string::String> = addrs.map(|sa| sa.ip().to_string()).collect();
                Ok(ips.join("\n"))
            })
    });
    match result {
        Ok(joined) => {
            net_clear_error();
            object::raven_string_from_bytes(joined.as_ptr(), joined.len())
        }
        Err(e) => {
            net_set_error(e.to_string());
            object::raven_string_from_bytes(std::ptr::null(), 0)
        }
    }
}

/// Probe whether `addr` ("host:port") accepts a TCP connection within a
/// short timeout. A pure boolean probe: never sets the last error.
///
/// # Safety
///
/// `addr` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_net_reachable(addr: *const object::String) -> bool {
    let Some(text) = env_name(addr) else {
        return false;
    };
    // Both the DNS resolve and the connect block on the network; run them
    // together outside the collector's running set.
    crate::gc::blocking(|| {
        let Ok(mut targets) = text.to_socket_addrs() else {
            return false;
        };
        targets.any(|sa| TcpStream::connect_timeout(&sa, Duration::from_millis(500)).is_ok())
    })
}

/// The message of the most recent fallible http op, empty when it
/// succeeded. The std/http wrapper reads this to build an `Err` only when
/// it is non-empty. A non-2xx HTTP status never sets it.
#[no_mangle]
pub extern "C" fn raven_http_last_error() -> *mut object::String {
    HTTP_LAST_ERROR.with(|e| {
        let msg = e.borrow();
        object::raven_string_from_bytes(msg.as_ptr(), msg.len())
    })
}

/// The standard reason phrase for an HTTP status code, for callers when
/// the server omits one.
fn http_reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// The response's reason phrase, falling back to the standard phrase for
/// its status code when the server sent none.
fn http_status_text(resp: &ureq::Response) -> std::string::String {
    let reason = resp.status_text();
    if reason.is_empty() {
        http_reason(resp.status()).to_string()
    } else {
        reason.to_string()
    }
}

/// The response headers as `Key: Value` lines joined by `\n`.
fn http_header_lines(resp: &ureq::Response) -> std::string::String {
    let mut header_lines: Vec<std::string::String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in resp.headers_names() {
        // `headers_names` can list a repeated header (two `Set-Cookie` lines)
        // more than once, and `header` only returns its first value. Process
        // each name once and emit one line per value with `all`, so a repeated
        // header keeps every value instead of duplicating the first.
        if !seen.insert(name.to_ascii_lowercase()) {
            continue;
        }
        for value in resp.all(&name) {
            header_lines.push(format!("{name}: {value}"));
        }
    }
    header_lines.join("\n")
}

/// Capture a ureq response into an owned `HttpResp`, reading the status,
/// reason, headers, and body eagerly (reading the body consumes the
/// response).
fn http_capture(resp: ureq::Response, head_request: bool) -> Option<HttpResp> {
    let status = resp.status();
    let status_text = http_status_text(&resp);
    let headers = http_header_lines(&resp);
    // A 1xx, 204, or 304 response, and any response to a HEAD request, carries no
    // body even when Content-Length advertises the length the body would have, so
    // an empty body is correct and must not be reported as a truncated transfer.
    let bodyless = head_request || (100..200).contains(&status) || status == 204 || status == 304;
    // The body length the server promised, if any. A body shorter than this was
    // cut short in transit. Bodyless responses never carry one to verify.
    let expected_len: Option<u64> = if bodyless {
        None
    } else {
        resp.header("Content-Length")
            .and_then(|v| v.trim().parse::<u64>().ok())
    };
    // Read the body as raw bytes (not `into_string`, which lossily replaces a
    // non-UTF-8 byte with U+FFFD and corrupts a binary response).
    let mut body = Vec::new();
    let read = resp.into_reader().read_to_end(&mut body);
    // A read error, or fewer bytes than Content-Length promised, means a
    // truncated transfer. Report it instead of returning the partial body with
    // a success status, so a caller can tell a complete response from a broken
    // one.
    if read.is_err() {
        http_set_error("incomplete response body".to_string());
        return None;
    }
    if let Some(expected) = expected_len {
        if (body.len() as u64) < expected {
            http_set_error(format!(
                "incomplete response body: expected {expected} bytes, received {}",
                body.len()
            ));
            return None;
        }
    }
    Some(HttpResp {
        status: status as i64,
        status_text,
        headers,
        body,
    })
}

/// Store a captured response and return its id, clearing the last error.
fn http_store(resp: HttpResp) -> i64 {
    let id = http_next_id();
    http_registry().lock().unwrap().insert(id, resp);
    http_clear_error();
    id
}

/// Perform an HTTP/1.1 request and store the response.
///
/// `method` is "GET"/"POST"/"PUT"/"DELETE", `url` the target, `body` the
/// request body (empty for GET/DELETE), and `headers` a String of
/// `Key: Value` lines separated by `\n` (empty for none). Returns a
/// response id, or 0 on a transport failure (DNS, connect, timeout) with
/// the last-error slot set.
///
/// A non-2xx HTTP status (for example 404 or 500) is a SUCCESSFUL request
/// that yielded a response: ureq surfaces it as `Error::Status`, and this
/// stores a normal response entry from it. Only `Error::Transport`
/// becomes id 0 plus a last-error.
///
/// # Safety
///
/// `method`, `url`, `body`, and `headers` must be valid
/// `raven_string_from_bytes`-built `String`s.
#[no_mangle]
pub extern "C" fn raven_http_request(
    method: *const object::String,
    url: *const object::String,
    body: *const object::String,
    headers: *const object::String,
) -> i64 {
    http_request_impl(method, url, body, headers, 0)
}

/// Like [`raven_http_request`], with an overall deadline. A positive
/// `timeout_ms` bounds the whole request (connect, send, and reading the
/// body); `timeout_ms <= 0` keeps the default timeouts.
///
/// # Safety
///
/// `method`, `url`, `body`, and `headers` must be valid
/// `raven_string_from_bytes`-built `String`s.
#[no_mangle]
pub extern "C" fn raven_http_request_timeout(
    method: *const object::String,
    url: *const object::String,
    body: *const object::String,
    headers: *const object::String,
    timeout_ms: i64,
) -> i64 {
    http_request_impl(method, url, body, headers, timeout_ms)
}

fn http_request_impl(
    method: *const object::String,
    url: *const object::String,
    body: *const object::String,
    headers: *const object::String,
    timeout_ms: i64,
) -> i64 {
    let (Some(method), Some(url), Some(headers)) =
        (env_name(method), env_name(url), env_name(headers))
    else {
        http_set_error("method, url, or headers is not valid UTF-8".to_string());
        return 0;
    };
    // The request body is sent as raw bytes: a Raven String is a byte buffer,
    // so a binary body (an image, a protobuf payload) passes through unchanged.
    // The method, URL, and header lines are still text.
    let body = string_bytes(body).to_vec();

    // A positive timeout is an overall deadline: ureq's `timeout` covers the
    // connect, the send, and every body read together, which is the right
    // bound for "this request may take up to N ms". Without one, the
    // defaults stand: 5s connect, 10s per read and write.
    let agent = if timeout_ms > 0 {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout(Duration::from_millis(timeout_ms as u64))
            .build()
    } else {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(10))
            .timeout_write(Duration::from_secs(10))
            .build()
    };

    let req = http_apply_headers(agent.request(method, url), headers);

    // The request round-trip and reading the response body block, so run them
    // outside the collector's running set. `http_capture`/`http_store` work
    // entirely in Rust memory and the registry (no GC allocation), so it is
    // sound inside the blocking region.
    // A response to a HEAD request carries no body, whatever Content-Length says.
    let head_request = method.eq_ignore_ascii_case("HEAD");
    crate::gc::blocking(|| {
        // GET and DELETE send no body; POST and PUT send `body`.
        let result = if body.is_empty() {
            req.call()
        } else {
            req.send_bytes(&body)
        };

        match result {
            // A captured `None` means a truncated body; `http_capture` has set
            // the last error, so surface the failure as id 0.
            Ok(resp) => http_capture(resp, head_request)
                .map(http_store)
                .unwrap_or(0),
            // A non-2xx status is a response, not a transport failure.
            Err(ureq::Error::Status(_, resp)) => http_capture(resp, head_request)
                .map(http_store)
                .unwrap_or(0),
            Err(ureq::Error::Transport(t)) => {
                http_set_error(t.to_string());
                0
            }
        }
    })
}

/// Apply `Key: Value` header lines (joined by `\n`) to a request.
fn http_apply_headers(mut req: ureq::Request, headers: &str) -> ureq::Request {
    for line in headers.split('\n') {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            req = req.set(name.trim(), value.trim());
        }
    }
    req
}

/// Status code of the stored response `id`, for example 200 or 404. An
/// unknown id yields 0.
#[no_mangle]
pub extern "C" fn raven_http_status(id: i64) -> i64 {
    http_registry()
        .lock()
        .unwrap()
        .get(&id)
        .map(|r| r.status)
        .unwrap_or(0)
}

/// Reason phrase of the stored response `id`, for example "OK". An
/// unknown id yields an empty `String`.
#[no_mangle]
pub extern "C" fn raven_http_status_text(id: i64) -> *mut object::String {
    // Copy out, then drop the registry lock before allocating the result: the
    // allocation can trigger a stop-the-world collection, and holding the lock
    // across it deadlocks a collector another worker is blocked behind.
    let text: std::string::String = {
        let registry = http_registry().lock().unwrap();
        registry
            .get(&id)
            .map(|r| r.status_text.clone())
            .unwrap_or_default()
    };
    object::raven_string_from_bytes(text.as_ptr(), text.len())
}

/// Body of the stored response `id`. An unknown id yields an empty
/// `String`.
#[no_mangle]
pub extern "C" fn raven_http_body(id: i64) -> *mut object::String {
    // Copy out before allocating; see `raven_http_status_text` for why the lock
    // must not be held across the allocation.
    let body: Vec<u8> = {
        let registry = http_registry().lock().unwrap();
        registry
            .get(&id)
            .map(|r| r.body.clone())
            .unwrap_or_default()
    };
    object::raven_string_from_bytes(body.as_ptr(), body.len())
}

/// A single response header value of `id` by `name` (case-insensitive),
/// or an empty `String` when absent or the id is unknown.
///
/// # Safety
///
/// `name` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_http_header(id: i64, name: *const object::String) -> *mut object::String {
    let wanted = env_name(name).unwrap_or("");
    // Build the owned value, then drop the lock before allocating; see
    // `raven_http_status_text` for why.
    let value: std::string::String = {
        let registry = http_registry().lock().unwrap();
        registry
            .get(&id)
            .and_then(|r| {
                r.headers.lines().find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    if k.trim().eq_ignore_ascii_case(wanted) {
                        Some(v.trim().to_string())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default()
    };
    object::raven_string_from_bytes(value.as_ptr(), value.len())
}

/// All response headers of `id` as `Key: Value` lines joined by `\n`. An
/// unknown id yields an empty `String`.
#[no_mangle]
pub extern "C" fn raven_http_headers(id: i64) -> *mut object::String {
    // Copy out before allocating; see `raven_http_status_text` for why.
    let headers: std::string::String = {
        let registry = http_registry().lock().unwrap();
        registry
            .get(&id)
            .map(|r| r.headers.clone())
            .unwrap_or_default()
    };
    object::raven_string_from_bytes(headers.as_ptr(), headers.len())
}

/// Drop the stored response `id`, releasing its captured data. An unknown
/// id is a no-op.
#[no_mangle]
pub extern "C" fn raven_http_free(id: i64) {
    http_registry().lock().unwrap().remove(&id);
}

/// Open a streaming HTTP request: send it, capture the status and headers,
/// and register the unread body behind a reader that
/// [`raven_http_stream_read`] pulls chunk by chunk. Returns the stream id,
/// or 0 on a transport failure with the last-error slot set. A non-2xx
/// status opens normally; the caller reads the status and decides.
///
/// A positive `timeout_ms` bounds the connect and each body read
/// individually (not the whole stream, which may legitimately stay open for
/// minutes); `timeout_ms <= 0` keeps the defaults (5s connect, 10s reads).
///
/// # Safety
///
/// `method`, `url`, `body`, and `headers` must be valid
/// `raven_string_from_bytes`-built `String`s.
#[no_mangle]
pub extern "C" fn raven_http_stream_open(
    method: *const object::String,
    url: *const object::String,
    body: *const object::String,
    headers: *const object::String,
    timeout_ms: i64,
) -> i64 {
    let (Some(method), Some(url), Some(headers)) =
        (env_name(method), env_name(url), env_name(headers))
    else {
        http_set_error("method, url, or headers is not valid UTF-8".to_string());
        return 0;
    };
    let body = string_bytes(body).to_vec();

    // Per-read timeouts, never an overall one: a stream is long-lived by
    // design, and what must not stall is each individual read.
    let read_write = if timeout_ms > 0 {
        Duration::from_millis(timeout_ms as u64)
    } else {
        Duration::from_secs(10)
    };
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(read_write)
        .timeout_write(read_write)
        .build();

    let req = http_apply_headers(agent.request(method, url), headers);
    let head_request = method.eq_ignore_ascii_case("HEAD");

    // The round-trip blocks; registry inserts touch only Rust memory, so the
    // whole open runs in the blocking region, like `raven_http_request`.
    crate::gc::blocking(|| {
        let result = if body.is_empty() {
            req.call()
        } else {
            req.send_bytes(&body)
        };
        let resp = match result {
            Ok(resp) => resp,
            // A non-2xx status is a response; stream its body like any other.
            Err(ureq::Error::Status(_, resp)) => resp,
            Err(ureq::Error::Transport(t)) => {
                http_set_error(t.to_string());
                return 0;
            }
        };
        let status = resp.status() as i64;
        let status_text = http_status_text(&resp);
        let headers = http_header_lines(&resp);
        // A HEAD response carries no body whatever Content-Length claims, so
        // it gets no reader and reads as an immediately ended stream.
        let reader = if head_request {
            None
        } else {
            Some(resp.into_reader())
        };
        let id = http_next_id();
        http_stream_registry().lock().unwrap().insert(
            id,
            HttpStreamEntry {
                status,
                status_text,
                headers,
                reader,
            },
        );
        http_clear_error();
        id
    })
}

/// Read the next chunk of stream `id`: whatever bytes the next socket read
/// returns, at most 8 KiB. An empty `String` means the stream ended (the
/// reader is dropped, releasing the connection) or, when the last-error
/// slot is set, that the read failed. One read at a time per stream: the
/// reader is taken out of the registry for the read's duration, so a
/// concurrent read on the same id observes end of stream.
#[no_mangle]
pub extern "C" fn raven_http_stream_read(id: i64) -> *mut object::String {
    http_clear_error();
    let taken = {
        let mut registry = http_stream_registry().lock().unwrap();
        registry.get_mut(&id).and_then(|entry| entry.reader.take())
    };
    let Some(mut reader) = taken else {
        // Unknown id, an ended or bodiless stream, or a concurrent read.
        return object::raven_string_from_bytes([].as_ptr(), 0);
    };

    let mut buf = vec![0u8; 8192];
    // The read blocks on the socket; the buffer is Rust memory, so the
    // blocking region is sound. Allocation of the result happens after.
    let result = crate::gc::blocking(|| reader.read(&mut buf));
    match result {
        // End of stream: the reader is dropped; the entry stays so the
        // status and headers remain readable until close.
        Ok(0) => object::raven_string_from_bytes(buf.as_ptr(), 0),
        Ok(n) => {
            // Put the reader back for the next read. A close that raced this
            // read removed the entry; the reader is then dropped here.
            if let Some(entry) = http_stream_registry().lock().unwrap().get_mut(&id) {
                entry.reader = Some(reader);
            }
            object::raven_string_from_bytes(buf.as_ptr(), n)
        }
        Err(e) => {
            http_set_error(e.to_string());
            object::raven_string_from_bytes(buf.as_ptr(), 0)
        }
    }
}

/// Status code of stream `id`, for example 200. An unknown id yields 0.
#[no_mangle]
pub extern "C" fn raven_http_stream_status(id: i64) -> i64 {
    http_stream_registry()
        .lock()
        .unwrap()
        .get(&id)
        .map(|e| e.status)
        .unwrap_or(0)
}

/// Reason phrase of stream `id`. An unknown id yields an empty `String`.
#[no_mangle]
pub extern "C" fn raven_http_stream_status_text(id: i64) -> *mut object::String {
    // Copy out, then drop the lock before allocating; see
    // `raven_http_status_text` for why.
    let text: std::string::String = {
        let registry = http_stream_registry().lock().unwrap();
        registry
            .get(&id)
            .map(|e| e.status_text.clone())
            .unwrap_or_default()
    };
    object::raven_string_from_bytes(text.as_ptr(), text.len())
}

/// All response headers of stream `id` as `Key: Value` lines joined by
/// `\n`. An unknown id yields an empty `String`.
#[no_mangle]
pub extern "C" fn raven_http_stream_headers(id: i64) -> *mut object::String {
    // Copy out, then drop the lock before allocating; see
    // `raven_http_status_text` for why.
    let headers: std::string::String = {
        let registry = http_stream_registry().lock().unwrap();
        registry
            .get(&id)
            .map(|e| e.headers.clone())
            .unwrap_or_default()
    };
    object::raven_string_from_bytes(headers.as_ptr(), headers.len())
}

/// Drop stream `id`, closing its connection. An unknown id is a no-op.
#[no_mangle]
pub extern "C" fn raven_http_stream_close(id: i64) {
    http_stream_registry().lock().unwrap().remove(&id);
}

/// The message of the most recent failed regex compile, empty when it
/// succeeded. The std/regex wrapper reads this to build an `Err` only
/// when the compile id is 0.
#[no_mangle]
pub extern "C" fn raven_regex_last_error() -> *mut object::String {
    REGEX_LAST_ERROR.with(|e| {
        let msg = e.borrow();
        object::raven_string_from_bytes(msg.as_ptr(), msg.len())
    })
}

/// Compile `pattern` (Rust regex, RE2-style: no backreferences or
/// lookaround) and register it. Returns the pattern id, or 0 on a syntax
/// error with the last-error slot set.
///
/// # Safety
///
/// `pattern` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_regex_compile(pattern: *const object::String) -> i64 {
    let Some(pattern) = env_name(pattern) else {
        regex_set_error("pattern is not valid UTF-8".to_string());
        return 0;
    };
    // The byte-oriented engine matches a haystack of arbitrary bytes, so a
    // Raven String that is not valid UTF-8 is matched as-is rather than
    // discarded. The pattern itself is still text.
    match regex::bytes::Regex::new(pattern) {
        Ok(re) => {
            let id = regex_next_id();
            regex_registry().lock().unwrap().insert(id, re);
            regex_clear_error();
            id
        }
        Err(e) => {
            regex_set_error(e.to_string());
            0
        }
    }
}

/// Whether the compiled pattern `id` matches anywhere in `text`. An
/// unknown id yields false.
///
/// # Safety
///
/// `text` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_regex_is_match(id: i64, text: *const object::String) -> bool {
    let text = string_bytes(text);
    let registry = regex_registry().lock().unwrap();
    registry.get(&id).is_some_and(|re| re.is_match(text))
}

/// Whether the compiled pattern `id` has a match in `text`. Lets the
/// wrapper tell "no match" apart from "matched the empty string", which
/// `raven_regex_find` cannot signal by its return value alone.
///
/// # Safety
///
/// `text` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_regex_has_match(id: i64, text: *const object::String) -> bool {
    raven_regex_is_match(id, text)
}

/// The first match of the compiled pattern `id` in `text`, or an empty
/// `String` when there is no match (pair with `raven_regex_has_match` to
/// tell a matched empty string apart from no match).
///
/// # Safety
///
/// `text` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_regex_find(id: i64, text: *const object::String) -> *mut object::String {
    let text = string_bytes(text);
    let value: Vec<u8> = {
        let registry = regex_registry().lock().unwrap();
        registry
            .get(&id)
            .and_then(|re| re.find(text).map(|m| m.as_bytes().to_vec()))
            .unwrap_or_default()
    };
    object::raven_string_from_bytes(value.as_ptr(), value.len())
}

/// All non-overlapping matches of the compiled pattern `id` in `text`,
/// joined by `\n`. An empty result means no matches. A match that itself
/// contains a literal newline is ambiguous under this scheme.
///
/// # Safety
///
/// `text` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_regex_find_all(
    id: i64,
    text: *const object::String,
) -> *mut object::String {
    let text = string_bytes(text);
    let value: Vec<u8> = {
        let registry = regex_registry().lock().unwrap();
        match registry.get(&id) {
            Some(re) => encode_bytes_list(re.find_iter(text).map(|m| m.as_bytes())),
            None => Vec::new(),
        }
    };
    object::raven_string_from_bytes(value.as_ptr(), value.len())
}

/// Encode a list of byte strings for transport to Raven as one `String`: each
/// element is its byte length in decimal, then a `:`, then its raw bytes. This
/// is unambiguous even when an element contains a newline or a colon, which the
/// previous newline join was not, and works for arbitrary (non-UTF-8) bytes.
/// `std/regex` decodes it.
fn encode_bytes_list<'a>(elems: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    for e in elems {
        out.extend_from_slice(e.len().to_string().as_bytes());
        out.push(b':');
        out.extend_from_slice(e);
    }
    out
}

/// The capture groups of the first match of the compiled pattern `id` in
/// `text`, joined by `\n`: group 0 (the whole match) first, then groups
/// 1, 2, and so on. An unmatched optional group becomes an empty line. An
/// empty result means no match (pair with `raven_regex_has_match`).
///
/// # Safety
///
/// `text` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_regex_captures(
    id: i64,
    text: *const object::String,
) -> *mut object::String {
    let text = string_bytes(text);
    let value: Vec<u8> = {
        let registry = regex_registry().lock().unwrap();
        registry
            .get(&id)
            .and_then(|re| {
                re.captures(text).map(|caps| {
                    encode_bytes_list(caps.iter().map(|g| g.map(|m| m.as_bytes()).unwrap_or(b"")))
                })
            })
            .unwrap_or_default()
    };
    object::raven_string_from_bytes(value.as_ptr(), value.len())
}

/// Replace every match of the compiled pattern `id` in `text` with
/// `repl`. The regex crate honors `$name`, `$1`, and `${1}` group
/// references in `repl`. An unknown id returns `text` unchanged.
///
/// # Safety
///
/// `text` and `repl` must be valid `raven_string_from_bytes`-built
/// `String`s.
#[no_mangle]
pub extern "C" fn raven_regex_replace_all(
    id: i64,
    text: *const object::String,
    repl: *const object::String,
) -> *mut object::String {
    let text = string_bytes(text);
    let repl = string_bytes(repl);
    let value: Vec<u8> = {
        let registry = regex_registry().lock().unwrap();
        match registry.get(&id) {
            Some(re) => re.replace_all(text, repl).into_owned(),
            None => text.to_vec(),
        }
    };
    object::raven_string_from_bytes(value.as_ptr(), value.len())
}

/// Split `text` on the compiled pattern `id`, joining the pieces by `\n`.
/// An unknown id returns `text` unchanged.
///
/// # Safety
///
/// `text` must be a valid `raven_string_from_bytes`-built `String`.
#[no_mangle]
pub extern "C" fn raven_regex_split(id: i64, text: *const object::String) -> *mut object::String {
    let text = string_bytes(text);
    let value: Vec<u8> = {
        let registry = regex_registry().lock().unwrap();
        match registry.get(&id) {
            Some(re) => encode_bytes_list(re.split(text)),
            // Unknown id: the text unsplit, as a single encoded element.
            None => encode_bytes_list(std::iter::once(text)),
        }
    };
    object::raven_string_from_bytes(value.as_ptr(), value.len())
}

/// Drop the compiled pattern `id` from the registry. An unknown id is a
/// no-op.
#[no_mangle]
pub extern "C" fn raven_regex_free(id: i64) {
    regex_registry().lock().unwrap().remove(&id);
}

/// The message of the most recent failed subprocess spawn, empty when the
/// spawn succeeded. The std/process wrapper reads this to build an `Err`
/// only when the run id is 0. A non-zero child exit never sets it.
#[no_mangle]
pub extern "C" fn raven_process_last_error() -> *mut object::String {
    PROCESS_LAST_ERROR.with(|e| {
        let msg = e.borrow();
        object::raven_string_from_bytes(msg.as_ptr(), msg.len())
    })
}

/// Spawn `program` with `args_nul_joined` (the child's args, each joined by
/// a single NUL byte; an empty String means no args), feed `stdin_data` to
/// the child's stdin (an empty String writes nothing), wait for it, and
/// capture stdout, stderr (lossy UTF-8), and the exit code into a registry
/// entry. Returns the entry id, or 0 on a spawn failure (for example the
/// program is not found) with the last-error slot set. A child that runs
/// but exits non-zero is NOT a spawn failure: its code and output are
/// captured and a normal id is returned.
///
/// # Safety
///
/// `program`, `args_nul_joined`, and `stdin_data` must be valid
/// `raven_string_from_bytes`-built `String`s.
#[no_mangle]
pub extern "C" fn raven_process_run(
    program: *const object::String,
    args_nul_joined: *const object::String,
    stdin_data: *const object::String,
) -> i64 {
    // A child that exits before reading all of its stdin closes the pipe, so
    // writing the rest would raise SIGPIPE and kill this program. Ignore it so
    // the write returns a (handled) error instead.
    ignore_sigpipe();
    let (Some(program), Some(args_joined)) = (env_name(program), env_name(args_nul_joined)) else {
        process_set_error("program or args is not valid UTF-8".to_string());
        return 0;
    };
    // stdin is fed as raw bytes: a Raven String is a byte buffer, so binary
    // input passes through unchanged. Only the program and its args, which are
    // OS strings, need to be valid UTF-8.
    let stdin_bytes = {
        let ptr = object::raven_string_bytes(stdin_data);
        let len = object::raven_string_len(stdin_data) as usize;
        if ptr.is_null() || len == 0 {
            Vec::new()
        } else {
            // SAFETY: a Raven String holds `len` initialized bytes.
            unsafe { slice::from_raw_parts(ptr, len).to_vec() }
        }
    };

    // Each arg is NUL-prefixed. An empty String is zero args; otherwise the
    // split has a leading empty piece (before the first NUL) to drop, and each
    // field after it is one arg. NUL-prefixing rather than NUL-separating keeps
    // an empty arg list and a single empty arg distinct. Program args
    // effectively never contain NUL, so this round-trips unambiguously.
    let args: Vec<&str> = if args_joined.is_empty() {
        Vec::new()
    } else {
        args_joined.split('\0').skip(1).collect()
    };

    let mut command = process::Command::new(program);
    command.args(&args);
    command.stdin(process::Stdio::piped());
    command.stdout(process::Stdio::piped());
    command.stderr(process::Stdio::piped());

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            process_set_error(e.to_string());
            return 0;
        }
    };

    // Feed stdin on a separate thread while the main thread drains stdout and
    // stderr. Writing all of stdin before reading the output deadlocks when the
    // child fills its output pipe buffer faster than we consume it: the child
    // blocks writing stdout and we block writing stdin.
    let stdin_pipe = child.stdin.take();
    let writer = std::thread::spawn(move || {
        if let Some(mut pipe) = stdin_pipe {
            // A broken pipe is fine: a child that exits without reading still
            // produced a valid result. Dropping the pipe closes stdin (EOF).
            let _ = pipe.write_all(&stdin_bytes);
        }
    });

    // Waiting for the child to exit blocks for its whole lifetime; run it
    // outside the collector's running set.
    let output = match crate::gc::blocking(|| child.wait_with_output()) {
        Ok(o) => o,
        Err(e) => {
            let _ = writer.join();
            process_set_error(e.to_string());
            return 0;
        }
    };
    let _ = writer.join();

    // A child terminated by a signal with no exit code maps to -1.
    let code = output.status.code().map(|c| c as i64).unwrap_or(-1);
    let result = ProcResult {
        code,
        stdout: output.stdout,
        stderr: output.stderr,
    };
    let id = process_next_id();
    process_registry().lock().unwrap().insert(id, result);
    process_clear_error();
    id
}

/// Exit code of the finished child `id`. A child terminated by a signal
/// with no code yields -1. An unknown id yields -1.
#[no_mangle]
pub extern "C" fn raven_process_exit_code(id: i64) -> i64 {
    process_registry()
        .lock()
        .unwrap()
        .get(&id)
        .map(|r| r.code)
        .unwrap_or(-1)
}

/// Captured stdout of the finished child `id`. An unknown id yields an
/// empty `String`.
#[no_mangle]
pub extern "C" fn raven_process_stdout(id: i64) -> *mut object::String {
    // Copy out before allocating; see `raven_http_status_text` for why the lock
    // must not be held across the allocation.
    let out: Vec<u8> = {
        let registry = process_registry().lock().unwrap();
        registry
            .get(&id)
            .map(|r| r.stdout.clone())
            .unwrap_or_default()
    };
    object::raven_string_from_bytes(out.as_ptr(), out.len())
}

/// Captured stderr of the finished child `id`. An unknown id yields an
/// empty `String`.
#[no_mangle]
pub extern "C" fn raven_process_stderr(id: i64) -> *mut object::String {
    // Copy out before allocating; see `raven_http_status_text` for why.
    let err: Vec<u8> = {
        let registry = process_registry().lock().unwrap();
        registry
            .get(&id)
            .map(|r| r.stderr.clone())
            .unwrap_or_default()
    };
    object::raven_string_from_bytes(err.as_ptr(), err.len())
}

/// Drop the finished child `id`, releasing its captured output. An unknown
/// id is a no-op.
#[no_mangle]
pub extern "C" fn raven_process_free(id: i64) {
    process_registry().lock().unwrap().remove(&id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::mem::{align_of, size_of};

    #[test]
    fn object_header_is_sixteen_bytes() {
        assert_eq!(size_of::<ObjectHeader>(), 16);
    }

    #[test]
    fn random_entropy_never_repeats_within_a_process() {
        // The call counter must make successive seeds distinct even when
        // many calls land in the same clock tick, so a fresh-per-call
        // entropy seed (for example UUID v4) cannot collide.
        let n = 100_000;
        let mut seen = HashSet::with_capacity(n);
        for _ in 0..n {
            assert!(seen.insert(raven_random_entropy()), "entropy seed repeated");
        }
    }

    #[test]
    fn object_header_alignment_divides_object_align() {
        assert!(OBJECT_ALIGN.is_power_of_two());
        assert_eq!(OBJECT_ALIGN % align_of::<ObjectHeader>(), 0);
    }

    // Regressions for #655 and #656: `raven_fs_list` joins names by a NUL byte
    // using their raw bytes, so a filename with a newline is not split and a
    // non-UTF-8 Unix name is preserved. Both names are Unix-only (a Windows
    // filename cannot contain a newline or be non-Unicode).
    #[cfg(unix)]
    #[test]
    fn list_dir_preserves_newline_and_non_utf8_names() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = std::env::temp_dir().join(format!("raven_listdir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(OsStr::from_bytes(b"a\nb")), b"1").unwrap();
        std::fs::write(dir.join(OsStr::from_bytes(b"f\xff")), b"2").unwrap();

        let path = dir.to_str().unwrap().as_bytes();
        let path_str = object::raven_string_from_bytes(path.as_ptr(), path.len());
        let result = raven_fs_list(path_str);
        let names: Vec<&[u8]> = string_bytes(result).split(|b| *b == 0).collect();

        assert_eq!(names.len(), 2, "two entries, neither split on the newline");
        assert!(
            names.contains(&&b"a\nb"[..]),
            "the newline name is preserved whole"
        );
        assert!(
            names.contains(&&b"f\xff"[..]),
            "the non-UTF-8 name is preserved"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn object_header_new_zeroes_gc_bits() {
        let h = ObjectHeader::new(TAG_STRING, 5, 8);
        assert_eq!(h.tag, TAG_STRING);
        assert_eq!(h.gc_bits, 0);
        assert_eq!(h.len, 5);
        assert_eq!(h.cap, 8);
    }

    #[test]
    fn tag_constants_are_distinct_and_dense() {
        let tags = [
            TAG_STRING,
            TAG_LIST,
            TAG_MAP,
            TAG_SET,
            TAG_CLOSURE,
            TAG_BOX,
            TAG_STRUCT,
        ];
        for (i, t) in tags.iter().enumerate() {
            assert_eq!(*t as usize, i + 1, "tag {i} should be {}", i + 1);
        }
    }

    #[test]
    fn alloc_dealloc_roundtrip_succeeds() {
        let size = 64;
        let align = OBJECT_ALIGN;
        let ptr = raven_alloc(size, align);
        assert!(!ptr.is_null(), "raven_alloc returned null");
        // Touch the memory so a miscompile or layout bug would show up
        // under sanitizers.
        unsafe {
            std::ptr::write_bytes(ptr, 0xAB, size);
        }
        raven_dealloc(ptr, size, align);
    }

    #[test]
    fn alloc_with_invalid_layout_returns_null() {
        // align is not a power of two: invalid layout, must not abort.
        let ptr = raven_alloc(8, 3);
        assert!(ptr.is_null());
    }

    #[test]
    fn dealloc_null_is_noop() {
        raven_dealloc(std::ptr::null_mut(), 16, OBJECT_ALIGN);
    }

    #[test]
    fn print_str_accepts_empty_slice() {
        // Empty slices must not dereference the pointer.
        raven_print_str(std::ptr::null(), 0);
        raven_println_str(std::ptr::null(), 0);
    }

    fn rv_string(s: &str) -> *mut object::String {
        object::raven_string_from_bytes(s.as_ptr(), s.len())
    }

    #[test]
    fn parked_accept_does_not_hold_the_registry_lock() {
        // A connection that has not yet arrived must not serialize other
        // goroutines' net operations: accept clones its handle under the
        // registry lock, then blocks without it (issue #377). With the lock
        // held across the syscall, a worker reading another stream could not
        // even acquire the registry while accept was parked.
        let lid = raven_net_listen(rv_string("127.0.0.1:0"));
        assert!(lid > 0, "listen failed");
        let port = {
            let reg = net_registry().lock().unwrap();
            match reg.get(&lid) {
                Some(Socket::Listener(l)) => l.local_addr().unwrap().port(),
                _ => panic!("listener not registered"),
            }
        };
        let handle = std::thread::spawn(move || raven_net_accept(lid));
        // Let the accept reach the blocking syscall (no client yet).
        std::thread::sleep(std::time::Duration::from_millis(200));
        let lock_free = net_registry().try_lock().is_ok();
        // Unblock the accept with a client so the thread finishes, no leak.
        let _client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _ = handle.join();
        assert!(lock_free, "registry lock held while accept was parked");
    }
}
