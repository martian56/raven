//! Top level declaration parsing.

use crate::ast::{
    Const, Decl, DeclKind, Enum, EnumVariant, Extern, ExternFn, Function, FunctionBody,
    GenericParam, Impl, Import, ImportSource, LetDecl, MacroDef, Param, Struct, StructField, Trait,
    VariantPayload,
};
use crate::error::{ParseError, RavenError};
use crate::lexer::TokenKind;

use super::{merge_spans, ParseResult, Parser};

impl Parser {
    /// Parse one top level declaration.
    pub(crate) fn parse_decl(&mut self) -> ParseResult<Decl> {
        // Leading attributes: `@derive(...)` attaches a derived trait list,
        // `@repr(C)` marks a struct for C memory layout. Both attach to the
        // struct or enum that immediately follows.
        let mut derives = Vec::new();
        let mut repr_c = false;
        while matches!(self.peek_kind(), TokenKind::At) {
            self.parse_item_attr(&mut derives, &mut repr_c)?;
            self.skip_separators();
        }
        // `macro name { ... }`. `macro` is a contextual identifier (not a
        // keyword), so match it by spelling. Only the formatter reaches this:
        // the compile pipeline expands and strips macros before parsing.
        if matches!(self.peek_kind(), TokenKind::Identifier(n) if n == "macro")
            && matches!(self.peek_kind_at(1), TokenKind::Identifier(_))
        {
            return self.parse_macro_def();
        }
        if self.peek_word("pub")
            && !matches!(self.peek_kind_at(1), TokenKind::LParen | TokenKind::Eq)
        {
            self.advance();
        }
        if self.peek_word("class") && matches!(self.peek_kind_at(1), TokenKind::Identifier(_)) {
            return self.parse_class_decl(derives, repr_c);
        }
        if matches!(self.peek_kind(), TokenKind::Struct) {
            return Err(self.legacy("`struct`", "declare it with `class Name { ... }`"));
        }
        if matches!(self.peek_kind(), TokenKind::Const) {
            return Err(self.legacy("`const`", "declare it with `let`"));
        }
        if self.peek_word("from") {
            return self.parse_from_import();
        }
        if self.peek_word("var") && matches!(self.peek_kind_at(1), TokenKind::Identifier(_)) {
            return self.parse_let_decl();
        }
        match self.peek_kind() {
            TokenKind::Enum if !repr_c => self.parse_enum_decl(derives),
            _ if repr_c => Err(self.unexpected("`class` after `@repr(C)`")),
            _ if !derives.is_empty() => {
                Err(self.unexpected("`class` or `enum` after `@derive(...)`"))
            }
            TokenKind::Fun => self.parse_function_decl(),
            TokenKind::Trait => self.parse_trait_decl(),
            TokenKind::Impl => self.parse_impl_decl(),
            TokenKind::Extern => self.parse_extern_decl(),
            TokenKind::Import => self.parse_import_decl(),
            TokenKind::Let => self.parse_let_decl(),
            _ => Err(self.unexpected("top level item")),
        }
    }

    /// Parse one `@name(...)` item attribute. `@derive(Name, ...)` appends
    /// trait names to `derives`; `@repr(C)` sets `repr_c`. The `@` is at the
    /// cursor on entry. Any other attribute name is a parse error.
    fn parse_item_attr(&mut self, derives: &mut Vec<String>, repr_c: &mut bool) -> ParseResult<()> {
        self.expect(&TokenKind::At, "`@`")?;
        let (name, name_span) = self.expect_ident("attribute name")?;
        match name.as_str() {
            "derive" => {
                self.expect(&TokenKind::LParen, "`(`")?;
                self.skip_newlines();
                while !matches!(self.peek_kind(), TokenKind::RParen) {
                    let (t, _) = self.expect_ident("trait name")?;
                    derives.push(t);
                    self.skip_newlines();
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                    self.skip_newlines();
                }
                self.expect(&TokenKind::RParen, "`)`")?;
                Ok(())
            }
            "repr" => {
                self.expect(&TokenKind::LParen, "`(`")?;
                let (layout, layout_span) = self.expect_ident("representation name")?;
                if layout != "C" {
                    return Err(RavenError::parse(
                        ParseError::Custom(format!(
                            "unknown representation `@repr({layout})`, expected `@repr(C)`"
                        )),
                        layout_span,
                    ));
                }
                self.expect(&TokenKind::RParen, "`)`")?;
                *repr_c = true;
                Ok(())
            }
            other => Err(RavenError::parse(
                ParseError::Custom(format!(
                    "unknown attribute `@{other}`, expected `@derive` or `@repr`"
                )),
                name_span,
            )),
        }
    }

