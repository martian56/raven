//! Mangled names for MIR-level runtime intrinsics.
//!
//! Some sugar (string interpolation today) desugars during HIR-to-MIR
//! lowering into ordinary `Call` rvalues whose callee mangled name is
//! one of the constants below. The code generator recognizes these
//! names and rewrites each call into a direct call to the matching
//! `raven-runtime` C-ABI symbol. Keeping the names in one module lets
//! the lowering and the back-end agree without a circular dependency.

/// Concatenate two heap `String` values into a new heap `String`.
/// Lowers to `raven_string_concat`.
pub const STR_CONCAT: &str = "__raven_str_concat";

/// Render an `Int` as a heap `String`. Lowers to `raven_int_to_string`.
pub const INT_TO_STRING: &str = "__raven_int_to_string";

/// Render a `Bool` as a heap `String`. Lowers to
/// `raven_bool_to_string`.
pub const BOOL_TO_STRING: &str = "__raven_bool_to_string";

/// Render a `Float` as a heap `String`. Lowers to
/// `raven_float_to_string`.
pub const FLOAT_TO_STRING: &str = "__raven_float_to_string";

/// Render a `Char` as a heap `String`. Lowers to
/// `raven_char_to_string`.
pub const CHAR_TO_STRING: &str = "__raven_char_to_string";
