//! Heap layout of Raven struct and enum values for the back end.
//!
//! Both struct and enum values are laid out as a runtime "struct value":
//! the standard 16-byte object header followed by a sequence of uniform
//! 8-byte field slots. A struct's slots are its fields in declaration
//! order. An enum's slot 0 holds the variant discriminant and slots
//! 1.. hold the active variant's payload. Keeping a single uniform slot
//! width means the back end never computes per-field offsets from
//! alignment, and the collector traces a value through a per-type GC
//! pointer bitmask the back end registers at startup.
//!
//! See `docs/v2/specs/object-layout.md` and `docs/v2/specs/codegen.md`.

use crate::mir::MirType;

/// Byte offset from the start of a struct or enum value to its first
/// field slot. Mirrors `raven-runtime`'s `STRUCT_FIELDS_OFFSET`.
pub const FIELDS_OFFSET: i32 = 16;

/// Width in bytes of one field slot. Mirrors `raven-runtime`'s
/// `STRUCT_FIELD_SLOT`.
pub const FIELD_SLOT: i32 = 8;

/// Byte offset of field slot `index` (zero based) from the start of the
/// value (the object header). Used when addressing from the object base.
pub fn field_offset(index: usize) -> i32 {
    FIELDS_OFFSET + (index as i32) * FIELD_SLOT
}

/// Byte offset of field slot `index` (zero based) relative to the field
/// base pointer returned by `raven_struct_fields`, which already points
/// past the header. This is what the load and store paths use, since
/// they address from the field base, not the object base.
pub fn slot_offset(index: usize) -> i32 {
    (index as i32) * FIELD_SLOT
}

/// Whether a value of `ty` is a GC pointer the collector must trace.
///
/// Every heap value (`Str`, `Struct`, `Enum`, `Option`, `Result`,
/// `List`, and a closure `Function`) is a single traced pointer. The
/// scalars (`Int`, `Float`, `Bool`, `Char`) and `Unit` are not.
pub fn is_gc_pointer(ty: &MirType) -> bool {
    matches!(
        ty,
        MirType::Str
            | MirType::Struct { .. }
            | MirType::Enum { .. }
            | MirType::Option(_)
            | MirType::Result(_, _)
            | MirType::List(_)
            | MirType::Function { .. }
    )
}

/// Build the GC pointer bitmask for a struct's fields in declaration
/// order: bit `i` is set when field slot `i` holds a GC pointer.
pub fn struct_pointer_mask(field_tys: &[MirType]) -> u64 {
    let mut mask = 0u64;
    for (i, ty) in field_tys.iter().enumerate() {
        if i < 64 && is_gc_pointer(ty) {
            mask |= 1u64 << i;
        }
    }
    mask
}

/// Build the GC pointer bitmask for an enum value. Slot 0 is the scalar
/// discriminant, so payload field `i` lands in slot `i + 1` and sets
/// bit `i + 1` when it is a GC pointer.
pub fn enum_pointer_mask(payload_tys: &[MirType]) -> u64 {
    let mut mask = 0u64;
    for (i, ty) in payload_tys.iter().enumerate() {
        let slot = i + 1;
        if slot < 64 && is_gc_pointer(ty) {
            mask |= 1u64 << slot;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_step_by_slot_width() {
        assert_eq!(field_offset(0), 16);
        assert_eq!(field_offset(1), 24);
        assert_eq!(field_offset(3), 40);
    }

    #[test]
    fn scalars_are_not_pointers() {
        assert!(!is_gc_pointer(&MirType::Int));
        assert!(!is_gc_pointer(&MirType::Float));
        assert!(!is_gc_pointer(&MirType::Bool));
        assert!(!is_gc_pointer(&MirType::Char));
        assert!(!is_gc_pointer(&MirType::Unit));
    }

    #[test]
    fn heap_values_are_pointers() {
        assert!(is_gc_pointer(&MirType::Str));
        assert!(is_gc_pointer(&MirType::List(Box::new(MirType::Int))));
        assert!(is_gc_pointer(&MirType::Option(Box::new(MirType::Int))));
    }

    #[test]
    fn struct_mask_marks_pointer_fields() {
        // Int, Str, Int -> only slot 1 is a pointer.
        let tys = vec![MirType::Int, MirType::Str, MirType::Int];
        assert_eq!(struct_pointer_mask(&tys), 0b010);
    }

    #[test]
    fn enum_mask_shifts_past_discriminant() {
        // Payload [Str] -> slot 1 (bit 1) is a pointer.
        assert_eq!(enum_pointer_mask(&[MirType::Str]), 0b010);
        // Payload [Int] -> no pointer slots.
        assert_eq!(enum_pointer_mask(&[MirType::Int]), 0);
    }
}