    /// Parse `macro name { (matcher) => { template } ... }`, capturing the
    /// body tokens raw. The leading `macro` identifier is at the cursor.
    fn parse_macro_def(&mut self) -> ParseResult<Decl> {
        let start = self.advance().span; // `macro`
        let (name, _) = self.expect_ident("macro name")?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        let (body, close_span) = self.capture_balanced(&TokenKind::LBrace, &TokenKind::RBrace)?;
        let span = merge_spans(&start, &close_span);
        Ok(Decl {
            kind: DeclKind::Macro(MacroDef {
                name,
                body,
                span: span.clone(),
            }),
            span,
        })
    }

    fn parse_function_decl(&mut self) -> ParseResult<Decl> {
        let fun = self.parse_function(false)?;
        let span = fun.span.clone();
        Ok(Decl {
            kind: DeclKind::Function(fun),
            span,
        })
    }

    /// Parse `fun ...` producing a [`Function`]. When `allow_signature_only`
    /// is true, the function body may be absent (trait member without
    /// default).
    fn parse_function(&mut self, allow_signature_only: bool) -> ParseResult<Function> {
        let start = self.peek().span.clone();
        self.expect(&TokenKind::Fun, "`fun`")?;
        let (name, _) = self.expect_ident("function name")?;
        let generics = self.parse_generic_params_opt()?;
        self.expect(&TokenKind::LParen, "`(`")?;
        let params = self.parse_param_list()?;
        self.expect(&TokenKind::RParen, "`)`")?;
        let ret = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let (body, end_span) = match self.peek_kind() {
            TokenKind::LBrace => {
                let block = self.parse_block()?;
                let s = block.span.clone();
                (FunctionBody::Block(block), s)
            }
            TokenKind::Eq => {
                self.advance();
                self.skip_newlines();
                let expr = self.parse_expr()?;
                let s = expr.span.clone();
                (FunctionBody::Expr(expr), s)
            }
            _ if allow_signature_only => {
                let s = self.peek().span.clone();
                (FunctionBody::None, s)
            }
            _ => return Err(self.unexpected("function body")),
        };

        let span = merge_spans(&start, &end_span);
        Ok(Function {
            name,
            generics,
            params,
            ret,
            body,
            span,
        })
    }

    /// `[T, U: Eq + Hash]`
    fn parse_bracket_generic_params(&mut self) -> ParseResult<Vec<GenericParam>> {
        self.advance();
        let mut params = Vec::new();
        self.skip_newlines();
        while !matches!(self.peek_kind(), TokenKind::RBracket) {
            let (name, name_span) = self.expect_ident("generic parameter name")?;
            let mut bounds = Vec::new();
            let mut span = name_span;
            if self.eat(&TokenKind::Colon) {
                loop {
                    let path = self.parse_type_path()?;
                    span = merge_spans(&span, &path.span);
                    bounds.push(path);
                    if !self.eat(&TokenKind::Plus) {
                        break;
                    }
                }
            }
            params.push(GenericParam { name, bounds, span });
            self.skip_newlines();
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            self.skip_newlines();
        }
        self.expect(&TokenKind::RBracket, "`]`")?;
        Ok(params)
    }

    fn parse_generic_params_opt(&mut self) -> ParseResult<Vec<GenericParam>> {
        if matches!(self.peek_kind(), TokenKind::LBracket) {
            return self.parse_bracket_generic_params();
        }
        if matches!(self.peek_kind(), TokenKind::Lt) {
            return Err(self.legacy("`<T>` generics", "write `[T]`"));
        }
        Ok(Vec::new())
    }

