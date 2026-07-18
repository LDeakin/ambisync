//! The core two-variant rewriter: one `VisitMut` pass, run once per variant over a cloned item.
//!
//! The source is written in async form, so the async pass only resolves the `alt!`/`sync_only!`/`async_only!` markers.
//! The sync pass does the actual derivation: unwrap `.await`, apply the substitution table to types, expression paths, patterns and trait bounds, apply `sync(calls(...))` renames, resolve markers, and rewrite macro token streams (best effort).

use proc_macro2::Span;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::visit_mut::{self, VisitMut};
use syn::{
    Attribute, Block, Expr, ExprBlock, Ident, Lit, LitStr, Macro, Meta, Pat, Path, Stmt, Token,
    TraitBound,
};

use crate::subst::{SubstTable, strip_awaits_in_tokens};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    Sync,
    Async,
}

impl Variant {
    pub fn is_sync(self) -> bool {
        self == Variant::Sync
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MarkerKind {
    Alt,
    SyncOnly,
    AsyncOnly,
}

/// Recognize `alt!` / `sync_only!` / `async_only!`, bare or `ambisync::`-prefixed.
fn marker_kind(path: &Path) -> Option<MarkerKind> {
    let kind = match path.segments.last()?.ident.to_string().as_str() {
        "alt" => MarkerKind::Alt,
        "sync_only" => MarkerKind::SyncOnly,
        "async_only" => MarkerKind::AsyncOnly,
        _ => return None,
    };
    match path.segments.len() {
        1 => Some(kind),
        2 if path.segments[0].ident == "ambisync" => Some(kind),
        _ => None,
    }
}

/// The parsed arms of `alt!(sync => ..., async => ...)`.
struct AltArms {
    sync: Expr,
    async_: Expr,
}

impl Parse for AltArms {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut sync = None;
        let mut async_ = None;
        while !input.is_empty() {
            if input.peek(Token![async]) {
                let kw = input.parse::<Token![async]>()?;
                if async_.is_some() {
                    return Err(syn::Error::new(kw.span, "duplicate `async` arm"));
                }
                input.parse::<Token![=>]>()?;
                async_ = Some(input.parse::<Expr>()?);
            } else {
                let key: Ident = input.parse()?;
                if key != "sync" {
                    return Err(syn::Error::new(key.span(), "expected `sync` or `async`"));
                }
                if sync.is_some() {
                    return Err(syn::Error::new(key.span(), "duplicate `sync` arm"));
                }
                input.parse::<Token![=>]>()?;
                sync = Some(input.parse::<Expr>()?);
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        match (sync, async_) {
            (Some(sync), Some(async_)) => Ok(Self { sync, async_ }),
            _ => Err(input.error("`alt!` requires both a `sync` and an `async` arm")),
        }
    }
}

pub struct Rewriter<'a> {
    variant: Variant,
    table: &'a SubstTable,
    error: Option<syn::Error>,
}

impl<'a> Rewriter<'a> {
    pub fn new(variant: Variant, table: &'a SubstTable) -> Self {
        Self {
            variant,
            table,
            error: None,
        }
    }

    pub fn take_error(&mut self) -> Option<syn::Error> {
        self.error.take()
    }

    fn record_error(&mut self, err: syn::Error) {
        match &mut self.error {
            Some(existing) => existing.combine(err),
            None => self.error = Some(err),
        }
    }

    fn is_sync(&self) -> bool {
        self.variant.is_sync()
    }

    /// Whether a `sync_only!`/`async_only!` marker's contents are kept in the current variant.
    fn marker_kept(&self, kind: MarkerKind) -> bool {
        match kind {
            MarkerKind::SyncOnly => self.is_sync(),
            MarkerKind::AsyncOnly => !self.is_sync(),
            MarkerKind::Alt => unreachable!(),
        }
    }

    fn resolve_alt(&mut self, mac: &Macro) -> Option<Expr> {
        match syn::parse2::<AltArms>(mac.tokens.clone()) {
            Ok(arms) => Some(if self.is_sync() {
                arms.sync
            } else {
                arms.async_
            }),
            Err(err) => {
                self.record_error(err);
                None
            }
        }
    }
}

impl VisitMut for Rewriter<'_> {
    fn visit_block_mut(&mut self, block: &mut Block) {
        let stmts = std::mem::take(&mut block.stmts);
        let count = stmts.len();
        let mut out = Vec::with_capacity(count);
        for (index, mut stmt) in stmts.into_iter().enumerate() {
            let is_tail = index + 1 == count;
            let marker = match &stmt {
                Stmt::Macro(sm) => {
                    marker_kind(&sm.mac.path).map(|k| (k, sm.mac.clone(), sm.semi_token))
                }
                Stmt::Expr(Expr::Macro(em), semi) => {
                    marker_kind(&em.mac.path).map(|k| (k, em.mac.clone(), *semi))
                }
                _ => None,
            };
            if let Some((kind, mac, semi)) = marker {
                match kind {
                    MarkerKind::Alt => {
                        if let Some(mut expr) = self.resolve_alt(&mac) {
                            self.visit_expr_mut(&mut expr);
                            out.push(Stmt::Expr(expr, semi));
                        }
                    }
                    MarkerKind::SyncOnly | MarkerKind::AsyncOnly => {
                        if self.marker_kept(kind) {
                            match mac.parse_body_with(Block::parse_within) {
                                Ok(inner) => {
                                    for mut inner_stmt in inner {
                                        self.visit_stmt_mut(&mut inner_stmt);
                                        out.push(inner_stmt);
                                    }
                                }
                                Err(err) => self.record_error(err),
                            }
                        } else if is_tail && semi.is_none() {
                            // Value position: the dropped variant yields `()`.
                            out.push(Stmt::Expr(unit_expr(mac.span()), None));
                        }
                    }
                }
                continue;
            }
            self.visit_stmt_mut(&mut stmt);
            out.push(stmt);
        }
        block.stmts = out;
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        // Resolve markers in expression position.
        if let Expr::Macro(em) = expr
            && let Some(kind) = marker_kind(&em.mac.path)
        {
            match kind {
                MarkerKind::Alt => {
                    if let Some(chosen) = self.resolve_alt(&em.mac) {
                        *expr = chosen;
                        self.visit_expr_mut(expr);
                    }
                }
                MarkerKind::SyncOnly | MarkerKind::AsyncOnly => {
                    if self.marker_kept(kind) {
                        match em.mac.parse_body_with(Block::parse_within) {
                            Ok(stmts) => {
                                *expr = Expr::Block(ExprBlock {
                                    attrs: Vec::new(),
                                    label: None,
                                    block: Block {
                                        brace_token: Default::default(),
                                        stmts,
                                    },
                                });
                                self.visit_expr_mut(expr);
                            }
                            Err(err) => self.record_error(err),
                        }
                    } else {
                        *expr = unit_expr(em.mac.span());
                    }
                }
            }
            return;
        }

        if self.is_sync() {
            // Unwrap `.await` (repeatedly, for directly nested awaits).
            while let Expr::Await(awaited) = expr {
                let base = (*awaited.base).clone();
                *expr = base;
            }

            // An immediately-awaited async block is not equivalent to a plain block when it contains `return` or uses `move`.
            // A called closure preserves both the async block's control-flow boundary and its capture mode while making the result synchronous.
            if let Expr::Async(async_expr) = expr {
                let span = async_expr.span();
                let attrs = std::mem::take(&mut async_expr.attrs);
                let capture = async_expr.capture;
                let block = async_expr.block.clone();
                *expr = syn::parse_quote_spanned!(span=> #(#attrs)* (#capture || #block)());
                self.visit_expr_mut(expr);
                return;
            }

            if let Expr::Closure(closure) = expr {
                closure.asyncness = None;
            }
        }

        if !self.table.is_empty() {
            match expr {
                Expr::MethodCall(call) => {
                    if let Some(to) = self.table.lookup_call(&call.method) {
                        call.method = to;
                    }
                }
                Expr::Call(call) => {
                    if let Expr::Path(func) = call.func.as_mut()
                        && func.qself.is_none()
                        && let Some(last) = func.path.segments.last_mut()
                        && last.arguments.is_none()
                        && let Some(to) = self.table.lookup_call(&last.ident)
                    {
                        last.ident = to;
                    }
                }
                Expr::Path(path) if path.qself.is_none() => {
                    self.table.subst_path(&mut path.path, true);
                }
                Expr::Struct(struct_expr) if struct_expr.qself.is_none() => {
                    self.table.subst_path(&mut struct_expr.path, true);
                }
                _ => {}
            }
        }

        visit_mut::visit_expr_mut(self, expr);
    }

    fn visit_type_mut(&mut self, ty: &mut syn::Type) {
        if let Some(replacement) = self.table.subst_type(ty) {
            *ty = replacement;
            return;
        }
        // The pattern rename is in place (leading ident only), so keep visiting:
        // generic arguments may hold further substitutable types.
        self.table.subst_type_pattern(ty);
        visit_mut::visit_type_mut(self, ty);
    }

    fn visit_trait_bound_mut(&mut self, bound: &mut TraitBound) {
        self.table.subst_path(&mut bound.path, false);
        visit_mut::visit_trait_bound_mut(self, bound);
    }

    fn visit_pat_mut(&mut self, pat: &mut Pat) {
        if !self.table.is_empty() {
            match pat {
                Pat::Struct(p) if p.qself.is_none() => {
                    self.table.subst_path(&mut p.path, true);
                }
                Pat::TupleStruct(p) if p.qself.is_none() => {
                    self.table.subst_path(&mut p.path, true);
                }
                Pat::Path(p) if p.qself.is_none() => {
                    self.table.subst_path(&mut p.path, true);
                }
                _ => {}
            }
        }
        visit_mut::visit_pat_mut(self, pat);
    }

    fn visit_attribute_mut(&mut self, attr: &mut Attribute) {
        // Doc comments follow the same substitutions as code, so intra-doc links and prose mentions track each variant's names.
        if let Meta::NameValue(nv) = &mut attr.meta
            && nv.path.is_ident("doc")
            && let Expr::Lit(lit) = &mut nv.value
            && let Lit::Str(text) = &mut lit.lit
            && let Some(rewritten) = self.table.rewrite_doc_text(&text.value())
        {
            *text = LitStr::new(&rewritten, text.span());
        }
        visit_mut::visit_attribute_mut(self, attr);
    }

    fn visit_macro_mut(&mut self, mac: &mut Macro) {
        // Markers are resolved at expr/stmt level before recursion reaches here.
        // Only the sync variant strips `.await`; both variants may need item-name rewrites when an exact output name was configured.
        let mut tokens = mac.tokens.clone();
        if self.is_sync() {
            tokens = strip_awaits_in_tokens(tokens);
        }
        if !self.table.is_empty() {
            tokens = self.table.rewrite_macro_tokens(tokens);
        }
        mac.tokens = tokens;
        visit_mut::visit_macro_mut(self, mac);
    }
}

fn unit_expr(span: Span) -> Expr {
    Expr::Verbatim(quote::quote_spanned!(span=> ()))
}
