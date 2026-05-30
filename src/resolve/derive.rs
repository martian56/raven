//! Compile-time `@derive(...)` expansion.
//!
//! A `@derive(Eq, Hash, ToString, Debug)` attribute on a struct or enum
//! asks the compiler to synthesize the corresponding trait impls instead
//! of the user hand writing `equals`/`hash`/`to_string`/`debug`. This pass
//! runs after stdlib expansion and before resolution, so the generated
//! impls flow through resolve, type checking, HIR, MIR, and codegen exactly
//! like a hand written impl.
//!
//! The generated impls are produced as Raven source text and re-parsed,
//! mirroring how the stdlib itself is merged. The generated bodies call the
//! field and payload types' own trait methods (`equals`, `hash`,
//! `to_string`, `debug`), so a field type must itself implement the derived
//! trait, which the type checker verifies. For a generic type the synthesized
//! impl carries the per-parameter bound for the trait being derived, for
//! example `impl<A: Eq, B: Eq> Eq for Pair<A, B>`.
//!
//! See `docs/v2/specs/derive.md` for the syntax, the generated-impl shapes,
//! and the formatting conventions.

use crate::ast::{Decl, DeclKind, Enum, EnumVariant, File, GenericParam, Struct, VariantPayload};
use crate::error::{RavenError, ResolveError};
use crate::lexer::Lexer;
use crate::parser::parse;

/// The four traits this slice can derive.
const SUPPORTED: &[&str] = &["Eq", "Hash", "ToString", "Debug"];

/// Whether `file` requests `@derive(Debug)` on any type. The `Debug` trait
/// lives in `std/fmt`, not the prelude, so the expander force-merges that
/// module when a derive needs it.
pub fn needs_fmt_module(file: &File) -> bool {
    file.items.iter().any(|d| match &d.kind {
        DeclKind::Struct(s) => s.derives.iter().any(|t| t == "Debug"),
        DeclKind::Enum(e) => e.derives.iter().any(|t| t == "Debug"),
        _ => false,
    })
}

/// Append a synthesized impl for every `@derive(...)` request in `items` to
/// `combined`. Returns an error if a derive names an unsupported trait or a
/// type the slice cannot derive (a struct enum variant payload).
pub fn expand_derives(items: &[Decl], combined: &mut Vec<Decl>) -> Result<(), RavenError> {
    let mut generated = String::new();
    for decl in items {
        match &decl.kind {
            DeclKind::Struct(s) if !s.derives.is_empty() => {
                for trait_name in &s.derives {
                    check_supported(trait_name, &decl.span)?;
                    generated.push_str(&struct_impl(s, trait_name));
                    generated.push('\n');
                }
            }
            DeclKind::Enum(e) if !e.derives.is_empty() => {
                for trait_name in &e.derives {
                    check_supported(trait_name, &decl.span)?;
                    generated.push_str(&enum_impl(e, trait_name, &decl.span)?);
                    generated.push('\n');
                }
            }
            _ => {}
        }
    }
    if generated.is_empty() {
        return Ok(());
    }
    let path = std::path::PathBuf::from("<derive>");
    let tokens = Lexer::new(generated.clone(), path.clone())
        .tokenize()
        .map_err(|e| derive_error(format!("lex: {e}")))?;
    let parsed = parse(&tokens).map_err(|e| derive_error(format!("parse: {e}")))?;
    combined.extend(parsed.items);
    Ok(())
}

fn check_supported(trait_name: &str, span: &crate::span::Span) -> Result<(), RavenError> {
    if SUPPORTED.contains(&trait_name) {
        Ok(())
    } else {
        Err(RavenError::resolve(
            ResolveError::Other(format!(
                "cannot derive `{trait_name}`: supported traits are Eq, Hash, ToString, Debug"
            )),
            span.clone(),
        ))
    }
}

/// Render the impl header generics and the self type, applying the derived
/// trait as a bound on every type parameter. `struct Pair<A, B>` derived for
/// `Eq` yields the header pieces `<A: Eq, B: Eq>` and `Pair<A, B>`.
fn impl_head(name: &str, generics: &[GenericParam], trait_name: &str) -> (String, String) {
    if generics.is_empty() {
        return (String::new(), name.to_string());
    }
    let bounds: Vec<String> = generics
        .iter()
        .map(|g| format!("{}: {trait_name}", g.name))
        .collect();
    let args: Vec<String> = generics.iter().map(|g| g.name.clone()).collect();
    (
        format!("<{}>", bounds.join(", ")),
        format!("{name}<{}>", args.join(", ")),
    )
}

fn struct_impl(s: &Struct, trait_name: &str) -> String {
    let (gen, self_ty) = impl_head(&s.name, &s.generics, trait_name);
    let body = match trait_name {
        "Eq" => struct_eq_body(s, &self_ty),
        "Hash" => struct_hash_body(s),
        "ToString" => struct_to_string_body(s, "to_string"),
        "Debug" => struct_to_string_body(s, "debug"),
        _ => unreachable!("checked by check_supported"),
    };
    format!("impl{gen} {trait_name} for {self_ty} {{\n{body}}}\n")
}

