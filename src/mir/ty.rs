//! Concretized type representation for MIR.
//!
//! `MirType` is a strict subset of [`tycheck::Ty`](crate::tycheck::Ty)
//! that drops the generic-parameter and inference-variable variants.
//! Monomorphization rewrites every `Ty::Param` into its concrete
//! substitute before any MIR is built, so MIR types are always ground.

use crate::resolve::DeclId;
use crate::tycheck::{FfiTy, Ty};
use std::fmt;

/// Ground C FFI primitive type carried through MIR. Mirrors
/// [`FfiTy`](crate::tycheck::FfiTy) without the inference-only cases;
/// the codegen back end maps each to a concrete Cranelift ABI type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirFfiTy {
    /// C `int`. Codegen maps it to `i32`.
    CInt,
    /// C `long`. Codegen maps it to `i64`.
    CLong,
    /// C `size_t`. Codegen maps it to a pointer-width int.
    CSize,
    /// `*const c_char`. Codegen maps it to a pointer-width int.
    CStr,
    /// C `float`. Codegen maps it to `f32`.
    CFloat,
    /// C `double`. Codegen maps it to `f64`.
    CDouble,
    /// `CPtr<T>`, an opaque pointer. Codegen maps it to a pointer-width
    /// int. The pointee mangle is kept for symbol uniqueness only.
    CPtr(Box<MirType>),
}

/// Ground type used inside MIR.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirType {
    Unit,
    Bool,
    Int,
    Float,
    Char,
    Str,
    Struct {
        id: DeclId,
        name: String,
        args: Vec<MirType>,
    },
    Enum {
        id: DeclId,
        name: String,
        args: Vec<MirType>,
    },
    Option(Box<MirType>),
    Result(Box<MirType>, Box<MirType>),
    List(Box<MirType>),
    Function {
        params: Vec<MirType>,
        ret: Box<MirType>,
    },
    /// A `dyn Trait` trait object. Lowered as a single GC pointer to a
    /// boxed two-slot fat pointer `{ data, vtable }`. `name` is the
    /// trait's short name and `methods` is the trait's method order, used
    /// by the back end to lay out vtables and pick a dispatch slot.
    Dyn {
        name: String,
        methods: Vec<String>,
    },
    /// A C FFI primitive type. Carried so the back end can pick the
    /// matching Cranelift ABI type for an `extern "C"` parameter,
    /// return, or `c"..."` literal.
    Ffi(MirFfiTy),
}

impl MirType {
    /// Build a [`MirType`] from a fully concretized [`Ty`].
    ///
    /// `Ty::Param` and `Ty::Var` panic here: callers must apply the
    /// monomorphization substitution before lowering each function.
    /// `Ty::Error` is mapped to [`MirType::Unit`] so that partially
    /// invalid programs do not abort the whole pipeline.
    pub fn from_ty(ty: &Ty) -> Self {
        match ty {
            Ty::Unit => MirType::Unit,
            Ty::Bool => MirType::Bool,
            Ty::Int => MirType::Int,
            Ty::Float => MirType::Float,
            Ty::Char => MirType::Char,
            Ty::Str => MirType::Str,
            Ty::Struct { id, name, args } => MirType::Struct {
                id: *id,
                name: name.clone(),
                args: args.iter().map(MirType::from_ty).collect(),
            },
            Ty::Enum { id, name, args } => MirType::Enum {
                id: *id,
                name: name.clone(),
                args: args.iter().map(MirType::from_ty).collect(),
            },
            Ty::Option(inner) => MirType::Option(Box::new(MirType::from_ty(inner))),
            Ty::Result(a, b) => {
                MirType::Result(Box::new(MirType::from_ty(a)), Box::new(MirType::from_ty(b)))
            }
            Ty::List(inner) => MirType::List(Box::new(MirType::from_ty(inner))),
            Ty::Function { params, ret } => MirType::Function {
                params: params.iter().map(MirType::from_ty).collect(),
                ret: Box::new(MirType::from_ty(ret)),
            },
            Ty::Dyn { name, methods } => MirType::Dyn {
                name: name.clone(),
                methods: methods.clone(),
            },
            Ty::SelfTy(inner) => MirType::from_ty(inner),
            Ty::Ffi(ffi) => MirType::Ffi(MirFfiTy::from_ffi(ffi)),
            Ty::Error => MirType::Unit,
            Ty::Param(_) | Ty::Var(_) => MirType::Unit,
        }
    }

