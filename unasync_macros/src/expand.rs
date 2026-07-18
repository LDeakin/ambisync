//! Item-level expansion: emit the async variant (the source, minus helper
//! attributes and resolved markers) and the derived sync variant, plus the
//! `scope`/`content!` shared-defaults wrappers.

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::visit_mut::VisitMut;
use syn::{
    Attribute, Fields, Ident, ImplItem, Item, ItemMod, LitStr, Meta, TraitItem, Type, parse_quote,
};

use crate::config::{Config, Flavor, sync_ident};
use crate::rewrite::{Rewriter, Variant};
use crate::subst::SubstTable;

pub fn dual(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let cfg = Config::parse_args(args)?;
    let item: Item = syn::parse2(input)?;
    expand_item(&item, &cfg, false)
}

pub fn test(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let cfg = Config::parse_args(args)?;
    let item: Item = syn::parse2(input)?;
    if !matches!(item, Item::Fn(_)) {
        return Err(syn::Error::new_spanned(
            &item,
            "#[unasync::test] can only be applied to functions",
        ));
    }
    expand_item(&item, &cfg, true)
}

pub fn scope(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let defaults = Config::parse_args(args)?;
    let module: ItemMod = syn::parse2(input)?;
    let Some((_, items)) = module.content else {
        return Err(syn::Error::new_spanned(
            &module,
            "#[unasync::scope] requires an inline module (`mod foo { ... }`); \
             for `mod foo;` use `unasync::content!` inside the module file",
        ));
    };
    let inner = process_items(items, &defaults)?;
    let attrs = &module.attrs;
    let vis = &module.vis;
    let unsafety = &module.unsafety;
    let ident = &module.ident;
    Ok(quote! {
        #(#attrs)*
        #vis #unsafety mod #ident {
            #inner
        }
    })
}

pub fn content(input: TokenStream) -> syn::Result<TokenStream> {
    let file: syn::File = syn::parse2(input)?;
    let mut defaults = Config::default();
    for attr in &file.attrs {
        if attr.path().is_ident("defaults") || is_unasync_path(attr.path(), "defaults") {
            let args = match &attr.meta {
                Meta::List(list) => list.tokens.clone(),
                _ => TokenStream::new(),
            };
            defaults = Config::parse_args(args)?;
        } else {
            return Err(syn::Error::new_spanned(
                attr,
                "unasync::content! only supports `#![defaults(...)]` inner attributes",
            ));
        }
    }
    process_items(file.items, &defaults)
}

/// Which unasync attribute an item inside `scope`/`content!` carries.
enum ScopedAttr {
    Dual,
    Test,
}

fn is_unasync_path(path: &syn::Path, name: &str) -> bool {
    path.segments.len() == 2
        && path.segments[0].ident == "unasync"
        && path.segments[1].ident == name
}

fn process_items(items: Vec<Item>, defaults: &Config) -> syn::Result<TokenStream> {
    let mut out = TokenStream::new();
    for mut item in items {
        let Some(attrs) = item_attrs_mut(&mut item) else {
            item.to_tokens(&mut out);
            continue;
        };
        let mut found = None;
        for (index, attr) in attrs.iter().enumerate() {
            let path = attr.path();
            if path.is_ident("dual") || is_unasync_path(path, "dual") {
                found = Some((index, ScopedAttr::Dual));
                break;
            }
            if is_unasync_path(path, "test") {
                found = Some((index, ScopedAttr::Test));
                break;
            }
        }
        let Some((index, kind)) = found else {
            item.to_tokens(&mut out);
            continue;
        };
        let attr = attrs.remove(index);
        let args = match &attr.meta {
            Meta::List(list) => list.tokens.clone(),
            _ => TokenStream::new(),
        };
        let cfg = Config::parse_args(args)?.merged_over(defaults);
        let is_test = matches!(kind, ScopedAttr::Test);
        if is_test && !matches!(item, Item::Fn(_)) {
            return Err(syn::Error::new_spanned(
                &attr,
                "#[unasync::test] can only be applied to functions",
            ));
        }
        out.extend(expand_item(&item, &cfg, is_test)?);
    }
    Ok(out)
}