    fn parse_param_list(&mut self) -> ParseResult<Vec<Param>> {
        let mut params = Vec::new();
        if matches!(self.peek_kind(), TokenKind::RParen) {
            return Ok(params);
        }
        // Leading `self` receiver gets a synthetic `Self` type.
        self.skip_newlines();
        if matches!(self.peek_kind(), TokenKind::SelfLower) {
            let tok = self.advance();
            let span = tok.span.clone();
            let self_ty = crate::ast::Type {
                kind: crate::ast::TypeKind::Path(crate::ast::TypePath {
                    segments: vec![crate::ast::TypePathSegment {
                        name: "Self".to_string(),
                        generics: Vec::new(),
                        span: span.clone(),
                    }],
                    span: span.clone(),
                }),
                span: span.clone(),
            };
            params.push(Param {
                name: "self".to_string(),
                ty: self_ty,
                default: None,
                span,
            });
            self.skip_newlines();
            if !self.eat(&TokenKind::Comma) {
                return Ok(params);
            }
        }
        loop {
            self.skip_newlines();
            let (name, name_span) = self.expect_ident("parameter name")?;
            self.expect(&TokenKind::Colon, "`:`")?;
            let ty = self.parse_type()?;
            let span = merge_spans(&name_span, &ty.span);
            let default = if self.eat(&TokenKind::Eq) {
                Some(self.parse_expr()?)
            } else {
                None
            };
            params.push(Param {
                name,
                ty,
                default,
                span,
            });
            self.skip_newlines();
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            self.skip_newlines();
            if matches!(self.peek_kind(), TokenKind::RParen) {
                break;
            }
        }
        Ok(params)
    }

