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

use crate::ast::{
    Decl, DeclKind, Enum, EnumVariant, File, GenericParam, Struct, Type, TypeKind, TypePath,
    VariantPayload,
};
use crate::error::{RavenError, ResolveError};
use crate::lexer::Lexer;
use crate::parser::parse;

/// The traits this pass can derive.
const SUPPORTED: &[&str] = &["Eq", "Hash", "ToString", "Debug", "ToJson", "FromJson"];

/// Whether a derive of `trait_name` is present on any type in `file`.
fn any_derive(file: &File, trait_name: &str) -> bool {
    file.items.iter().any(|d| match &d.kind {
        DeclKind::Struct(s) => s.derives.iter().any(|t| t == trait_name),
        DeclKind::Enum(e) => e.derives.iter().any(|t| t == trait_name),
        _ => false,
    })
}

/// Whether `file` requests `@derive(Debug)` on any type. The `Debug` trait
/// lives in `std/fmt`, not the prelude, so the expander force-merges that
/// module when a derive needs it.
pub fn needs_fmt_module(file: &File) -> bool {
    any_derive(file, "Debug")
}

/// Whether `file` requests `@derive(ToJson)` or `@derive(FromJson)` on any
/// type. Those traits and the `JsonValue` tree live in `std/json` (which
/// itself pulls in `std/error` and `std/collections`), so the expander
/// force-merges `std/json` when a derive needs it, mirroring how `Debug`
/// force-merges `std/fmt`.
pub fn needs_json_module(file: &File) -> bool {
    any_derive(file, "ToJson") || any_derive(file, "FromJson")
}