fn expand_item(item: &Item, cfg: &Config, is_test: bool) -> syn::Result<TokenStream> {
    if cfg.skip_sync && cfg.skip_async {
        return Err(syn::Error::new_spanned(
            item,
            "`skip_sync` together with `skip_async` would emit nothing",
        ));
    }
    let mut out = TokenStream::new();
    if !cfg.skip_sync {
        out.extend(emit_variant(item, cfg, Variant::Sync, is_test)?);
    }
    if !cfg.skip_async {
        out.extend(emit_variant(item, cfg, Variant::Async, is_test)?);
    }
    Ok(out)
}

fn emit_variant(
    source: &Item,
    cfg: &Config,
    variant: Variant,
    is_test: bool,
) -> syn::Result<TokenStream> {
    let mut item = source.clone();
    // Built for both passes: the sync pass applies it via the rewriter, the
    // async pass only consults it to compute sync names for doc links.
    let mut table = SubstTable::build(cfg);

    match &mut item {
        Item::Fn(func) => {
            passthrough_attrs(&mut func.attrs, variant)?;
            if variant.is_sync() {
                func.sig.asyncness = None;
                if !cfg.keep_self {
                    func.sig.ident = sync_ident(&cfg.rename, "rename", &func.sig.ident)?;
                }
                if is_test {
                    func.attrs.push(parse_quote!(#[test]));
                }
            } else {
                if !cfg.keep_self {
                    let sync_name = sync_ident(&cfg.rename, "rename", &func.sig.ident)?;
                    stub_docs(
                        &mut func.attrs,
                        &sync_name.to_string(),
                        cfg.async_doc.as_ref(),
                    );
                }
                if is_test {
                    let attrs = cfg
                        .async_attr
                        .clone()
                        .unwrap_or_else(|| vec![parse_quote!(#[tokio::test])]);
                    func.attrs.extend(attrs);
                }
            }
        }
        Item::Trait(item_trait) => {
            passthrough_attrs(&mut item_trait.attrs, variant)?;
            filter_trait_items(&mut item_trait.items, variant);
            let sync_trait_ident = if cfg.keep_self {
                item_trait.ident.clone()
            } else {
                sync_ident(&cfg.rename_types, "rename_types", &item_trait.ident)?
            };
            let mut has_async = false;
            for trait_item in &mut item_trait.items {
                if let TraitItem::Fn(method) = trait_item {
                    passthrough_attrs(&mut method.attrs, variant)?;
                    if method.sig.asyncness.is_none() {
                        continue;
                    }
                    has_async = true;
                    if variant.is_sync() {
                        method.sig.asyncness = None;
                        if !cfg.keep_self {
                            let sync_name = sync_ident(&cfg.rename, "rename", &method.sig.ident)?;
                            table.add_auto_call(&method.sig.ident, &sync_name);
                            method.sig.ident = sync_name;
                        }
                    } else if !cfg.keep_self {
                        let sync_name = sync_ident(&cfg.rename, "rename", &method.sig.ident)?;
                        stub_docs(
                            &mut method.attrs,
                            &format!("{sync_trait_ident}::{sync_name}"),
                            None,
                        );
                    }
                }
            }
            if variant.is_sync() {
                item_trait.ident = sync_trait_ident;
            } else {
                if !cfg.keep_self {
                    stub_docs(
                        &mut item_trait.attrs,
                        &sync_trait_ident.to_string(),
                        cfg.async_doc.as_ref(),
                    );
                }
                match cfg.flavor {
                    Flavor::AsyncFn => {
                        if has_async {
                            item_trait
                                .attrs
                                .push(parse_quote!(#[allow(async_fn_in_trait)]));
                        }
                    }
                    Flavor::AsyncTrait => item_trait.attrs.extend(async_trait_attrs(cfg)),
                }
            }
        }
        Item::Impl(item_impl) => {
            passthrough_attrs(&mut item_impl.attrs, variant)?;
            filter_impl_items(&mut item_impl.items, variant);
            let inherent = item_impl.trait_.is_none();
            let sync_self_name = sync_self_name(&table, &item_impl.self_ty);
            for impl_item in &mut item_impl.items {
                if let ImplItem::Fn(method) = impl_item {
                    passthrough_attrs(&mut method.attrs, variant)?;
                    if method.sig.asyncness.is_none() {
                        continue;
                    }
                    if variant.is_sync() {
                        method.sig.asyncness = None;
                        if !cfg.keep_self {
                            let sync_name = sync_ident(&cfg.rename, "rename", &method.sig.ident)?;
                            table.add_auto_call(&method.sig.ident, &sync_name);
                            method.sig.ident = sync_name;
                        }
                    } else if !cfg.keep_self
                        && inherent
                        && let Some(self_name) = &sync_self_name
                    {
                        let sync_name = sync_ident(&cfg.rename, "rename", &method.sig.ident)?;
                        stub_docs(
                            &mut method.attrs,
                            &format!("{self_name}::{sync_name}"),
                            None,
                        );
                    }
                }
            }
            if variant.is_sync() {
                if let Some((_, trait_path, _)) = &mut item_impl.trait_ {
                    table.subst_path(trait_path, false);
                }
            } else if cfg.flavor == Flavor::AsyncTrait && !inherent {
                item_impl.attrs.extend(async_trait_attrs(cfg));
            }
        }
        Item::Struct(item_struct) => {
            passthrough_attrs(&mut item_struct.attrs, variant)?;
            filter_fields(&mut item_struct.fields, variant);
            if !cfg.keep_self {
                let sync_name = sync_ident(&cfg.rename_types, "rename_types", &item_struct.ident)?;
                if variant.is_sync() {
                    item_struct.ident = sync_name;
                } else {
                    stub_docs(
                        &mut item_struct.attrs,
                        &sync_name.to_string(),
                        cfg.async_doc.as_ref(),
                    );
                }
            }
        }
        Item::Enum(item_enum) => {
            passthrough_attrs(&mut item_enum.attrs, variant)?;
            let variants = std::mem::take(&mut item_enum.variants);
            item_enum.variants = variants
                .into_iter()
                .filter_map(|mut var| {
                    if keep_for_variant(&mut var.attrs, variant) {
                        filter_fields(&mut var.fields, variant);
                        Some(var)
                    } else {
                        None
                    }
                })
                .collect();
            if !cfg.keep_self {
                let sync_name = sync_ident(&cfg.rename_types, "rename_types", &item_enum.ident)?;
                if variant.is_sync() {
                    item_enum.ident = sync_name;
                } else {
                    stub_docs(
                        &mut item_enum.attrs,
                        &sync_name.to_string(),
                        cfg.async_doc.as_ref(),
                    );
                }
            }
        }
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "#[unasync::dual] supports `fn`, `trait`, `impl`, `struct` and `enum` items",
            ));
        }
    }

    let mut rewriter = Rewriter::new(variant, &table);
    rewriter.visit_item_mut(&mut item);
    if let Some(err) = rewriter.take_error() {
        return Err(err);
    }

    let gate = if variant.is_sync() {
        &cfg.sync_gate
    } else {
        &cfg.async_gate
    };
    if let Some(feature) = gate
        && let Some(attrs) = item_attrs_mut(&mut item)
    {
        attrs.insert(0, parse_quote!(#[cfg(feature = #feature)]));
    }

    Ok(quote!(#item))
}

fn item_attrs_mut(item: &mut Item) -> Option<&mut Vec<Attribute>> {
    match item {
        Item::Fn(i) => Some(&mut i.attrs),
        Item::Trait(i) => Some(&mut i.attrs),
        Item::Impl(i) => Some(&mut i.attrs),
        Item::Struct(i) => Some(&mut i.attrs),
        Item::Enum(i) => Some(&mut i.attrs),
        Item::Const(i) => Some(&mut i.attrs),
        Item::Static(i) => Some(&mut i.attrs),
        Item::Type(i) => Some(&mut i.attrs),
        Item::Mod(i) => Some(&mut i.attrs),
        Item::Use(i) => Some(&mut i.attrs),
        Item::Macro(i) => Some(&mut i.attrs),
        _ => None,
    }
}

/// Handle a `#[sync_only]`/`#[async_only]` gating attribute on a field, enum
/// variant, or trait/impl item: returns whether the element is kept in this
/// variant, removing the marker attribute either way.
fn keep_for_variant(attrs: &mut Vec<Attribute>, variant: Variant) -> bool {
    let mut keep = true;
    attrs.retain(|attr| {
        let path = attr.path();
        if path.is_ident("sync_only") || is_unasync_path(path, "sync_only") {
            if !variant.is_sync() {
                keep = false;
            }
            false
        } else if path.is_ident("async_only") || is_unasync_path(path, "async_only") {
            if variant.is_sync() {
                keep = false;
            }
            false
        } else {
            true
        }
    });
    keep
}

fn filter_fields(fields: &mut Fields, variant: Variant) {
    match fields {
        Fields::Named(named) => {
            let all = std::mem::take(&mut named.named);
            named.named = all
                .into_iter()
                .filter_map(|mut field| {
                    keep_for_variant(&mut field.attrs, variant).then_some(field)
                })
                .collect();
        }
        Fields::Unnamed(unnamed) => {
            let all = std::mem::take(&mut unnamed.unnamed);
            unnamed.unnamed = all
                .into_iter()
                .filter_map(|mut field| {
                    keep_for_variant(&mut field.attrs, variant).then_some(field)
                })
                .collect();
        }
        Fields::Unit => {}
    }
}

fn filter_trait_items(items: &mut Vec<TraitItem>, variant: Variant) {
    items.retain_mut(|item| {
        let attrs = match item {
            TraitItem::Const(i) => &mut i.attrs,
            TraitItem::Fn(i) => &mut i.attrs,
            TraitItem::Type(i) => &mut i.attrs,
            TraitItem::Macro(i) => &mut i.attrs,
            _ => return true,
        };
        keep_for_variant(attrs, variant)
    });
}

fn filter_impl_items(items: &mut Vec<ImplItem>, variant: Variant) {
    items.retain_mut(|item| {
        let attrs = match item {
            ImplItem::Const(i) => &mut i.attrs,
            ImplItem::Fn(i) => &mut i.attrs,
            ImplItem::Type(i) => &mut i.attrs,
            ImplItem::Macro(i) => &mut i.attrs,
            _ => return true,
        };
        keep_for_variant(attrs, variant)
    });
}

/// Resolve `#[sync_attr(...)]` / `#[async_attr(...)]` passthrough attributes:
/// the matching variant's payload is unwrapped into a real attribute, the
/// other variant's is dropped.
fn passthrough_attrs(attrs: &mut Vec<Attribute>, variant: Variant) -> syn::Result<()> {
    let mut out = Vec::with_capacity(attrs.len());
    for attr in attrs.drain(..) {
        let path = attr.path();
        let which = if path.is_ident("sync_attr") || is_unasync_path(path, "sync_attr") {
            Some(Variant::Sync)
        } else if path.is_ident("async_attr") || is_unasync_path(path, "async_attr") {
            Some(Variant::Async)
        } else {
            None
        };
        match which {
            Some(target) if target == variant => {
                let Meta::List(list) = &attr.meta else {
                    return Err(syn::Error::new_spanned(
                        &attr,
                        "expected `#[sync_attr(...)]` / `#[async_attr(...)]` with arguments",
                    ));
                };
                let tokens = &list.tokens;
                out.push(parse_quote!(#[#tokens]));
            }
            Some(_) => {}
            None => out.push(attr),
        }
    }
    *attrs = out;
    Ok(())
}

/// Replace an async variant's doc comments with the conventional stub:
/// `/// Async variant of [`target`].`
fn stub_docs(attrs: &mut Vec<Attribute>, target: &str, extra: Option<&LitStr>) {
    attrs.retain(|attr| !matches!(&attr.meta, Meta::NameValue(nv) if nv.path.is_ident("doc")));
    let text = format!(" Async variant of [`{target}`].");
    attrs.insert(0, parse_quote!(#[doc = #text]));
    if let Some(extra) = extra {
        attrs.insert(1, parse_quote!(#[doc = ""]));
        attrs.insert(2, parse_quote!(#[doc = #extra]));
    }
}

fn async_trait_attrs(cfg: &Config) -> Vec<Attribute> {
    if cfg.wasm_send {
        vec![
            parse_quote!(#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]),
            parse_quote!(#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]),
        ]
    } else {
        vec![parse_quote!(#[async_trait::async_trait])]
    }
}

/// The last ident of the impl self type after mapping it through the table —
/// i.e. the sync struct name, used for doc links on async inherent methods.
fn sync_self_name(table: &SubstTable, self_ty: &Type) -> Option<Ident> {
    let mut ty = self_ty.clone();
    if let Some(substituted) = table.subst_type(&ty) {
        ty = substituted;
    } else if let Type::Path(type_path) = &mut ty {
        table.subst_path(&mut type_path.path, false);
    }
    type_last_ident(&ty)
}

fn type_last_ident(ty: &Type) -> Option<Ident> {
    match ty {
        Type::Path(type_path) => type_path.path.segments.last().map(|seg| seg.ident.clone()),
        Type::Reference(reference) => type_last_ident(&reference.elem),
        _ => None,
    }
}