fn struct_eq_body(s: &Struct, self_ty: &str) -> String {
    // The `other` parameter is annotated with the concrete self type rather
    // than `Self`: the type checker does not yet accept `Self` as a non
    // receiver parameter type in a method signature.
    if s.fields.is_empty() {
        return format!("    fun equals(self, other: {self_ty}) -> Bool {{ return true }}\n");
    }
    let parts: Vec<String> = s
        .fields
        .iter()
        .map(|f| format!("self.{0}.equals(other.{0})", f.name))
        .collect();
    format!(
        "    fun equals(self, other: {self_ty}) -> Bool {{ return {} }}\n",
        parts.join(" && ")
    )
}

fn struct_hash_body(s: &Struct) -> String {
    let mut body = String::from("    fun hash(self) -> Int {\n        let h = 17\n");
    for f in &s.fields {
        body.push_str(&format!("        h = h * 31 + self.{}.hash()\n", f.name));
    }
    body.push_str("        return h\n    }\n");
    body
}

/// Build a `to_string` or `debug` body for a struct. `method` is the field
/// formatter to call (`to_string` for readable output, `debug` for quoted
/// output). The output shape is `TypeName { field: value, ... }`.
fn struct_to_string_body(s: &Struct, method: &str) -> String {
    let fn_name = if method == "debug" {
        "debug"
    } else {
        "to_string"
    };
    if s.fields.is_empty() {
        return format!(
            "    fun {fn_name}(self) -> String {{ return \"{}\" }}\n",
            s.name
        );
    }
    let mut parts = Vec::new();
    for f in &s.fields {
        parts.push(format!("{0}: ${{self.{0}.{method}()}}", f.name));
    }
    format!(
        "    fun {fn_name}(self) -> String {{ return \"{} {{ {} }}\" }}\n",
        s.name,
        parts.join(", ")
    )
}

fn enum_impl(e: &Enum, trait_name: &str, span: &crate::span::Span) -> Result<String, RavenError> {
    // A struct enum variant (named-field payload) is out of scope here:
    // a readable derive for it needs field-by-field projection that the
    // tuple path does not cover, so reject it with a clear message rather
    // than emit a wrong impl.
    for v in &e.variants {
        if let VariantPayload::Struct(_) = v.payload {
            return Err(RavenError::resolve(
                ResolveError::Other(format!(
                    "cannot derive `{trait_name}` for enum `{}`: struct-style variant `{}` is not supported yet",
                    e.name, v.name
                )),
                span.clone(),
            ));
        }
    }
    let (gen, self_ty) = impl_head(&e.name, &e.generics, trait_name);
    let body = match trait_name {
        "Eq" => enum_eq_body(e, &self_ty),
        "Hash" => enum_hash_body(e),
        "ToString" => enum_to_string_body(e, "to_string"),
        "Debug" => enum_to_string_body(e, "debug"),
        _ => unreachable!("checked by check_supported"),
    };
    Ok(format!(
        "impl{gen} {trait_name} for {self_ty} {{\n{body}}}\n"
    ))
}

/// Number of positional payload slots a variant carries.
fn variant_arity(v: &EnumVariant) -> usize {
    match &v.payload {
        VariantPayload::Unit => 0,
        VariantPayload::Tuple(tys) => tys.len(),
        VariantPayload::Struct(fields) => fields.len(),
    }
}

fn enum_eq_body(e: &Enum, self_ty: &str) -> String {
    let mut body = format!(
        "    fun equals(self, other: {self_ty}) -> Bool {{\n        return match self {{\n"
    );
    for v in &e.variants {
        let n = variant_arity(v);
        if n == 0 {
            body.push_str(&format!(
                "            {0} -> match other {{ {0} -> true, _ -> false }},\n",
                v.name
            ));
        } else {
            let a_binds: Vec<String> = (0..n).map(|i| format!("a{i}")).collect();
            let b_binds: Vec<String> = (0..n).map(|i| format!("b{i}")).collect();
            let cmp: Vec<String> = (0..n).map(|i| format!("a{i}.equals(b{i})")).collect();
            body.push_str(&format!(
                "            {0}({1}) -> match other {{ {0}({2}) -> {3}, _ -> false }},\n",
                v.name,
                a_binds.join(", "),
                b_binds.join(", "),
                cmp.join(" && ")
            ));
        }
    }
    body.push_str("        }\n    }\n");
    body
}

fn enum_hash_body(e: &Enum) -> String {
    let mut body = String::from("    fun hash(self) -> Int {\n        return match self {\n");
    for (idx, v) in e.variants.iter().enumerate() {
        let n = variant_arity(v);
        if n == 0 {
            body.push_str(&format!("            {} -> {},\n", v.name, idx * 17 + 1));
        } else {
            let binds: Vec<String> = (0..n).map(|i| format!("a{i}")).collect();
            let mut acc = format!("{}", idx * 17 + 1);
            for i in 0..n {
                acc = format!("({acc}) * 31 + a{i}.hash()");
            }
            body.push_str(&format!(
                "            {}({}) -> {},\n",
                v.name,
                binds.join(", "),
                acc
            ));
        }
    }
    body.push_str("        }\n    }\n");
    body
}

