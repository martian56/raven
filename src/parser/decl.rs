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
        match self.peek_kind() {
            TokenKind::Struct => self.parse_struct_decl(derives, repr_c),
            TokenKind::Enum if !repr_c => self.parse_enum_decl(derives),
            _ if repr_c => Err(self.unexpected("`struct` after `@repr(C)`")),
            _ if !derives.is_empty() => {
                Err(self.unexpected("`struct` or `enum` after `@derive(...)`"))
            }
            TokenKind::Fun => self.parse_function_decl(),
            TokenKind::Trait => self.parse_trait_decl(),
            TokenKind::Impl => self.parse_impl_decl(),
            TokenKind::Extern => self.parse_extern_decl(),
            TokenKind::Import => self.parse_import_decl(),
            TokenKind::Const => self.parse_const_decl(),
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

    fn parse_generic_params_opt(&mut self) -> ParseResult<Vec<GenericParam>> {
        if !matches!(self.peek_kind(), TokenKind::Lt) {
            return Ok(Vec::new());
        }
        self.advance(); // <
        let mut params = Vec::new();
        loop {
            self.skip_newlines();
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
            if matches!(self.peek_kind(), TokenKind::Gt | TokenKind::Shr) {
                break;
            }
        }
        // Close angle: same as type args.
        match self.peek_kind() {
            TokenKind::Gt => {
                self.advance();
            }
            TokenKind::Shr => {
                // Split into two `>` and consume the first half.
                self.split_shr_close_first();
            }
            _ => return Err(self.unexpected("`>`")),
        }
        Ok(params)
    }

    /// Split a `>>` at the cursor into two `>` and consume the first.
    /// Mirror of the type args closer; kept here because generic
    /// parameters can also nest (e.g. `fun foo<T: Box<U>>(...)`).
    fn split_shr_close_first(&mut self) {
        let tok = self.tokens[self.pos].clone();
        let second_half = crate::span::Span::new(
            tok.span.file.clone(),
            tok.span.start + 1,
            tok.span.end,
            tok.span.line,
            tok.span.col.saturating_add(1),
        );
        self.tokens[self.pos] = crate::lexer::Token {
            kind: TokenKind::Gt,
            span: second_half,
        };
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
            params.push(Param { name, ty, span });
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

    fn parse_struct_decl(&mut self, derives: Vec<String>, repr_c: bool) -> ParseResult<Decl> {
        let start = self.advance().span; // struct
        let (name, _) = self.expect_ident("struct name")?;
        let generics = self.parse_generic_params_opt()?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        self.skip_separators();
        let mut fields: Vec<StructField> = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::RBrace) {
            let (fname, fspan) = self.expect_ident("field name")?;
            self.expect(&TokenKind::Colon, "`:`")?;
            let ty = self.parse_type()?;
            let span = merge_spans(&fspan, &ty.span);
            if fields.iter().any(|f| f.name == fname) {
                return Err(RavenError::parse(ParseError::DuplicateField(fname), fspan));
            }
            fields.push(StructField {
                name: fname,
                ty,
                span,
            });
            // Separator: `,`, newline, or both.
            if !self.eat(&TokenKind::Comma) && !matches!(self.peek_kind(), TokenKind::Newline) {
                break;
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
                span: span.clone(),
            }),
            span,
        })
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
            let params = self.parse_param_list()?;
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
        if self.eat(&TokenKind::LBrace) {
            self.skip_separators();
            while !matches!(self.peek_kind(), TokenKind::RBrace) {
                let (n, _) = self.expect_ident("identifier")?;
                selectors.push(n);
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
                while matches!(self.peek_kind(), TokenKind::Slash) {
                    self.advance(); // /
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

    fn parse_const_decl(&mut self) -> ParseResult<Decl> {
        let start = self.advance().span; // const
        let (name, _) = self.expect_ident("identifier")?;
        self.expect(&TokenKind::Colon, "`:`")?;
        let ty = self.parse_type()?;
        self.expect(&TokenKind::Eq, "`=`")?;
        self.skip_newlines();
        let value = self.parse_expr()?;
        let span = merge_spans(&start, &value.span);
        Ok(Decl {
            kind: DeclKind::Const(Const {
                name,
                ty,
                value,
                span: span.clone(),
            }),
            span,
        })
    }

    fn parse_let_decl(&mut self) -> ParseResult<Decl> {
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
        Ok(Decl {
            kind: DeclKind::Let(LetDecl {
                name,
                ty,
                init,
                span: span.clone(),
            }),
            span,
        })
    }
}