    /// Produce a stable identifier-safe textual mangling of this type
    /// for use inside monomorphized function names.
    pub fn mangle(&self) -> String {
        match self {
            MirType::Unit => "Unit".into(),
            MirType::Bool => "Bool".into(),
            MirType::Int => "Int".into(),
            MirType::Float => "Float".into(),
            MirType::Char => "Char".into(),
            MirType::Str => "Str".into(),
            MirType::Struct { name, args, .. } | MirType::Enum { name, args, .. } => {
                if args.is_empty() {
                    name.clone()
                } else {
                    let inner: Vec<String> = args.iter().map(|a| a.mangle()).collect();
                    format!("{}_{}", name, inner.join("_"))
                }
            }
            MirType::Option(inner) => format!("Option_{}", inner.mangle()),
            MirType::Result(a, b) => format!("Result_{}_{}", a.mangle(), b.mangle()),
            MirType::List(inner) => format!("List_{}", inner.mangle()),
            MirType::Function { params, ret } => {
                let mut parts: Vec<String> = params.iter().map(|p| p.mangle()).collect();
                parts.push(ret.mangle());
                format!("Fn_{}", parts.join("_"))
            }
            MirType::Dyn { name, .. } => format!("dyn_{}", name),
            MirType::Ffi(ffi) => ffi.mangle(),
        }
    }

    /// The mangled symbol for a method `method` implemented for this type.
    /// Both the impl method definition and a static or virtual call site
    /// agree on this name, so a per-type method has a unique symbol even
    /// when several types implement a method of the same name.
    pub fn method_symbol(&self, method: &str) -> String {
        method_symbol(&self.mangle(), method)
    }
}

/// Build the mangled symbol for `method` on a type whose mangled name is
/// `type_mangle`. Used by HIR (defining impl method symbols) and MIR
/// (resolving a call site), which must agree on the name.
pub fn method_symbol(type_mangle: &str, method: &str) -> String {
    format!("{}${}", type_mangle, method)
}

impl fmt::Display for MirType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MirType::Unit => f.write_str("()"),
            MirType::Bool => f.write_str("Bool"),
            MirType::Int => f.write_str("Int"),
            MirType::Float => f.write_str("Float"),
            MirType::Char => f.write_str("Char"),
            MirType::Str => f.write_str("String"),
            MirType::Struct { name, args, .. } | MirType::Enum { name, args, .. } => {
                f.write_str(name)?;
                if !args.is_empty() {
                    f.write_str("<")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "{}", a)?;
                    }
                    f.write_str(">")?;
                }
                Ok(())
            }
            MirType::Option(inner) => write!(f, "Option<{}>", inner),
            MirType::Result(t, e) => write!(f, "Result<{}, {}>", t, e),
            MirType::List(inner) => write!(f, "List<{}>", inner),
            MirType::Function { params, ret } => {
                f.write_str("fun(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
            MirType::Dyn { name, .. } => write!(f, "dyn {}", name),
            MirType::Ffi(ffi) => write!(f, "{}", ffi),
        }
    }
}

impl MirFfiTy {
    /// Build a ground MIR FFI type from a checker FFI type.
    pub fn from_ffi(ffi: &FfiTy) -> Self {
        match ffi {
            FfiTy::CInt => MirFfiTy::CInt,
            FfiTy::CLong => MirFfiTy::CLong,
            FfiTy::CSize => MirFfiTy::CSize,
            FfiTy::CStr => MirFfiTy::CStr,
            FfiTy::CFloat => MirFfiTy::CFloat,
            FfiTy::CDouble => MirFfiTy::CDouble,
            FfiTy::CPtr(inner) => MirFfiTy::CPtr(Box::new(MirType::from_ty(inner))),
        }
    }

    /// Identifier-safe mangling for use inside symbol names.
    pub fn mangle(&self) -> String {
        match self {
            MirFfiTy::CInt => "CInt".into(),
            MirFfiTy::CLong => "CLong".into(),
            MirFfiTy::CSize => "CSize".into(),
            MirFfiTy::CStr => "CStr".into(),
            MirFfiTy::CFloat => "CFloat".into(),
            MirFfiTy::CDouble => "CDouble".into(),
            MirFfiTy::CPtr(inner) => format!("CPtr_{}", inner.mangle()),
        }
    }
}

impl fmt::Display for MirFfiTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MirFfiTy::CInt => f.write_str("CInt"),
            MirFfiTy::CLong => f.write_str("CLong"),
            MirFfiTy::CSize => f.write_str("CSize"),
            MirFfiTy::CStr => f.write_str("CStr"),
            MirFfiTy::CFloat => f.write_str("CFloat"),
            MirFfiTy::CDouble => f.write_str("CDouble"),
            MirFfiTy::CPtr(inner) => write!(f, "CPtr<{}>", inner),
        }
    }
}