/// Build a `to_string` or `debug` body for an enum. A unit variant prints
/// its name; a payload variant prints `Name(p0, p1)` using each payload's
/// `to_string`/`debug`.
fn enum_to_string_body(e: &Enum, method: &str) -> String {
    let fn_name = if method == "debug" {
        "debug"
    } else {
        "to_string"
    };
    let mut body = format!("    fun {fn_name}(self) -> String {{\n        return match self {{\n");
    for v in &e.variants {
        let n = variant_arity(v);
        if n == 0 {
            body.push_str(&format!("            {0} -> \"{0}\",\n", v.name));
        } else {
            let binds: Vec<String> = (0..n).map(|i| format!("a{i}")).collect();
            let parts: Vec<String> = (0..n).map(|i| format!("${{a{i}.{method}()}}")).collect();
            body.push_str(&format!(
                "            {0}({1}) -> \"{0}({2})\",\n",
                v.name,
                binds.join(", "),
                parts.join(", ")
            ));
        }
    }
    body.push_str("        }\n    }\n");
    body
}

fn derive_error(detail: String) -> RavenError {
    let span = crate::span::Span::point(
        std::sync::Arc::new(std::path::PathBuf::from("<derive>")),
        0,
        1,
        1,
    );
    RavenError::resolve(
        ResolveError::Other(format!("derive expansion failed: {detail}")),
        span,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Impl;
    use std::path::PathBuf;

    fn parse_src(src: &str) -> File {
        let tokens = Lexer::new(src.to_string(), PathBuf::from("main.rv"))
            .tokenize()
            .expect("lex");
        parse(&tokens).expect("parse")
    }

    /// Collect the trait impls (`impl Trait for Type`) that `expand_derives`
    /// synthesizes for `src`.
    fn derived_impls(src: &str) -> Vec<Impl> {
        let file = parse_src(src);
        let mut out = Vec::new();
        expand_derives(&file.items, &mut out).expect("expand");
        out.into_iter()
            .filter_map(|d| match d.kind {
                DeclKind::Impl(i) => Some(i),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn struct_derive_generates_one_impl_per_trait() {
        let impls =
            derived_impls("@derive(Eq, Hash, ToString, Debug)\nstruct Point { x: Int, y: Int }\n");
        let traits: Vec<String> = impls
            .iter()
            .map(|i| i.trait_or_type.segments[0].name.clone())
            .collect();
        assert_eq!(traits, vec!["Eq", "Hash", "ToString", "Debug"]);
        // Every impl targets `Point` and carries one method.
        for i in &impls {
            assert_eq!(i.for_type.as_ref().unwrap().segments[0].name, "Point");
            assert_eq!(i.items.len(), 1);
        }
    }

    #[test]
    fn generic_derive_carries_per_param_bound() {
        let impls = derived_impls("@derive(Eq)\nstruct Pair<A, B> { first: A, second: B }\n");
        let eq = &impls[0];
        // The impl is generic with the derived trait as each param's bound.
        let bounds: Vec<(String, String)> = eq
            .generics
            .iter()
            .map(|g| (g.name.clone(), g.bounds[0].segments[0].name.clone()))
            .collect();
        assert_eq!(
            bounds,
            vec![
                ("A".to_string(), "Eq".to_string()),
                ("B".to_string(), "Eq".to_string())
            ]
        );
        // The self type is `Pair<A, B>`.
        let for_ty = eq.for_type.as_ref().unwrap();
        assert_eq!(for_ty.segments[0].name, "Pair");
        assert_eq!(for_ty.segments[0].generics.len(), 2);
    }

    #[test]
    fn enum_payload_derive_generates_eq_and_to_string() {
        let impls = derived_impls(
            "@derive(Eq, ToString)\nenum Shape { Dot, Circle(Int), Rect(Int, Int) }\n",
        );
        assert_eq!(impls.len(), 2);
        for i in &impls {
            assert_eq!(i.for_type.as_ref().unwrap().segments[0].name, "Shape");
        }
    }

    #[test]
    fn no_derive_generates_nothing() {
        let impls = derived_impls("struct Point { x: Int }\n");
        assert!(impls.is_empty());
    }

    #[test]
    fn unsupported_trait_is_rejected() {
        let file = parse_src("@derive(Ord)\nstruct Point { x: Int }\n");
        let mut out = Vec::new();
        let err = expand_derives(&file.items, &mut out).expect_err("Ord is not derivable");
        assert!(format!("{err}").contains("Ord"), "got: {err}");
    }

    #[test]
    fn struct_variant_enum_is_rejected() {
        let file = parse_src("@derive(Eq)\nenum E { V(a: Int) }\n");
        let mut out = Vec::new();
        let err = expand_derives(&file.items, &mut out).expect_err("struct variant not supported");
        assert!(
            format!("{err}").contains("struct-style variant"),
            "got: {err}"
        );
    }
}
