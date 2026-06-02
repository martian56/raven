//! Runtime type metadata and the `Any` boxing primitives.
//!
//! Compile-time reflection renders type names and field-name lists from
//! the compiler's static type info. Runtime reflection needs the same
//! information reachable from a value of unknown static type, so the
//! back-end registers a per-type metadata record at program startup and
//! the `Any` box carries the value's runtime type id. The metadata maps a
//! type id to its rendered name, its struct flag, and (for a struct) its
//! field names, per-field type ids, and per-field GC-pointer flags. See
//! `docs/v2/specs/runtime-reflection.md`.
//!
//! `Any` reuses the `Box` object (`TAG_BOX`): the payload holds the value
//! (a pointer for heap types, an inline scalar otherwise) and the box's
//! `type_id` word holds the value's runtime type id. The GC already traces
//! a box payload when its `payload_is_gc_ptr` flag is set, so an `Any`
//! that boxes the only reference to a heap value keeps it alive.

use crate::object::{
    raven_box_new, raven_box_payload, raven_list_new, raven_list_push, raven_string_from_bytes,
    raven_struct_fields, Box, List, ObjectHeader, String as RavenString,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::c_char;

/// One field's reflection record.
struct FieldMeta {
    name: *const c_char,
    type_id: u32,
    is_gc_ptr: bool,
}

/// Static metadata for one monomorphic type, registered once by the
/// program entry shim. Pointers come from the program's read-only data, so
/// they outlive the program and need no ownership tracking here.
struct TypeMeta {
    name: *const c_char,
    is_struct: bool,
    fields: Vec<FieldMeta>,
}

// The metadata pointers are only read, never mutated, so the table is sound
// to hold across the single-threaded v2 runtime.
unsafe impl Send for TypeMeta {}

thread_local! {
    /// Type id to metadata. Populated by `raven_type_register` at startup,
    /// before any reflection call. A lookup of an unregistered id yields
    /// empty defaults, keeping a reflection call on a type the back-end did
    /// not register total rather than crashing.
    static TYPE_META: RefCell<HashMap<u32, TypeMeta>> = RefCell::new(HashMap::new());
}

/// Register a type's reflection metadata.
///
/// `type_id` is the same small integer the back-end assigns each
/// monomorphic type for the GC descriptor. `name` is a NUL-terminated C
/// string. `is_struct` is nonzero for a struct. `field_count` names how
/// many entries the three field arrays hold: `field_names[i]` is a
/// NUL-terminated C string, `field_type_ids[i]` is field `i`'s registered
/// type id, and `field_is_gc_ptr[i]` is nonzero when field `i` holds a GC
/// pointer. All pointers must outlive the program (read-only data the
/// back-end emits).
///
/// # Safety
///
/// `name` must be a valid NUL-terminated pointer, and each field array
/// must point to `field_count` valid entries living for the program.
#[no_mangle]
pub unsafe extern "C" fn raven_type_register(
    type_id: u32,
    name: *const c_char,
    is_struct: u32,
    field_count: u32,
    field_names: *const *const c_char,
    field_type_ids: *const u32,
    field_is_gc_ptr: *const u32,
) {
    let mut fields = Vec::with_capacity(field_count as usize);
    if !field_names.is_null() && !field_type_ids.is_null() && !field_is_gc_ptr.is_null() {
        for i in 0..field_count as usize {
            // SAFETY: caller guarantees `field_count` valid entries.
            let (n, tid, gc) = unsafe {
                (
                    *field_names.add(i),
                    *field_type_ids.add(i),
                    *field_is_gc_ptr.add(i),
                )
            };
            fields.push(FieldMeta {
                name: n,
                type_id: tid,
                is_gc_ptr: gc != 0,
            });
        }
    }
    let meta = TypeMeta {
        name,
        is_struct: is_struct != 0,
        fields,
    };
    TYPE_META.with(|m| {
        m.borrow_mut().insert(type_id, meta);
    });
}

/// Run `f` with the metadata of `type_id`, or return `default` when the id
/// was never registered.
fn with_meta<R>(type_id: u32, default: R, f: impl FnOnce(&TypeMeta) -> R) -> R {
    TYPE_META.with(|m| match m.borrow().get(&type_id) {
        Some(meta) => f(meta),
        None => default,
    })
}

/// Build a fresh Raven `String` from a NUL-terminated C string, or null
/// when `s` is null.
fn raven_string_from_cstr(s: *const c_char) -> *mut RavenString {
    if s.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `s` is a valid NUL-terminated string from read-only data.
    let bytes = unsafe { std::ffi::CStr::from_ptr(s) }.to_bytes();
    raven_string_from_bytes(bytes.as_ptr(), bytes.len())
}

/// Box `value` into a fresh `Any`, tagging it with `type_id`.
///
/// The eight-byte `value` is the value's machine word: a GC pointer for a
/// heap type, or an inline scalar (sign- or zero-extended to 64 bits) for
/// an immediate. `is_gc_ptr` is nonzero when `value` is a GC pointer the
/// collector must trace through the `Any`; that flag makes the box keep a
/// heap payload alive. Returns null on allocation failure.
#[no_mangle]
pub extern "C" fn raven_any_new(value: u64, type_id: u32, is_gc_ptr: u32) -> *mut Box {
    let b = raven_box_new(8, 8, is_gc_ptr);
    if b.is_null() {
        return b;
    }
    // SAFETY: `raven_box_new` returned an 8-byte payload box; store the
    // value word and the runtime type id in the box's `type_id` slot.
    unsafe {
        (*b).type_id = type_id;
        let payload = raven_box_payload(b) as *mut u64;
        payload.write(value);
    }
    b
}

/// Return the runtime type id stored in an `Any`, or `u32::MAX` when `a`
/// is null.
#[no_mangle]
pub extern "C" fn raven_any_type_id(a: *const Box) -> u32 {
    if a.is_null() {
        return u32::MAX;
    }
    // SAFETY: `a` is a live `Any` box.
    unsafe { (*a).type_id }
}

/// Return the eight-byte payload word stored in an `Any` (a GC pointer or
/// an inline scalar), or zero when `a` is null.
#[no_mangle]
pub extern "C" fn raven_any_payload(a: *const Box) -> u64 {
    if a.is_null() {
        return 0;
    }
    // SAFETY: `a` is a live `Any` box with an eight-byte payload.
    unsafe { (raven_box_payload(a as *mut Box) as *const u64).read() }
}

/// Return the runtime type name of the value in `a` as a fresh Raven
/// `String`, or null when the type was not registered.
#[no_mangle]
pub extern "C" fn raven_any_type_name(a: *const Box) -> *mut RavenString {
    let type_id = raven_any_type_id(a);
    let name = with_meta(type_id, std::ptr::null(), |m| m.name);
    raven_string_from_cstr(name)
}

/// Return the struct field names of the value in `a` as a fresh Raven
/// `List<String>`. The list is empty when the value is not a struct or the
/// type was not registered.
#[no_mangle]
pub extern "C" fn raven_any_field_names(a: *const Box) -> *mut List {
    let type_id = raven_any_type_id(a);
    let names: Vec<*const c_char> = with_meta(type_id, Vec::new(), |m| {
        if m.is_struct {
            m.fields.iter().map(|f| f.name).collect()
        } else {
            Vec::new()
        }
    });
    let list = raven_list_new(8, 8, names.len() as u32, 1);
    if list.is_null() {
        return list;
    }
    for n in names {
        let s = raven_string_from_cstr(n);
        let slot = s as u64;
        // push copies one eight-byte slot (the String pointer).
        raven_list_push(list, &slot as *const u64 as *const u8);
    }
    list
}

/// Read the field named by the Raven `String` `name` from the struct in
/// `a`, box it into a fresh `Any`, and return it. Returns null when `a` is
/// not a registered struct or has no such field.
#[no_mangle]
pub extern "C" fn raven_any_get_field(a: *const Box, name: *const RavenString) -> *mut Box {
    if a.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `name` is a live Raven String; read its bytes for the lookup.
    let wanted = unsafe {
        let len = (*name).header.len as usize;
        let bytes = (*name).bytes;
        std::slice::from_raw_parts(bytes, len)
    };
    let type_id = raven_any_type_id(a);
    let found = with_meta(type_id, None, |m| {
        if !m.is_struct {
            return None;
        }
        m.fields.iter().enumerate().find_map(|(i, f)| {
            if f.name.is_null() {
                return None;
            }
            // SAFETY: `f.name` is a NUL-terminated read-only C string.
            let fname = unsafe { std::ffi::CStr::from_ptr(f.name) }.to_bytes();
            if fname == wanted {
                Some((i, f.type_id, f.is_gc_ptr))
            } else {
                None
            }
        })
    });
    let (index, field_type_id, is_gc_ptr) = match found {
        Some(t) => t,
        None => return std::ptr::null_mut(),
    };
    // The struct pointer is the `Any` payload word.
    let struct_ptr = raven_any_payload(a) as *const ObjectHeader;
    if struct_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let fields = raven_struct_fields(struct_ptr);
    if fields.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: field slots are eight bytes each in declaration order.
    let value = unsafe { (fields.add(index * 8) as *const u64).read() };
    raven_any_new(value, field_type_id, is_gc_ptr as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn register_point() -> (CString, CString, CString) {
        let name = CString::new("Point").unwrap();
        let fx = CString::new("x").unwrap();
        let fy = CString::new("y").unwrap();
        let field_names = [fx.as_ptr(), fy.as_ptr()];
        let field_ids = [7u32, 7u32];
        let field_gc = [0u32, 0u32];
        // SAFETY: pointers live for the duration of the call.
        unsafe {
            raven_type_register(
                3,
                name.as_ptr(),
                1,
                2,
                field_names.as_ptr(),
                field_ids.as_ptr(),
                field_gc.as_ptr(),
            );
        }
        (name, fx, fy)
    }

    #[test]
    fn any_roundtrips_tag_and_payload() {
        std::thread::spawn(|| {
            let a = raven_any_new(42, 5, 0);
            assert!(!a.is_null());
            assert_eq!(raven_any_type_id(a), 5);
            assert_eq!(raven_any_payload(a), 42);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn type_name_and_field_names_from_any() {
        std::thread::spawn(|| {
            let _keep = register_point();
            let a = raven_any_new(0, 3, 0);
            let s = raven_any_type_name(a);
            // SAFETY: a non-null Raven String with `len` bytes.
            unsafe {
                let bytes = std::slice::from_raw_parts((*s).bytes, (*s).header.len as usize);
                assert_eq!(bytes, b"Point");
            }
            let list = raven_any_field_names(a);
            // SAFETY: a List<String> with two String-pointer slots.
            unsafe {
                assert_eq!((*list).header.len, 2);
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn get_field_reads_a_scalar_field() {
        std::thread::spawn(|| {
            let _keep = register_point();
            // Build a Point struct value with x = 11, y = 22.
            use crate::gc::raven_struct_register;
            use crate::object::raven_struct_new;
            raven_struct_register(3, 0);
            let s = raven_struct_new(2, 3);
            let fields = raven_struct_fields(s) as *mut u64;
            // SAFETY: two writable slots.
            unsafe {
                fields.add(0).write(11);
                fields.add(1).write(22);
            }
            let a = raven_any_new(s as u64, 3, 1);
            let xname = CString::new("x").unwrap();
            let xstr = raven_string_from_bytes(xname.as_bytes().as_ptr(), xname.as_bytes().len());
            let fx = raven_any_get_field(a, xstr);
            assert!(!fx.is_null());
            assert_eq!(raven_any_payload(fx), 11);
            assert_eq!(raven_any_type_id(fx), 7);

            let yname = CString::new("y").unwrap();
            let ystr = raven_string_from_bytes(yname.as_bytes().as_ptr(), yname.as_bytes().len());
            let fy = raven_any_get_field(a, ystr);
            assert_eq!(raven_any_payload(fy), 22);

            let zname = CString::new("z").unwrap();
            let zstr = raven_string_from_bytes(zname.as_bytes().as_ptr(), zname.as_bytes().len());
            assert!(raven_any_get_field(a, zstr).is_null());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn any_keeps_a_heap_struct_payload_alive_across_collection() {
        std::thread::spawn(|| {
            use crate::gc::{
                raven_gc_collect, raven_gc_enter_frame, raven_gc_leave_frame,
                raven_gc_live_objects, raven_struct_register,
            };
            use crate::object::{raven_string_new, raven_struct_new};
            let _keep = register_point();
            // Point has a scalar slot 0 and (pretend) a GC pointer slot 1.
            raven_struct_register(3, 0b10);
            let name = raven_string_new(3);
            let s = raven_struct_new(2, 3);
            // SAFETY: slot 0 scalar, slot 1 the String pointer.
            unsafe {
                let fields = raven_struct_fields(s) as *mut u64;
                fields.add(0).write(99);
                fields.add(1).write(name as u64);
            }
            // Box the struct into an Any with the GC-pointer flag set, then
            // root only the Any. The struct and string are reachable solely
            // through the Any now.
            let a = raven_any_new(s as u64, 3, 1);
            // Root the Any through the frame ABI codegen emits: the local
            // holding the Any lives in `root`, and the registered array entry
            // is the *address* of that slot.
            let mut root: *mut u8 = a as *mut u8;
            let mut frame: [*mut *mut u8; 1] = [&mut root as *mut *mut u8];
            raven_gc_enter_frame(frame.as_mut_ptr() as *mut *mut u8, 1);
            // Any box + struct + string = 3 live objects.
            assert_eq!(raven_gc_live_objects(), 3);
            raven_gc_collect();
            // All three survive because the Any keeps its payload alive and
            // the struct's field mask keeps tracing its String field.
            assert_eq!(raven_gc_live_objects(), 3);
            // SAFETY: read both fields back through the surviving struct; the
            // scalar is intact and the String pointer still points at the
            // surviving string.
            unsafe {
                let fields = raven_struct_fields(s) as *const u64;
                assert_eq!(fields.add(0).read(), 99);
                assert_eq!(fields.add(1).read(), name as u64);
                assert_eq!((*name).header.tag, crate::object::TAG_STRING);
            }
            raven_gc_leave_frame();
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn unregistered_and_null_are_total() {
        std::thread::spawn(|| {
            assert_eq!(raven_any_type_id(std::ptr::null()), u32::MAX);
            assert_eq!(raven_any_payload(std::ptr::null()), 0);
            let a = raven_any_new(0, 999, 0);
            assert!(raven_any_type_name(a).is_null());
            let list = raven_any_field_names(a);
            // SAFETY: an empty List.
            unsafe {
                assert_eq!((*list).header.len, 0);
            }
        })
        .join()
        .unwrap();
    }
}