/// Append a synthesized impl for every `@derive(...)` request in `items` to
/// `combined`. Returns an error if a derive names an unsupported trait or a
/// type the slice cannot derive (a struct enum variant payload).
pub fn expand_derives(items: &[Decl], combined: &mut Vec<Decl>) -> Result<(), RavenError> {
    let mut generated = String::new();
    let mut needs_json_helpers = false;
    for decl in items {
        match &decl.kind {
            DeclKind::Struct(s) if !s.derives.is_empty() => {
                for trait_name in &s.derives {
                    check_supported(trait_name, &decl.span)?;
                    if trait_name == "ToJson" || trait_name == "FromJson" {
                        needs_json_helpers = true;
                    }
                    generated.push_str(&struct_impl(s, trait_name));
                    generated.push('\n');
                }
            }
            DeclKind::Enum(e) if !e.derives.is_empty() => {
                for trait_name in &e.derives {
                    check_supported(trait_name, &decl.span)?;
                    if trait_name == "ToJson" || trait_name == "FromJson" {
                        needs_json_helpers = true;
                    }
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
    // The derived `from_json` bodies call these helpers by bare name. A
    // bundled stdlib free function is namespaced (`std.json.f`) and so not
    // callable by its bare name from generated source, so the helpers are
    // emitted into the generated file itself, exactly once per program.
    if needs_json_helpers {
        generated.insert_str(0, JSON_HELPERS);
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
                "cannot derive `{trait_name}`: supported traits are Eq, Hash, ToString, Debug, ToJson, FromJson"
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
        "ToJson" => struct_to_json_body(s),
        "FromJson" => struct_from_json_body(s, &self_ty),
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
        "ToJson" => enum_to_json_body(e),
        "FromJson" => enum_from_json_body(e, &self_ty),
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

// ----- ToJson / FromJson -----
//
// The JSON encoding: a struct becomes a JSON object keyed by field name;
// an enum becomes a tagged object `{"tag": "Variant", "values": [...]}`,
// where a unit variant has an empty `values` array. Each field or payload
// slot serializes through its own type's `to_json` and decodes through its
// `from_json`, so a member type must itself implement the trait (the type
// checker reports a missing impl with its normal bound diagnostic).

/// Helper free functions the derived `from_json` bodies call by bare name.
/// They are emitted into the generated file (not the bundled `std/json`)
/// because a bundled free function is namespaced and so unreachable by its
/// bare name from generated source. The names are prefixed to avoid
/// colliding with a user declaration.
const JSON_HELPERS: &str = r#"fun raven_derive_json_decode<T: FromJson>(j: JsonValue) -> Result<T, Error> {
    return T.from_json(j)
}
fun raven_derive_json_field(j: JsonValue, key: String) -> Result<JsonValue, Error> {
    return match j.get(key) {
        Some(v) -> Ok(v),
        None -> Err(Error { kind: "json", message: "missing field ".concat(key) }),
    }
}
fun raven_derive_json_index(j: JsonValue, i: Int) -> Result<JsonValue, Error> {
    return match j.at(i) {
        Some(v) -> Ok(v),
        None -> Err(Error { kind: "json", message: "missing payload element" }),
    }
}
fun raven_derive_json_tag(j: JsonValue) -> Result<String, Error> {
    let t = raven_derive_json_field(j, "tag")?
    return match t {
        Str(s) -> Ok(s),
        _ -> Err(Error { kind: "json", message: "enum tag is not a string" }),
    }
}
fun raven_derive_tagged(tag: String, values: List<JsonValue>) -> JsonValue {
    let m: Map<String, JsonValue> = Map.new()
    m.set("tag", JsonValue.Str(tag))
    m.set("values", JsonValue.Array(values))
    return JsonValue.Object(m)
}
fun raven_derive_tagged_unit(tag: String) -> JsonValue {
    let values: List<JsonValue> = []
    return raven_derive_tagged(tag, values)
}
"#;

/// Render a `Type` back to its source form so a derived `from_json` can
/// annotate a decoded local with the field's exact declared type (which
/// drives the `from_json_value` type parameter). Only the type shapes that
/// appear in a struct field or a tuple variant are handled; an unexpected
/// shape renders as `_` and fails the later type check rather than silently
/// miscompiling.
fn render_type(ty: &Type) -> String {
    match &ty.kind {
        TypeKind::Path(p) => render_type_path(p),
        TypeKind::Optional(inner) => format!("Option<{}>", render_type(inner)),
        TypeKind::Unit => "()".to_string(),
        TypeKind::Dyn(p) => format!("dyn {}", render_type_path(p)),
        TypeKind::Function { .. } => "_".to_string(),
    }
}

fn render_type_path(p: &TypePath) -> String {
    let segs: Vec<String> = p
        .segments
        .iter()
        .map(|s| {
            if s.generics.is_empty() {
                s.name.clone()
            } else {
                let args: Vec<String> = s.generics.iter().map(render_type).collect();
                format!("{}<{}>", s.name, args.join(", "))
            }
        })
        .collect();
    segs.join(".")
}

/// `to_json` for a struct: build a JSON object keyed by field name, each
/// value the field's own `to_json()`. A field-less struct is the empty
/// object.
fn struct_to_json_body(s: &Struct) -> String {
    let mut body = String::from(
        "    fun to_json(self) -> JsonValue {\n        let m: Map<String, JsonValue> = Map.new()\n",
    );
    for f in &s.fields {
        body.push_str(&format!(
            "        m.set(\"{0}\", self.{0}.to_json())\n",
            f.name
        ));
    }
    body.push_str("        return JsonValue.Object(m)\n    }\n");
    body
}

/// `from_json` for a struct: pull each field from the object by name and
/// decode it to the field's declared type, propagating a missing or wrong
/// typed field as an `Err`, then construct the struct.
fn struct_from_json_body(s: &Struct, self_ty: &str) -> String {
    let mut body = format!("    fun from_json(j: JsonValue) -> Result<{self_ty}, Error> {{\n");
    for f in &s.fields {
        body.push_str(&format!(
            "        let f_{0}: {1} = raven_derive_json_decode(raven_derive_json_field(j, \"{0}\")?)?\n",
            f.name,
            render_type(&f.ty)
        ));
    }
    let inits: Vec<String> = s
        .fields
        .iter()
        .map(|f| format!("{0}: f_{0}", f.name))
        .collect();
    if inits.is_empty() {
        body.push_str(&format!("        return Ok({} {{}})\n    }}\n", s.name));
    } else {
        body.push_str(&format!(
            "        return Ok({} {{ {} }})\n    }}\n",
            s.name,
            inits.join(", ")
        ));
    }
    body
}

/// The positional payload types of a tuple variant (empty for a unit
/// variant). Struct-style variants are rejected before this is reached.
fn variant_tuple_types(v: &EnumVariant) -> &[Type] {
    match &v.payload {
        VariantPayload::Tuple(tys) => tys,
        _ => &[],
    }
}

/// `to_json` for an enum: match self and emit the tagged object
/// `{"tag": "Variant", "values": [p0, p1, ...]}`. A unit variant emits an
/// empty `values` array.
fn enum_to_json_body(e: &Enum) -> String {
    let mut body =
        String::from("    fun to_json(self) -> JsonValue {\n        return match self {\n");
    for v in &e.variants {
        let n = variant_arity(v);
        if n == 0 {
            body.push_str(&format!(
                "            {0} -> raven_derive_tagged_unit(\"{0}\"),\n",
                v.name
            ));
        } else {
            let binds: Vec<String> = (0..n).map(|i| format!("a{i}")).collect();
            let vals: Vec<String> = (0..n).map(|i| format!("a{i}.to_json()")).collect();
            body.push_str(&format!(
                "            {0}({1}) -> raven_derive_tagged(\"{0}\", [{2}]),\n",
                v.name,
                binds.join(", "),
                vals.join(", ")
            ));
        }
    }
    body.push_str("        }\n    }\n");
    body
}

/// `from_json` for an enum: read the tag, dispatch to the matching variant,
/// decode each payload slot positionally, and error on an unknown tag.
fn enum_from_json_body(e: &Enum, self_ty: &str) -> String {
    let mut body = format!(
        "    fun from_json(j: JsonValue) -> Result<{self_ty}, Error> {{\n        let tag = raven_derive_json_tag(j)?\n        let vals = raven_derive_json_field(j, \"values\")?\n"
    );
    for v in &e.variants {
        body.push_str(&format!("        if tag.equals(\"{}\") {{\n", v.name));
        let tys = variant_tuple_types(v);
        if tys.is_empty() {
            body.push_str(&format!("            return Ok({}.{})\n", e.name, v.name));
        } else {
            for (i, ty) in tys.iter().enumerate() {
                body.push_str(&format!(
                    "            let p{0}: {1} = raven_derive_json_decode(raven_derive_json_index(vals, {0})?)?\n",
                    i,
                    render_type(ty)
                ));
            }
            let args: Vec<String> = (0..tys.len()).map(|i| format!("p{i}")).collect();
            body.push_str(&format!(
                "            return Ok({}.{}({}))\n",
                e.name,
                v.name,
                args.join(", ")
            ));
        }
        body.push_str("        }\n");
    }
    body.push_str(
        "        return Err(Error { kind: \"json\", message: \"unknown tag \".concat(tag) })\n    }\n",
    );
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

    /// All declarations `expand_derives` synthesizes for `src`, including the
    /// emitted JSON helper free functions.
    fn derived_decls(src: &str) -> Vec<Decl> {
        let file = parse_src(src);
        let mut out = Vec::new();
        expand_derives(&file.items, &mut out).expect("expand");
        out
    }

    #[test]
    fn json_derive_generates_to_and_from_json_impls() {
        let impls = derived_impls("@derive(ToJson, FromJson)\nstruct Point { x: Int, y: Int }\n");
        let traits: Vec<String> = impls
            .iter()
            .map(|i| i.trait_or_type.segments[0].name.clone())
            .collect();
        assert_eq!(traits, vec!["ToJson", "FromJson"]);
        for i in &impls {
            assert_eq!(i.for_type.as_ref().unwrap().segments[0].name, "Point");
            assert_eq!(i.items.len(), 1);
        }
        let method_names: Vec<String> = impls
            .iter()
            .flat_map(|i| i.items.iter().map(|m| m.name.clone()))
            .collect();
        assert_eq!(method_names, vec!["to_json", "from_json"]);
    }

    #[test]
    fn json_derive_emits_helper_functions_once() {
        // Two types both derive the JSON traits; the shared helper free
        // functions must be emitted exactly once.
        let decls = derived_decls(
            "@derive(ToJson)\nstruct A { x: Int }\n@derive(FromJson)\nstruct B { y: Int }\n",
        );
        let decode_count = decls
            .iter()
            .filter(|d| matches!(&d.kind, DeclKind::Function(f) if f.name == "raven_derive_json_decode"))
            .count();
        assert_eq!(decode_count, 1, "decode helper must be emitted once");
        let field_count = decls
            .iter()
            .filter(
                |d| matches!(&d.kind, DeclKind::Function(f) if f.name == "raven_derive_json_field"),
            )
            .count();
        assert_eq!(field_count, 1);
    }

    #[test]
    fn non_json_derive_emits_no_json_helpers() {
        let decls = derived_decls("@derive(Eq)\nstruct Point { x: Int }\n");
        let has_helper = decls.iter().any(
            |d| matches!(&d.kind, DeclKind::Function(f) if f.name.starts_with("raven_derive_json")),
        );
        assert!(!has_helper, "no JSON helper without a JSON derive");
    }

    #[test]
    fn json_derive_on_generic_carries_per_param_bound() {
        let impls = derived_impls("@derive(ToJson)\nstruct Pair<A, B> { first: A, second: B }\n");
        let to_json = &impls[0];
        let bounds: Vec<(String, String)> = to_json
            .generics
            .iter()
            .map(|g| (g.name.clone(), g.bounds[0].segments[0].name.clone()))
            .collect();
        assert_eq!(
            bounds,
            vec![
                ("A".to_string(), "ToJson".to_string()),
                ("B".to_string(), "ToJson".to_string())
            ]
        );
    }

    #[test]
    fn json_derive_rejects_struct_variant_enum() {
        let file = parse_src("@derive(ToJson)\nenum E { V(a: Int) }\n");
        let mut out = Vec::new();
        let err = expand_derives(&file.items, &mut out).expect_err("struct variant not supported");
        assert!(
            format!("{err}").contains("struct-style variant"),
            "got: {err}"
        );
    }
}