    fn parse_trait_decl(&mut self) -> ParseResult<Decl> {
        let start = self.advance().span; // trait
        let (name, _) = self.expect_ident("trait name")?;
        let generics = self.parse_generic_params_opt()?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        self.skip_separators();
        let mut members = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::RBrace) {
            // Only `fun` members are allowed.
            if !matches!(self.peek_kind(), TokenKind::Fun) {
                return Err(self.unexpected("`fun` or `}`"));
            }
            let f = self.parse_function(true)?;
            members.push(f);
            self.skip_separators();
        }
        let rb = self.expect(&TokenKind::RBrace, "`}`")?;
        let span = merge_spans(&start, &rb.span);
        Ok(Decl {
            kind: DeclKind::Trait(Trait {
                name,
                generics,
                members,
                span: span.clone(),
            }),
            span,
        })
    }

    fn parse_impl_decl(&mut self) -> ParseResult<Decl> {
        let start = self.advance().span; // impl
        let generics = self.parse_generic_params_opt()?;
        let first_path = self.parse_type_path()?;
        let (trait_or_type, for_type) = if self.eat(&TokenKind::For) {
            let target = self.parse_type_path()?;
            (first_path, Some(target))
        } else {
            (first_path, None)
        };
        self.expect(&TokenKind::LBrace, "`{`")?;
        self.skip_separators();
        let mut items = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::RBrace) {
            if !matches!(self.peek_kind(), TokenKind::Fun) {
                return Err(self.unexpected("`fun` or `}`"));
            }
            items.push(self.parse_function(false)?);
            self.skip_separators();
        }
        let rb = self.expect(&TokenKind::RBrace, "`}`")?;
        let span = merge_spans(&start, &rb.span);
        Ok(Decl {
            kind: DeclKind::Impl(Impl {
                generics,
                trait_or_type,
                for_type,
                items,
                from_class: false,
                span: span.clone(),
            }),
            span,
        })
    }

    fn parse_enum_decl(&mut self, derives: Vec<String>) -> ParseResult<Decl> {
        let start = self.advance().span; // enum
        let (name, _) = self.expect_ident("enum name")?;
        let generics = self.parse_generic_params_opt()?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        self.skip_separators();
        let mut variants = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::RBrace) {
            let (vname, vspan) = self.expect_ident("variant name")?;
            let mut span = vspan;
            let payload = if self.eat(&TokenKind::LParen) {
                self.skip_newlines();
                // Disambiguate: if next is `Identifier Colon`, it is
                // a named field payload.
                let is_named = matches!(self.peek_kind(), TokenKind::Identifier(_))
                    && matches!(self.peek_kind_at(1), TokenKind::Colon);
                if is_named {
                    let mut fields: Vec<StructField> = Vec::new();
                    loop {
                        self.skip_newlines();
                        let (fname, fspan) = self.expect_ident("field name")?;
                        self.expect(&TokenKind::Colon, "`:`")?;
                        let ty = self.parse_type()?;
                        let s = merge_spans(&fspan, &ty.span);
                        if fields.iter().any(|f| f.name == fname) {
                            return Err(RavenError::parse(
                                ParseError::DuplicateField(fname),
                                fspan,
                            ));
                        }
                        fields.push(StructField {
                            default: None,
                            name: fname,
                            ty,
                            span: s,
                        });
                        self.skip_newlines();
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                        self.skip_newlines();
                        if matches!(self.peek_kind(), TokenKind::RParen) {
                            break;
                        }
                    }
                    let rp = self.expect(&TokenKind::RParen, "`)`")?;
                    span = merge_spans(&span, &rp.span);
                    VariantPayload::Struct(fields)
                } else {
                    let mut tys = Vec::new();
                    if !matches!(self.peek_kind(), TokenKind::RParen) {
                        loop {
                            self.skip_newlines();
                            tys.push(self.parse_type()?);
                            self.skip_newlines();
                            if !self.eat(&TokenKind::Comma) {
                                break;
                            }
                            self.skip_newlines();
                            if matches!(self.peek_kind(), TokenKind::RParen) {
                                break;
                            }
                        }
                    }
                    let rp = self.expect(&TokenKind::RParen, "`)`")?;
                    span = merge_spans(&span, &rp.span);
                    VariantPayload::Tuple(tys)
                }
            } else {
                VariantPayload::Unit
            };
            variants.push(EnumVariant {
                name: vname,
                payload,
                field_names: Vec::new(),
                span,
            });
            if !self.eat(&TokenKind::Comma) && !matches!(self.peek_kind(), TokenKind::Newline) {
                break;
            }
            self.skip_separators();
        }
        let rb = self.expect(&TokenKind::RBrace, "`}`")?;
        let span = merge_spans(&start, &rb.span);
        Ok(Decl {
            kind: DeclKind::Enum(Enum {
                name,
                generics,
                variants,
                derives,
                span: span.clone(),
            }),
            span,
        })
    }

    fn parse_extern_decl(&mut self) -> ParseResult<Decl> {
        let start = self.advance().span; // extern
        let abi = match self.peek_kind().clone() {
            TokenKind::StringLit(s) => {
                self.advance();
                s
            }
            _ => return Err(self.unexpected("string literal for ABI")),
        };
        self.expect(&TokenKind::LBrace, "`{`")?;
        self.skip_separators();
        let mut items = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::RBrace) {
            let item_start = self.peek().span.clone();
            self.expect(&TokenKind::Fun, "`fun`")?;
            let (name, _) = self.expect_ident("function name")?;
            self.expect(&TokenKind::LParen, "`(`")?;
            // Extern parameters, with an optional trailing `...` variadic
            // marker. Parsed inline (rather than via `parse_param_list`) so the
            // `...` is only accepted here, in an extern signature.
            self.skip_newlines();
            let mut params = Vec::new();
            let mut variadic = false;
            if !matches!(self.peek_kind(), TokenKind::RParen) {
                loop {
                    self.skip_newlines();
                    if matches!(self.peek_kind(), TokenKind::DotDotDot) {
                        self.advance();
                        variadic = true;
                        self.skip_newlines();
                        break;
                    }
                    let (pname, name_span) = self.expect_ident("parameter name")?;
                    self.expect(&TokenKind::Colon, "`:`")?;
                    let ty = self.parse_type()?;
                    let span = merge_spans(&name_span, &ty.span);
                    params.push(Param {
                        default: None,
                        name: pname,
                        ty,
                        span,
                    });
                    self.skip_newlines();
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            let rp = self.expect(&TokenKind::RParen, "`)`")?;
            let ret = if self.eat(&TokenKind::Arrow) {
                Some(self.parse_type()?)
            } else {
                None
            };
            let end_span = ret.as_ref().map(|t| t.span.clone()).unwrap_or(rp.span);
            let item_span = merge_spans(&item_start, &end_span);
            items.push(ExternFn {
                name,
                params,
                ret,
                variadic,
                span: item_span,
            });
            self.skip_separators();
        }
        let rb = self.expect(&TokenKind::RBrace, "`}`")?;
        let span = merge_spans(&start, &rb.span);
        Ok(Decl {
            kind: DeclKind::Extern(Extern {
                abi,
                items,
                span: span.clone(),
            }),
            span,
        })
    }

    fn parse_import_decl(&mut self) -> ParseResult<Decl> {
        let start = self.advance().span; // import
        let (source, mut span) = self.parse_import_source(&start)?;
        let alias = if self.eat(&TokenKind::As) {
            let (n, sp) = self.expect_ident("alias name")?;
            span = merge_spans(&span, &sp);
            Some(n)
        } else {
            None
        };
        let mut selectors = Vec::new();
        if matches!(self.peek_kind(), TokenKind::LBrace) {
            return Err(self.legacy(
                "`import path { names }`",
                "write `from path import name, other`",
            ));
        }
        if self.eat(&TokenKind::LBrace) {
            self.skip_separators();
            while !matches!(self.peek_kind(), TokenKind::RBrace) {
                let (n, _) = self.expect_ident("identifier")?;
                let alias = if self.eat(&TokenKind::As) {
                    let (a, _) = self.expect_ident("alias name")?;
                    Some(a)
                } else {
                    None
                };
                selectors.push(crate::ast::ImportSelector { name: n, alias });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                self.skip_separators();
            }
            let rb = self.expect(&TokenKind::RBrace, "`}`")?;
            span = merge_spans(&span, &rb.span);
        }
        Ok(Decl {
            kind: DeclKind::Import(Import {
                source,
                alias,
                selectors,
                span: span.clone(),
            }),
            span,
        })
    }

    fn parse_import_source(
        &mut self,
        start: &crate::span::Span,
    ) -> ParseResult<(ImportSource, crate::span::Span)> {
        // Three forms:
        //   import std/io/fs ...
        //   import "github.com/..."
        //   import "./local"
        match self.peek_kind().clone() {
            TokenKind::StringLit(s) => {
                let tok = self.advance();
                let span = merge_spans(start, &tok.span);
                Ok((ImportSource::Quoted(s), span))
            }
            TokenKind::Identifier(ref n) if n == "std" => {
                let mut span = self.peek().span.clone();
                self.advance(); // std
                let mut parts: Vec<String> = Vec::new();
                while matches!(self.peek_kind(), TokenKind::Slash | TokenKind::Dot) {
                    if matches!(self.peek_kind(), TokenKind::Slash) {
                        return Err(self.legacy("`import std/...`", "write `import std.name`"));
                    }
                    self.advance(); // .
                    let (name, sp) = self.expect_ident("identifier")?;
                    parts.push(name);
                    span = merge_spans(&span, &sp);
                }
                if parts.is_empty() {
                    return Err(RavenError::parse(ParseError::InvalidImportPath, span));
                }
                Ok((ImportSource::Std(parts), span))
            }
            _ => Err(self.unexpected("import path")),
        }
    }

    /// `class Name[T]: Trait, Other { field: T = default  fun method(self) {..} }`.
    /// Emits the struct, one inherent impl holding every method, and an empty
    /// impl per listed trait; expansion moves each trait's methods over.
    fn parse_class_decl(&mut self, derives: Vec<String>, repr_c: bool) -> ParseResult<Decl> {
        let is_class = self.peek_word("class");
        let start = self.advance().span;
        let (name, name_span) = self.expect_ident("class name")?;
        let generics = self.parse_generic_params_opt()?;
        let mut traits = Vec::new();
        if self.eat(&TokenKind::Colon) {
            loop {
                traits.push(self.parse_type_path()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::LBrace, "`{`")?;
        self.skip_separators();
        let mut fields: Vec<StructField> = Vec::new();
        let mut methods = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::RBrace) {
            if self.peek_word("pub") {
                self.advance();
            }
            if matches!(self.peek_kind(), TokenKind::Fun) {
                methods.push(self.parse_function(false)?);
            } else {
                let (fname, fspan) = self.expect_ident("field name or `fun`")?;
                self.expect(&TokenKind::Colon, "`:`")?;
                let ty = self.parse_type()?;
                let default = if self.eat(&TokenKind::Eq) {
                    self.skip_newlines();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                if fields.iter().any(|f| f.name == fname) {
                    return Err(RavenError::parse(ParseError::DuplicateField(fname), fspan));
                }
                let span = merge_spans(&fspan, &ty.span);
                fields.push(StructField {
                    name: fname,
                    ty,
                    default,
                    span,
                });
                self.eat(&TokenKind::Comma);
            }
            self.skip_separators();
        }
        let rb = self.expect(&TokenKind::RBrace, "`}`")?;
        let span = merge_spans(&start, &rb.span);
        Ok(Decl {
            kind: DeclKind::Struct(Struct {
                name,
                generics,
                fields,
                derives,
                repr_c,
                is_class,
                methods,
                implements: traits,
                name_span,
                span: span.clone(),
            }),
            span,
        })
    }

    /// `from std.json import parse, stringify as s` and
    /// `from "./models" import User`.
    fn parse_from_import(&mut self) -> ParseResult<Decl> {
        let start = self.advance().span;
        let (source, mut span) = self.parse_import_source(&start)?;
        if !self.peek_word("import") && !matches!(self.peek_kind(), TokenKind::Import) {
            return Err(self.unexpected("`import`"));
        }
        self.advance();
        let mut selectors = Vec::new();
        let paren = self.eat(&TokenKind::LParen);
        loop {
            self.skip_newlines();
            let (n, sp) = self.expect_ident("name to import")?;
            span = merge_spans(&span, &sp);
            let alias = if self.eat(&TokenKind::As) {
                let (a, sp) = self.expect_ident("alias name")?;
                span = merge_spans(&span, &sp);
                Some(a)
            } else {
                None
            };
            selectors.push(crate::ast::ImportSelector { name: n, alias });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        if paren {
            self.skip_newlines();
            self.expect(&TokenKind::RParen, "`)`")?;
        }
        Ok(Decl {
            kind: DeclKind::Import(Import {
                source,
                alias: None,
                selectors,
                span: span.clone(),
            }),
            span,
        })
    }

    fn parse_let_decl(&mut self) -> ParseResult<Decl> {
        let immutable = matches!(self.peek_kind(), TokenKind::Let);
        let start = self.advance().span; // let
        let (name, _) = self.expect_ident("identifier")?;
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let init = if self.eat(&TokenKind::Eq) {
            self.skip_newlines();
            Some(self.parse_expr()?)
        } else {
            None
        };
        let end_span = init
            .as_ref()
            .map(|e| e.span.clone())
            .or_else(|| ty.as_ref().map(|t| t.span.clone()))
            .unwrap_or_else(|| start.clone());
        let span = merge_spans(&start, &end_span);
        // A module `let` bound to a constant expression is a compile-time
        // constant, folded and inlined at each use.
        if let Some(value) = init.as_ref().filter(|e| immutable && is_constant_expr(e)) {
            return Ok(Decl {
                kind: DeclKind::Const(Const {
                    name,
                    ty,
                    value: value.clone(),
                    span: span.clone(),
                }),
                span,
            });
        }
        Ok(Decl {
            kind: DeclKind::Let(LetDecl {
                name,
                ty,
                init,
                mutable: !immutable,
                span: span.clone(),
            }),
            span,
        })
    }
}

/// Literals combined with operators: a value known at compile time.
fn is_constant_expr(e: &crate::ast::Expr) -> bool {
    use crate::ast::ExprKind;
    match &e.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Char(_) => true,
        ExprKind::Unary { operand, .. } => is_constant_expr(operand),
        ExprKind::Binary { lhs, rhs, .. } => is_constant_expr(lhs) && is_constant_expr(rhs),
        ExprKind::Paren(inner) => is_constant_expr(inner),
        _ => false,
    }
}
