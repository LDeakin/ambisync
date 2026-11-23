//! Item-level expansion: emit the async variant (the source, minus helper attributes and resolved markers) and the derived sync variant, plus the `scope`/`scoped!` shared-defaults wrappers.

use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::visit_mut::VisitMut;
use syn::{
    parse_quote, AttrStyle, Attribute, Fields, Ident, ImplItem, ImplItemFn, Item, ItemMod,
    ItemTrait, LitStr, Meta, Token, TraitItem, Visibility,
};

use crate::config::{async_ident, Config, Declaration, Flavor, MemberFallback, SendPolicy};
use crate::rewrite::{Rewriter, Variant};
use crate::subst::SubstTable;

pub fn ambisync(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let cfg = Config::parse_args(args)?;
    let item: Item = syn::parse2(input)?;
    expand_item(&item, &cfg, false)
}

/// Expand the async methods of a mixed sync/async trait or impl in place.
///
/// Ordinary members are emitted once. Each async method is emitted as written (and optionally
/// feature-gated) alongside a mechanically derived synchronous sibling.
///
/// Unlike `#[ambisync]`, a single item is emitted, so the `types(...)` table applies only inside
/// the derived sync method bodies and signatures — never to the trait name or, for impls, to the
/// impl header, which serves both APIs.
pub fn paired(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let cfg = Config::parse_args(args)?;
    cfg.validate_effective()?;
    validate_paired_config(&cfg)?;

    let item: Item = syn::parse2(input)?;
    match item {
        Item::Trait(item_trait) => paired_trait(item_trait, &cfg),
        Item::Impl(item_impl) => paired_impl(item_impl, &cfg),
        other => Err(syn::Error::new_spanned(
            other,
            "#[ambisync::paired] supports `trait` and `impl` items",
        )),
    }
}

fn paired_trait(mut item: ItemTrait, cfg: &Config) -> syn::Result<TokenStream> {
    let mut output_items = Vec::with_capacity(item.items.len());
    let mut has_async_methods = false;

    for trait_item in std::mem::take(&mut item.items) {
        let TraitItem::Fn(mut source) = trait_item else {
            output_items.push(trait_item);
            continue;
        };
        if source.sig.asyncness.is_none() {
            output_items.push(TraitItem::Fn(source));
            continue;
        }
        let rewrite_trait_method = |method: syn::TraitItemFn, variant, table: &SubstTable| {
            let mut item = TraitItem::Fn(method);
            let mut rewriter = Rewriter::new(variant, table);
            rewriter.visit_trait_item_mut(&mut item);
            rewriter.take_error().map_or_else(
                || {
                    let TraitItem::Fn(method) = item else {
                        unreachable!("rewriting a trait method preserves its item kind")
                    };
                    Ok(method)
                },
                Err,
            )
        };
        if let Some(variant) = take_only_marker(&mut source.attrs)? {
            has_async_methods |= !variant.is_sync();
            let method = paired_only(source, cfg, variant, rewrite_trait_method)?;
            output_items.push(TraitItem::Fn(method));
            continue;
        }
        has_async_methods = true;

        let (sync_method, async_method) = paired_methods(source, cfg, rewrite_trait_method)?;
        output_items.push(TraitItem::Fn(sync_method));
        output_items.push(TraitItem::Fn(async_method));
    }

    item.items = output_items;
    if has_async_methods {
        match cfg.flavor() {
            Flavor::AsyncFn => item.attrs.push(parse_quote!(#[allow(async_fn_in_trait)])),
            Flavor::AsyncTrait => item.attrs.extend(paired_async_trait_attrs(cfg)),
        }
    }
    Ok(quote!(#item))
}

fn paired_impl(mut item: syn::ItemImpl, cfg: &Config) -> syn::Result<TokenStream> {
    normalize_signature_only_members(&mut item.items);
    let mut output_items = Vec::with_capacity(item.items.len());
    let mut has_async_methods = false;

    for impl_item in std::mem::take(&mut item.items) {
        let ImplItem::Fn(mut source) = impl_item else {
            output_items.push(impl_item);
            continue;
        };
        if source.sig.asyncness.is_none() {
            output_items.push(ImplItem::Fn(source));
            continue;
        }
        let rewrite_impl_method = |method: ImplItemFn, variant, table: &SubstTable| {
            let mut item = ImplItem::Fn(method);
            let mut rewriter = Rewriter::new(variant, table);
            rewriter.visit_impl_item_mut(&mut item);
            rewriter.take_error().map_or_else(
                || {
                    let ImplItem::Fn(method) = item else {
                        unreachable!("rewriting an impl method preserves its item kind")
                    };
                    Ok(method)
                },
                Err,
            )
        };
        if let Some(variant) = take_only_marker(&mut source.attrs)? {
            has_async_methods |= !variant.is_sync();
            let method = paired_only(source, cfg, variant, rewrite_impl_method)?;
            output_items.push(ImplItem::Fn(method));
            continue;
        }
        has_async_methods = true;

        let (sync_method, async_method) = paired_methods(source, cfg, rewrite_impl_method)?;
        output_items.push(ImplItem::Fn(sync_method));
        output_items.push(ImplItem::Fn(async_method));
    }

    item.items = output_items;
    restore_signature_only_members(&mut item.items);
    if has_async_methods && matches!(cfg.flavor(), Flavor::AsyncTrait) {
        item.attrs.extend(paired_async_trait_attrs(cfg));
    }
    Ok(quote!(#item))
}

/// Take a `#[sync_only]` / `#[async_only]` marker off an async method, if present.
///
/// Returns the variant the method is restricted to. Unlike `#[ambisync]`, which emits two items
/// and filters members per item, `#[ambisync::paired]` emits one item, so a marked `async fn`
/// yields only that variant's form rather than a sync/async pair.
fn take_only_marker(attrs: &mut Vec<Attribute>) -> syn::Result<Option<Variant>> {
    let mut found = None;
    let mut out = Vec::with_capacity(attrs.len());
    for attr in attrs.drain(..) {
        let path = attr.path();
        let variant = if path.is_ident("sync_only") || is_ambisync_path(path, "sync_only") {
            Some(Variant::Sync)
        } else if path.is_ident("async_only") || is_ambisync_path(path, "async_only") {
            Some(Variant::Async)
        } else {
            None
        };
        match variant {
            Some(variant) => {
                if found.is_some() {
                    return Err(syn::Error::new_spanned(
                        &attr,
                        "duplicate `#[sync_only]` / `#[async_only]` on method",
                    ));
                }
                found = Some(variant);
            }
            None => out.push(attr),
        }
    }
    *attrs = out;
    Ok(found)
}

/// Emit a single variant of a `#[sync_only]` / `#[async_only]` async method.
///
/// The sync form is derived (asyncness stripped, renames applied); the async form is emitted as
/// written. Either way only one method is produced, and it carries that variant's feature gate.
fn paired_only<M, F>(mut source: M, cfg: &Config, variant: Variant, rewrite: F) -> syn::Result<M>
where
    M: PairedMethod,
    F: Fn(M, Variant, &SubstTable) -> syn::Result<M>,
{
    let table = if variant.is_sync() {
        let mut table = SubstTable::build(cfg);
        rewrite_method_signature(
            source.sig_mut(),
            &[],
            cfg,
            Variant::Sync,
            MemberFallback::Error,
            &mut table,
        )?;
        table
    } else {
        SubstTable::default()
    };

    let mut method = rewrite(source, variant, &table)?;
    let gate = if variant.is_sync() {
        &cfg.sync.gate
    } else {
        &cfg.async_.gate
    };
    if let Some(feature) = gate {
        method
            .attrs_mut()
            .insert(0, parse_quote!(#[cfg(feature = #feature)]));
    }
    Ok(method)
}

/// Derive the synchronous sibling of one async method and return it beside the async original.
///
/// `rewrite` applies the variant [`Rewriter`] to a method in its own item kind, which is the only
/// part that differs between traits and impls.
fn paired_methods<M, F>(mut source: M, cfg: &Config, rewrite: F) -> syn::Result<(M, M)>
where
    M: PairedMethod + Clone,
    F: Fn(M, Variant, &SubstTable) -> syn::Result<M>,
{
    let sync_name = take_sync_name(source.attrs_mut())?;
    let sync_signature = take_sync_signature(source.attrs_mut())?;
    if sync_name.is_some() && sync_signature.is_some() {
        return Err(syn::Error::new_spanned(
            source.sig(),
            "`#[sync_name(...)]` and `#[sync_signature(fn ...)]` cannot be used on the same method",
        ));
    }

    let source_name = source.sig().ident.clone();
    let sync_name = if let Some(signature) = &sync_signature {
        signature.ident.clone()
    } else if let Some(sync_name) = sync_name {
        sync_name
    } else {
        cfg.sync
            .sync_member_ident(&source_name, MemberFallback::Error)?
    };
    ensure_distinct(&sync_name, &source_name)?;

    let mut sync_method = source.clone();
    passthrough_attrs(sync_method.attrs_mut(), Variant::Sync)?;
    let mut table = SubstTable::build(cfg);
    rewrite_method_signature(
        sync_method.sig_mut(),
        &[(source_name, sync_name)],
        cfg,
        Variant::Sync,
        MemberFallback::Error,
        &mut table,
    )?;
    let mut sync_method = rewrite(sync_method, Variant::Sync, &table)?;
    if let Some(signature) = sync_signature {
        *sync_method.sig_mut() = signature;
    }
    if let Some(feature) = &cfg.sync.gate {
        sync_method
            .attrs_mut()
            .insert(0, parse_quote!(#[cfg(feature = #feature)]));
    }

    passthrough_attrs(source.attrs_mut(), Variant::Async)?;
    let mut async_method = rewrite(source, Variant::Async, &SubstTable::default())?;
    if let Some(feature) = &cfg.async_.gate {
        async_method
            .attrs_mut()
            .insert(0, parse_quote!(#[cfg(feature = #feature)]));
    }

    Ok((sync_method, async_method))
}

/// The parts of a trait or impl method that [`paired_methods`] manipulates uniformly.
trait PairedMethod {
    fn attrs_mut(&mut self) -> &mut Vec<Attribute>;
    fn sig(&self) -> &syn::Signature;
    fn sig_mut(&mut self) -> &mut syn::Signature;
}

impl PairedMethod for syn::TraitItemFn {
    fn attrs_mut(&mut self) -> &mut Vec<Attribute> {
        &mut self.attrs
    }
    fn sig(&self) -> &syn::Signature {
        &self.sig
    }
    fn sig_mut(&mut self) -> &mut syn::Signature {
        &mut self.sig
    }
}

impl PairedMethod for ImplItemFn {
    fn attrs_mut(&mut self) -> &mut Vec<Attribute> {
        &mut self.attrs
    }
    fn sig(&self) -> &syn::Signature {
        &self.sig
    }
    fn sig_mut(&mut self) -> &mut syn::Signature {
        &mut self.sig
    }
}

fn validate_paired_config(cfg: &Config) -> syn::Result<()> {
    let unsupported = cfg
        .sync
        .name
        .as_ref()
        .map(Ident::span)
        .or_else(|| cfg.async_.name.as_ref().map(Ident::span))
        .or_else(|| cfg.sync.declaration.as_ref().map(Declaration::span))
        .or_else(|| cfg.async_.declaration.as_ref().map(Declaration::span))
        .or_else(|| cfg.sync.doc.as_ref().map(LitStr::span))
        .or_else(|| cfg.async_.doc.as_ref().map(LitStr::span))
        .or_else(|| cfg.sync.attrs.first().map(Attribute::span))
        .or_else(|| cfg.async_.attrs.first().map(Attribute::span))
        .or_else(|| {
            cfg.sync
                .test_attr
                .as_ref()
                .and_then(|attrs| attrs.first())
                .map(Attribute::span)
        })
        .or_else(|| {
            cfg.async_
                .test_attr
                .as_ref()
                .and_then(|attrs| attrs.first())
                .map(Attribute::span)
        });
    if let Some(span) = unsupported {
        return Err(syn::Error::new(
            span,
            "`#[ambisync::paired]` accepts feature/flavor/send and sync rename tables; item-level names, declarations, docs, attributes and test attributes are not applicable",
        ));
    }
    Ok(())
}

fn paired_async_trait_attrs(cfg: &Config) -> Vec<Attribute> {
    let gate = cfg.async_.gate.as_ref();
    match (gate, cfg.send_policy()) {
        (None, _) => async_trait_attrs(cfg),
        (Some(feature), SendPolicy::Always) => {
            vec![parse_quote!(#[cfg_attr(feature = #feature, async_trait::async_trait)])]
        }
        (Some(feature), SendPolicy::Never) => {
            vec![parse_quote!(#[cfg_attr(feature = #feature, async_trait::async_trait(?Send))])]
        }
        (Some(feature), SendPolicy::Cfg(predicate)) => {
            let negated = negate_cfg_predicate(&predicate);
            vec![
                parse_quote!(#[cfg_attr(all(feature = #feature, #predicate), async_trait::async_trait)]),
                parse_quote!(#[cfg_attr(all(feature = #feature, #negated), async_trait::async_trait(?Send))]),
            ]
        }
    }
}

pub fn test(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let cfg = Config::parse_args(args)?;
    let item: Item = syn::parse2(input)?;
    if !matches!(item, Item::Fn(_)) {
        return Err(syn::Error::new_spanned(
            &item,
            "#[ambisync::test] can only be applied to functions",
        ));
    }
    expand_item(&item, &cfg, true)
}

pub fn scope(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let defaults = Config::parse_args(args)?;
    validate_scope_defaults(&defaults)?;
    let module: ItemMod = syn::parse2(input)?;
    let Some((_, items)) = module.content else {
        return Err(syn::Error::new_spanned(
            &module,
            "#[ambisync::scope] requires an inline module (`mod foo { ... }`); \
             for `mod foo;` use `ambisync::scoped!` inside the module file",
        ));
    };
    let inner = process_items(items, &defaults)?;
    let (outer_attrs, inner_attrs): (Vec<_>, Vec<_>) = module
        .attrs
        .into_iter()
        .partition(|attr| matches!(attr.style, AttrStyle::Outer));
    let vis = &module.vis;
    let unsafety = &module.unsafety;
    let ident = &module.ident;
    Ok(quote! {
        #(#outer_attrs)*
        #vis #unsafety mod #ident {
            #(#inner_attrs)*
            #inner
        }
    })
}

pub fn scoped(input: TokenStream) -> syn::Result<TokenStream> {
    let file: syn::File = syn::parse2(input)?;
    let mut defaults = Config::default();
    for attr in &file.attrs {
        if attr.path().is_ident("defaults") || is_ambisync_path(attr.path(), "defaults") {
            let args = match &attr.meta {
                Meta::List(list) => list.tokens.clone(),
                _ => TokenStream::new(),
            };
            defaults = Config::parse_args(args)?;
        } else {
            return Err(syn::Error::new_spanned(
                attr,
                "ambisync::scoped! only supports `#![defaults(...)]` inner attributes",
            ));
        }
    }
    validate_scope_defaults(&defaults)?;
    process_items(file.items, &defaults)
}

fn validate_scope_defaults(defaults: &Config) -> syn::Result<()> {
    if defaults.sync.name.is_some()
        || defaults.async_.name.is_some()
        || defaults.sync.declaration.is_some()
        || defaults.async_.declaration.is_some()
    {
        let exact_name = defaults
            .sync
            .name
            .as_ref()
            .or(defaults.async_.name.as_ref());
        let declaration = defaults
            .sync
            .declaration
            .as_ref()
            .or(defaults.async_.declaration.as_ref());
        let span =
            exact_name.map_or_else(|| declaration.expect("checked above").span(), Ident::span);
        return Err(syn::Error::new(
            span,
            "exact variant names and declarations are item-specific and cannot be scope/scoped! \
             defaults; put them on the item",
        ));
    }
    Ok(())
}

/// Which ambisync attribute an item inside `scope`/`scoped!` carries.
enum ScopedAttr {
    Ambisync,
    Test,
}

fn is_ambisync_path(path: &syn::Path, name: &str) -> bool {
    path.segments.len() == 2
        && path.segments[0].ident == "ambisync"
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
            if path.is_ident("ambisync") || is_ambisync_path(path, "ambisync") {
                found = Some((index, ScopedAttr::Ambisync));
                break;
            }
            if is_ambisync_path(path, "test") {
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
                "#[ambisync::test] can only be applied to functions",
            ));
        }
        out.extend(expand_item(&item, &cfg, is_test)?);
    }
    Ok(out)
}

fn expand_item(item: &Item, cfg: &Config, is_test: bool) -> syn::Result<TokenStream> {
    cfg.validate_effective()?;
    validate_declaration_kinds(item, cfg)?;
    if !is_test && (cfg.sync.test_attr.is_some() || cfg.async_.test_attr.is_some()) {
        return Err(syn::Error::new_spanned(
            item,
            "`sync(test_attr = #[...])` / `async(test_attr = #[...])` are only valid on \
             `#[ambisync::test]`",
        ));
    }
    let mut out = TokenStream::new();
    out.extend(emit_variant(item, cfg, Variant::Sync, is_test)?);
    out.extend(emit_variant(item, cfg, Variant::Async, is_test)?);
    Ok(out)
}

fn validate_declaration_kinds(item: &Item, cfg: &Config) -> syn::Result<()> {
    let expected = match item {
        Item::Fn(_) => "function",
        Item::Trait(_) => "trait",
        Item::Impl(_) => "impl",
        Item::Struct(_) => "struct",
        Item::Enum(_) => "enum",
        _ => return Ok(()),
    };
    for (variant, declaration) in [
        ("sync", cfg.sync.declaration.as_ref()),
        ("async", cfg.async_.declaration.as_ref()),
    ] {
        if let Some(declaration) = declaration {
            if declaration.kind_name() != expected {
                return Err(syn::Error::new(
                    declaration.span(),
                    format!(
                        "`{variant}(declaration {{ ... }})` for this item must declare a \
                         {expected}, not a {}",
                        declaration.kind_name()
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn emit_variant(
    source: &Item,
    cfg: &Config,
    variant: Variant,
    is_test: bool,
) -> syn::Result<TokenStream> {
    let mut item = source.clone();
    let mut member_sync_signatures = Vec::new();
    let mut table = if variant.is_sync() {
        SubstTable::build(cfg)
    } else {
        SubstTable::default()
    };

    match &mut item {
        Item::Fn(func) => {
            passthrough_attrs(&mut func.attrs, variant)?;
            if func.sig.asyncness.is_none() {
                return Err(syn::Error::new_spanned(
                    &func.sig,
                    "ambisync functions must be written in async form",
                ));
            }
            let source_name = func.sig.ident.clone();
            let async_name = cfg.async_.declaration.as_ref().map_or_else(
                || async_ident(cfg.async_.name.as_ref(), &source_name),
                |declaration| {
                    declaration
                        .ident()
                        .expect("function declaration has a name")
                        .clone()
                },
            );
            let sync_name = if let Some(declaration) = &cfg.sync.declaration {
                declaration
                    .ident()
                    .expect("function declaration has a name")
                    .clone()
            } else {
                cfg.sync
                    .sync_fn_ident(cfg.sync.name.as_ref(), &async_name)?
            };
            ensure_distinct(&sync_name, &async_name)?;
            if variant.is_sync() {
                func.sig.asyncness = None;
                table.add_item_call(&source_name, &sync_name);
                func.sig.ident = sync_name;
            } else {
                table.add_item_call(&source_name, &async_name);
                func.sig.ident = async_name;
            }
        }
        Item::Trait(item_trait) => {
            passthrough_attrs(&mut item_trait.attrs, variant)?;
            filter_trait_items(&mut item_trait.items, variant);
            let mut member_sync_names = Vec::new();
            for trait_item in &mut item_trait.items {
                if let TraitItem::Fn(method) = trait_item {
                    let sync_name = take_sync_name(&mut method.attrs)?;
                    let sync_signature = take_sync_signature(&mut method.attrs)?;
                    if sync_name.is_some() && sync_signature.is_some() {
                        return Err(syn::Error::new_spanned(
                            &method.sig,
                            "`#[sync_name(...)]` and `#[sync_signature(fn ...)]` cannot be used on the same method",
                        ));
                    }
                    if let Some(sync_signature) = sync_signature {
                        let source_name = method.sig.ident.clone();
                        let sync_name = sync_signature.ident.clone();
                        if variant.is_sync() {
                            table.add_item_call(&source_name, &sync_name);
                            member_sync_signatures.push(sync_signature);
                        }
                        member_sync_names.push((source_name, sync_name));
                    } else if let Some(sync_name) = sync_name {
                        let source_name = method.sig.ident.clone();
                        if variant.is_sync() {
                            table.add_item_call(&source_name, &sync_name);
                        }
                        member_sync_names.push((source_name, sync_name));
                    }
                }
            }
            let source_trait_ident = item_trait.ident.clone();
            let async_trait_ident = cfg.async_.declaration.as_ref().map_or_else(
                || async_ident(cfg.async_.name.as_ref(), &source_trait_ident),
                |declaration| {
                    declaration
                        .ident()
                        .expect("trait declaration has a name")
                        .clone()
                },
            );
            let sync_trait_ident = if let Some(declaration) = &cfg.sync.declaration {
                declaration
                    .ident()
                    .expect("trait declaration has a name")
                    .clone()
            } else {
                cfg.sync
                    .sync_type_ident(cfg.sync.name.as_ref(), &async_trait_ident)?
            };
            ensure_distinct(&sync_trait_ident, &async_trait_ident)?;
            let output_trait_ident = if variant.is_sync() {
                &sync_trait_ident
            } else {
                &async_trait_ident
            };
            table.add_item_type(&source_trait_ident, output_trait_ident);
            for trait_item in &mut item_trait.items {
                if let TraitItem::Fn(method) = trait_item {
                    passthrough_attrs(&mut method.attrs, variant)?;
                    // The sync and async traits are distinct, so their methods cannot clash.
                    rewrite_method_signature(
                        &mut method.sig,
                        &member_sync_names,
                        cfg,
                        variant,
                        MemberFallback::Identity,
                        &mut table,
                    )?;
                }
            }
            item_trait.ident = if variant.is_sync() {
                sync_trait_ident
            } else {
                async_trait_ident
            };
        }
        Item::Impl(item_impl) => {
            if cfg.sync.name.is_some() || cfg.async_.name.is_some() {
                return Err(syn::Error::new_spanned(
                    item_impl,
                    "`sync(name = ...)` / `async(name = ...)` cannot name an impl block; \
                     use `sync(types(...))` for its trait/self types and `sync(fns(...))` \
                     for methods",
                ));
            }
            passthrough_attrs(&mut item_impl.attrs, variant)?;
            normalize_signature_only_members(&mut item_impl.items);
            filter_impl_items(&mut item_impl.items, variant);
            let mut member_sync_names = Vec::new();
            for impl_item in &mut item_impl.items {
                if let ImplItem::Fn(method) = impl_item {
                    let sync_name = take_sync_name(&mut method.attrs)?;
                    let sync_signature = take_sync_signature(&mut method.attrs)?;
                    if sync_name.is_some() && sync_signature.is_some() {
                        return Err(syn::Error::new_spanned(
                            &method.sig,
                            "`#[sync_name(...)]` and `#[sync_signature(fn ...)]` cannot be used on the same method",
                        ));
                    }
                    if let Some(sync_signature) = sync_signature {
                        let source_name = method.sig.ident.clone();
                        let sync_name = sync_signature.ident.clone();
                        if variant.is_sync() {
                            table.add_item_call(&source_name, &sync_name);
                            member_sync_signatures.push(sync_signature);
                        }
                        member_sync_names.push((source_name, sync_name));
                    } else if let Some(sync_name) = sync_name {
                        let source_name = method.sig.ident.clone();
                        if variant.is_sync() {
                            table.add_item_call(&source_name, &sync_name);
                        }
                        member_sync_names.push((source_name, sync_name));
                    }
                }
            }
            let fallback = if variant.is_sync() && impl_namespaces_differ(item_impl, &table) {
                MemberFallback::Identity
            } else {
                MemberFallback::Error
            };
            for impl_item in &mut item_impl.items {
                if let ImplItem::Fn(method) = impl_item {
                    passthrough_attrs(&mut method.attrs, variant)?;
                    rewrite_method_signature(
                        &mut method.sig,
                        &member_sync_names,
                        cfg,
                        variant,
                        fallback,
                        &mut table,
                    )?;
                }
            }
            if variant.is_sync() {
                if let Some((trait_path, _)) = &mut item_impl.trait_ {
                    table.subst_path(trait_path, false);
                }
            }
        }
        Item::Struct(item_struct) => {
            passthrough_attrs(&mut item_struct.attrs, variant)?;
            filter_fields(&mut item_struct.fields, variant);
            let source_name = item_struct.ident.clone();
            let async_name = cfg.async_.declaration.as_ref().map_or_else(
                || async_ident(cfg.async_.name.as_ref(), &source_name),
                |declaration| {
                    declaration
                        .ident()
                        .expect("struct declaration has a name")
                        .clone()
                },
            );
            let sync_name = if let Some(declaration) = &cfg.sync.declaration {
                declaration
                    .ident()
                    .expect("struct declaration has a name")
                    .clone()
            } else {
                cfg.sync
                    .sync_type_ident(cfg.sync.name.as_ref(), &async_name)?
            };
            ensure_distinct(&sync_name, &async_name)?;
            let output_name = if variant.is_sync() {
                &sync_name
            } else {
                &async_name
            };
            table.add_item_type(&source_name, output_name);
            item_struct.ident = output_name.clone();
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
            let source_name = item_enum.ident.clone();
            let async_name = cfg.async_.declaration.as_ref().map_or_else(
                || async_ident(cfg.async_.name.as_ref(), &source_name),
                |declaration| {
                    declaration
                        .ident()
                        .expect("enum declaration has a name")
                        .clone()
                },
            );
            let sync_name = if let Some(declaration) = &cfg.sync.declaration {
                declaration
                    .ident()
                    .expect("enum declaration has a name")
                    .clone()
            } else {
                cfg.sync
                    .sync_type_ident(cfg.sync.name.as_ref(), &async_name)?
            };
            ensure_distinct(&sync_name, &async_name)?;
            let output_name = if variant.is_sync() {
                &sync_name
            } else {
                &async_name
            };
            table.add_item_type(&source_name, output_name);
            item_enum.ident = output_name.clone();
        }
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "#[ambisync] supports `fn`, `trait`, `impl`, `struct` and `enum` items",
            ));
        }
    }

    let mut rewriter = Rewriter::new(variant, &table);
    rewriter.visit_item_mut(&mut item);
    if let Some(err) = rewriter.take_error() {
        return Err(err);
    }
    if variant.is_sync() {
        apply_sync_signatures(&mut item, member_sync_signatures);
    }

    apply_variant_metadata(&mut item, cfg, variant, is_test)?;

    if let Item::Impl(item_impl) = &mut item {
        restore_signature_only_members(&mut item_impl.items);
    }

    let gate = if variant.is_sync() {
        &cfg.sync.gate
    } else {
        &cfg.async_.gate
    };
    if let Some(feature) = gate {
        if let Some(attrs) = item_attrs_mut(&mut item) {
            attrs.insert(0, parse_quote!(#[cfg(feature = #feature)]));
        }
    }

    Ok(quote!(#item))
}

fn apply_variant_metadata(
    item: &mut Item,
    cfg: &Config,
    variant: Variant,
    is_test: bool,
) -> syn::Result<()> {
    let (doc, attrs, test_attr, declaration) = if variant.is_sync() {
        (
            &cfg.sync.doc,
            &cfg.sync.attrs,
            &cfg.sync.test_attr,
            &cfg.sync.declaration,
        )
    } else {
        (
            &cfg.async_.doc,
            &cfg.async_.attrs,
            &cfg.async_.test_attr,
            &cfg.async_.declaration,
        )
    };

    if let Some(declaration) = declaration {
        apply_declaration(item, declaration)?;
    }

    apply_async_flavor(item, cfg, variant);

    let Some(item_attrs) = item_attrs_mut(item) else {
        return Ok(());
    };
    if let Some(doc) = doc {
        append_doc(item_attrs, doc);
    }
    item_attrs.extend(attrs.iter().cloned());

    if is_test {
        let default = if variant.is_sync() {
            vec![parse_quote!(#[test])]
        } else {
            vec![parse_quote!(#[tokio::test])]
        };
        item_attrs.extend(test_attr.clone().unwrap_or(default));
    }
    Ok(())
}

fn apply_declaration(item: &mut Item, declaration: &Declaration) -> syn::Result<()> {
    match (item, declaration) {
        (Item::Fn(item), Declaration::Fn(declaration)) => {
            item.attrs = declaration.attrs.clone();
            item.vis = declaration.vis.clone();
            item.sig = declaration.sig.clone();
        }
        (Item::Trait(item), Declaration::Trait(declaration)) => {
            let members = std::mem::take(&mut item.items);
            *item = declaration.clone();
            item.items = members;
        }
        (Item::Impl(item), Declaration::Impl(declaration)) => {
            let members = std::mem::take(&mut item.items);
            *item = declaration.clone();
            item.items = members;
        }
        (Item::Struct(item), Declaration::Struct(declaration)) => {
            let fields = item.fields.clone();
            let semi_token = item.semi_token;
            *item = declaration.clone();
            item.fields = fields;
            item.semi_token = semi_token;
        }
        (Item::Enum(item), Declaration::Enum(declaration)) => {
            let variants = std::mem::take(&mut item.variants);
            *item = declaration.clone();
            item.variants = variants;
        }
        _ => {
            return Err(syn::Error::new(
                declaration.span(),
                "declaration kind does not match the annotated item",
            ));
        }
    }
    Ok(())
}

fn apply_async_flavor(item: &mut Item, cfg: &Config, variant: Variant) {
    if variant.is_sync() {
        return;
    }
    match item {
        Item::Trait(item_trait) => match cfg.flavor() {
            Flavor::AsyncFn => {
                if item_trait.items.iter().any(
                    |item| matches!(item, TraitItem::Fn(method) if method.sig.asyncness.is_some()),
                ) {
                    item_trait
                        .attrs
                        .push(parse_quote!(#[allow(async_fn_in_trait)]));
                }
            }
            Flavor::AsyncTrait => item_trait.attrs.extend(async_trait_attrs(cfg)),
        },
        Item::Impl(item_impl)
            if cfg.flavor() == Flavor::AsyncTrait && item_impl.trait_.is_some() =>
        {
            item_impl.attrs.extend(async_trait_attrs(cfg));
        }
        _ => {}
    }
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

/// Handle a `#[sync_only]`/`#[async_only]` gating attribute on a field, enum variant, or trait/impl item: returns whether the element is kept in this variant, removing the marker attribute either way.
fn keep_for_variant(attrs: &mut Vec<Attribute>, variant: Variant) -> bool {
    let mut keep = true;
    attrs.retain(|attr| {
        let path = attr.path();
        if path.is_ident("sync_only") || is_ambisync_path(path, "sync_only") {
            if !variant.is_sync() {
                keep = false;
            }
            false
        } else if path.is_ident("async_only") || is_ambisync_path(path, "async_only") {
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

/// A signature-only impl member (`pub async fn f(...);`), which syn parses as
/// `ImplItem::Verbatim`. Such members are meaningful to attribute macros that accept
/// body-less methods inside impls (for example the `inherent` crate).
struct SignatureOnlyFn {
    attrs: Vec<Attribute>,
    vis: Visibility,
    defaultness: Option<Token![default]>,
    sig: syn::Signature,
}

impl Parse for SignatureOnlyFn {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis = input.parse()?;
        let defaultness = input.parse()?;
        let sig = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(Self {
            attrs,
            vis,
            defaultness,
            sig,
        })
    }
}

/// The internal marker distinguishing a normalized signature-only member from an ordinary method.
const SIGNATURE_ONLY_MARKER: &str = "__ambisync_signature_only";

/// Turn each signature-only `ImplItem::Verbatim` member into a temporary `ImplItem::Fn` with an
/// empty body and an internal marker attribute, so member filtering, renaming, and rewriting
/// treat it like an ordinary method. [`restore_signature_only_members`] undoes this on emission.
/// Verbatim members that are not signature-only fns pass through untouched.
fn normalize_signature_only_members(items: &mut [ImplItem]) {
    for item in items {
        let ImplItem::Verbatim(tokens) = item else {
            continue;
        };
        let Ok(member) = syn::parse2::<SignatureOnlyFn>(tokens.clone()) else {
            continue;
        };
        let SignatureOnlyFn {
            mut attrs,
            vis,
            defaultness,
            sig,
        } = member;
        let marker = Ident::new(SIGNATURE_ONLY_MARKER, proc_macro2::Span::call_site());
        attrs.push(parse_quote!(#[#marker]));
        let mut modifiers = syn::FnModifiers::default();
        modifiers.defaultness = defaultness;
        *item = ImplItem::Fn(ImplItemFn {
            attrs,
            vis,
            modifiers,
            sig,
            block: parse_quote!({}),
        });
    }
}

/// Re-emit normalized signature-only members in their body-less form, dropping the marker.
fn restore_signature_only_members(items: &mut [ImplItem]) {
    for item in items {
        let ImplItem::Fn(method) = item else {
            continue;
        };
        let Some(marker) = method
            .attrs
            .iter()
            .position(|attr| attr.path().is_ident(SIGNATURE_ONLY_MARKER))
        else {
            continue;
        };
        method.attrs.remove(marker);
        let ImplItemFn {
            attrs,
            vis,
            modifiers,
            sig,
            ..
        } = &*method;
        let defaultness = &modifiers.defaultness;
        *item = ImplItem::Verbatim(quote!(#(#attrs)* #vis #defaultness #sig;));
    }
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

/// Whether the sync impl derived from `item_impl` implements a different trait or has a different self type than the
/// async impl, so that methods of the same name in each cannot clash.
fn impl_namespaces_differ(item_impl: &syn::ItemImpl, table: &SubstTable) -> bool {
    let mut header = item_impl.clone();
    header.items.clear();
    header.attrs.clear();
    if let Some((trait_path, _)) = &mut header.trait_ {
        table.subst_path(trait_path, false);
    }
    // Errors in the header are reported by the rewrite of the whole item.
    Rewriter::new(Variant::Sync, table).visit_item_impl_mut(&mut header);
    let key = |item: &syn::ItemImpl| {
        (
            item.trait_
                .as_ref()
                .map(|(path, _)| path.to_token_stream().to_string()),
            item.self_ty.to_token_stream().to_string(),
        )
    };
    key(&header) != key(item_impl)
}

fn rewrite_method_signature(
    signature: &mut syn::Signature,
    member_sync_names: &[(Ident, Ident)],
    cfg: &Config,
    variant: Variant,
    fallback: MemberFallback,
    table: &mut SubstTable,
) -> syn::Result<()> {
    if !variant.is_sync() {
        return Ok(());
    }

    let source_name = signature.ident.clone();
    let explicit_sync_name = member_sync_names
        .iter()
        .find(|(source, _)| *source == source_name)
        .map(|(_, sync)| sync.clone());
    if signature.asyncness.is_some() {
        signature.asyncness = None;
        let sync_name = if let Some(sync_name) = explicit_sync_name {
            sync_name
        } else {
            cfg.sync.sync_member_ident(&source_name, fallback)?
        };
        table.add_auto_call(&source_name, &sync_name);
        signature.ident = sync_name;
    } else if let Some(sync_name) = explicit_sync_name {
        signature.ident = sync_name;
    }
    Ok(())
}

/// Remove and parse a `#[sync_name(...)]` member-level sync name override.
fn take_sync_name(attrs: &mut Vec<Attribute>) -> syn::Result<Option<Ident>> {
    let mut sync_name = None;
    let mut out = Vec::with_capacity(attrs.len());
    for attr in attrs.drain(..) {
        let path = attr.path();
        if path.is_ident("sync_name") || is_ambisync_path(path, "sync_name") {
            if sync_name.is_some() {
                return Err(syn::Error::new_spanned(
                    &attr,
                    "duplicate `#[sync_name(...)]` on method",
                ));
            }
            sync_name = Some(attr.parse_args::<Ident>()?);
        } else {
            out.push(attr);
        }
    }
    *attrs = out;
    Ok(sync_name)
}

/// Remove and parse a `#[sync_signature(fn ...)]` member-level sync signature override.
fn take_sync_signature(attrs: &mut Vec<Attribute>) -> syn::Result<Option<syn::Signature>> {
    let mut sync_signature = None;
    let mut out = Vec::with_capacity(attrs.len());
    for attr in attrs.drain(..) {
        let path = attr.path();
        if path.is_ident("sync_signature") || is_ambisync_path(path, "sync_signature") {
            if sync_signature.is_some() {
                return Err(syn::Error::new_spanned(
                    &attr,
                    "duplicate `#[sync_signature(fn ...)]` on method",
                ));
            }
            let signature = attr.parse_args::<syn::Signature>()?;
            if signature.asyncness.is_some() {
                return Err(syn::Error::new_spanned(
                    &signature,
                    "`#[sync_signature(fn ...)]` must declare a synchronous method",
                ));
            }
            sync_signature = Some(signature);
        } else {
            out.push(attr);
        }
    }
    *attrs = out;
    Ok(sync_signature)
}

/// Apply verbatim sync signatures after body/type rewriting, so the declarations themselves are
/// not passed through the async-to-sync substitution table.
fn apply_sync_signatures(item: &mut Item, signatures: Vec<syn::Signature>) {
    for signature in signatures {
        match item {
            Item::Trait(item_trait) => {
                if let Some(TraitItem::Fn(method)) = item_trait.items.iter_mut().find(
                    |item| matches!(item, TraitItem::Fn(method) if method.sig.ident == signature.ident),
                ) {
                    method.sig = signature;
                }
            }
            Item::Impl(item_impl) => {
                if let Some(ImplItem::Fn(method)) = item_impl.items.iter_mut().find(
                    |item| matches!(item, ImplItem::Fn(method) if method.sig.ident == signature.ident),
                ) {
                    method.sig = signature;
                }
            }
            _ => unreachable!("member sync signatures only occur on traits and impls"),
        }
    }
}

/// Resolve `#[sync_attr(...)]` / `#[async_attr(...)]` passthrough attributes: the matching variant's payload is unwrapped into a real attribute, the other variant's is dropped.
fn passthrough_attrs(attrs: &mut Vec<Attribute>, variant: Variant) -> syn::Result<()> {
    let mut out = Vec::with_capacity(attrs.len());
    for attr in attrs.drain(..) {
        let path = attr.path();
        let which = if path.is_ident("sync_attr") || is_ambisync_path(path, "sync_attr") {
            Some(Variant::Sync)
        } else if path.is_ident("async_attr") || is_ambisync_path(path, "async_attr") {
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

fn append_doc(attrs: &mut Vec<Attribute>, extra: &LitStr) {
    if attrs
        .iter()
        .any(|attr| matches!(&attr.meta, Meta::NameValue(nv) if nv.path.is_ident("doc")))
    {
        attrs.push(parse_quote!(#[doc = ""]));
    }
    attrs.push(parse_quote!(#[doc = #extra]));
}

fn async_trait_attrs(cfg: &Config) -> Vec<Attribute> {
    match cfg.send_policy() {
        SendPolicy::Always => vec![parse_quote!(#[async_trait::async_trait])],
        SendPolicy::Never => vec![parse_quote!(#[async_trait::async_trait(?Send)])],
        SendPolicy::Cfg(predicate) => {
            let negated = negate_cfg_predicate(&predicate);
            vec![
                parse_quote!(#[cfg_attr(#predicate, async_trait::async_trait)]),
                parse_quote!(#[cfg_attr(#negated, async_trait::async_trait(?Send))]),
            ]
        }
    }
}

/// Negate a cfg predicate, unwrapping a top-level `not(...)` instead of stacking `not(not(...))`.
fn negate_cfg_predicate(predicate: &TokenStream) -> TokenStream {
    if let Ok(Meta::List(list)) = syn::parse2::<Meta>(predicate.clone()) {
        if list.path.is_ident("not") {
            return list.tokens;
        }
    }
    quote!(not(#predicate))
}

fn ensure_distinct(sync: &Ident, async_: &Ident) -> syn::Result<()> {
    if sync == async_ {
        return Err(syn::Error::new(
            sync.span(),
            format!(
                "the sync and async variants would both be named `{sync}`; choose a distinct \
                 `sync(name = ...)` or `async(name = ...)`"
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use quote::quote;

    fn expand(args: proc_macro2::TokenStream, input: proc_macro2::TokenStream) -> String {
        super::ambisync(args, input)
            .expect("expansion succeeds")
            .to_string()
    }

    #[test]
    fn sync_docs_follow_substitutions() {
        let out = expand(
            quote! {
                sync(
                    name = "read",
                    types(AsyncStore => Store, mod_x::AsyncThing => mod_y::Thing),
                    fns(async_helper => helper),
                ),
                async(feature = "async"),
            },
            quote! {
                /// Reads an [`AsyncStore`] and a [`mod_x::AsyncThing`], using
                /// [`async_helper`]; see also [`async_read`], `AsyncStore::new`,
                /// `AsyncStoreExt` and [`other::AsyncStore`].
                async fn async_read(store: &AsyncStore) -> u32 {
                    async_helper(store).await
                }
            },
        );
        assert!(out.contains("[`Store`] and a [`mod_y::Thing`]"));
        assert!(out.contains("[`helper`]"));
        assert!(out.contains("[`read`]"));
        assert!(out.contains("`Store::new`"));
        // Longer identifiers and longer paths are not partial-matched.
        assert!(out.contains("`AsyncStoreExt`"));
        assert!(out.contains("[`other::AsyncStore`]"));
        // The async variant keeps the source docs verbatim.
        assert!(out.contains("[`AsyncStore`] and a [`mod_x::AsyncThing`]"));
    }

    #[test]
    fn async_docs_follow_the_async_rename() {
        let out = expand(
            quote! {
                sync(name = "read"),
                async(feature = "async", name = "async_read"),
            },
            quote! {
                /// See [`internal_read`].
                async fn internal_read() -> u32 {
                    1
                }
            },
        );
        assert!(out.contains("[`read`]"));
        assert!(out.contains("[`async_read`]"));
        assert!(!out.contains("internal_read"));
    }

    #[test]
    fn pattern_entries_substitute_types_and_calls() {
        let out = expand(
            quote! {
                sync(
                    types("Async{}"),
                    fns("{}_async", "create_async_{} => create_{}"),
                ),
                async(feature = "async"),
            },
            quote! {
                /// Reads an [`AsyncStore`] via [`read_async`], [`helper_async`] and
                /// [`create_async_store`]; `MyAsyncStore` is untouched.
                async fn read_async(store: &AsyncStore) -> Vec<AsyncItem> {
                    let created: AsyncStore = create_async_store().await;
                    let value = helper_async().await;
                    assert_eq!(items_async(&created, value).await.len(), value);
                    items_async(&created, value).await
                }
            },
        );
        // Signature: fn name via the `fns("{}_async")` pattern, types via `types("Async{}")`.
        assert!(
            out.contains("fn read (store : & Store) -> Vec < Item >"),
            "{out}"
        );
        // Calls: suffix pattern, arrow pattern, and calls inside macro tokens.
        assert!(out.contains("create_store ()"), "{out}");
        assert!(out.contains("helper ()"), "{out}");
        assert!(out.contains("items (& created , value)"), "{out}");
        // Docs follow the pattern renames at identifier boundaries.
        assert!(out.contains("[`Store`]"), "{out}");
        assert!(out.contains("[`read`]"), "{out}");
        assert!(out.contains("[`helper`]"), "{out}");
        assert!(out.contains("[`create_store`]"), "{out}");
        assert!(out.contains("`MyAsyncStore`"), "{out}");
        // The async variant is untouched.
        assert!(
            out.contains("async fn read_async (store : & AsyncStore)"),
            "{out}"
        );
    }

    #[test]
    fn closures_returning_async_blocks_become_plain_closures() {
        let out = expand(
            quote! { sync(name = "total"), async(feature = "async") },
            quote! {
                async fn async_total(values: Vec<u32>) -> u32 {
                    let first = || async { 1u32 };
                    let rest = || async move { values.iter().sum::<u32>() };
                    let checked = || async {
                        if false {
                            return 0u32;
                        }
                        2u32
                    };
                    first().await + rest().await + checked().await
                }
            },
        );
        assert!(out.contains("let first = | | { 1u32 } ;"), "{out}");
        assert!(
            out.contains("let rest = move | | { values . iter () . sum :: < u32 > () } ;"),
            "{out}"
        );
        assert!(
            out.contains("let checked = | | { if false { return 0u32 ; } 2u32 } ;"),
            "{out}"
        );
        assert!(out.contains("first () + rest () + checked ()"), "{out}");
    }

    #[test]
    fn exact_entries_win_over_patterns() {
        let out = expand(
            quote! {
                sync(
                    name = "read",
                    types("Async{}", AsyncStore => BlockingStore),
                    fns("async_{}", async_helper => blocking_helper),
                ),
                async(feature = "async"),
            },
            quote! {
                async fn async_read(store: &AsyncStore, task: &AsyncTask) -> u32 {
                    async_helper(store).await + async_other(task).await
                }
            },
        );
        assert!(out.contains("store : & BlockingStore"), "{out}");
        assert!(out.contains("task : & Task"), "{out}");
        assert!(out.contains("blocking_helper (store)"), "{out}");
        assert!(out.contains("other (task)"), "{out}");
    }

    #[test]
    fn signature_only_impl_members_are_rewritten() {
        // `pub async fn f(...);` members inside impls (syn: `ImplItem::Verbatim`, meaningful to
        // attribute macros like `inherent`) receive the same filtering, renaming, and rewriting
        // as ordinary methods, and stay body-less in both expansions.
        let out = expand(
            quote! {
                sync(fns("async_{}"), types(AsyncStore => Store, AsyncReadOps => ReadOps)),
                async(feature = "async"),
            },
            quote! {
                impl AsyncReadOps for Cached<AsyncStore> {
                    /// Retrieve via [`AsyncStore`].
                    #[allow(clippy::missing_errors_doc)]
                    pub async fn async_retrieve(&self) -> Result<u32, AsyncStore>;

                    pub async unsafe fn async_scary<T: Send>(&self, x: T) -> u32;

                    #[sync_only]
                    pub async fn async_blocking(&self) -> u32;

                    #[async_only]
                    pub async fn async_concurrent(&self) -> u32;
                }
            },
        );
        // Sync expansion: renamed, `async` stripped, types substituted, docs follow, `;` form kept.
        assert!(
            out.contains("pub fn retrieve (& self) -> Result < u32 , Store > ;"),
            "{out}"
        );
        assert!(
            out.contains("pub unsafe fn scary < T : Send > (& self , x : T) -> u32 ;"),
            "{out}"
        );
        assert!(out.contains("[`Store`]"), "{out}");
        assert!(out.contains("impl ReadOps for Cached < Store >"), "{out}");
        // Async expansion: kept as written.
        assert!(
            out.contains("pub async fn async_retrieve (& self) -> Result < u32 , AsyncStore > ;"),
            "{out}"
        );
        // `#[sync_only]` / `#[async_only]` gate signature-only members like other members.
        assert!(out.contains("fn blocking (& self)"), "{out}");
        assert!(!out.contains("fn async_blocking"), "{out}");
        assert!(out.contains("fn async_concurrent (& self)"), "{out}");
        assert!(!out.contains("fn concurrent"), "{out}");
        // No internal marker leaks into the output.
        assert!(!out.contains("__ambisync_signature_only"), "{out}");
    }

    #[test]
    fn send_policies_control_async_trait_emission() {
        let expand_with = |async_options: proc_macro2::TokenStream| {
            expand(
                quote! {
                    sync(name = "Value", fns("async_{}")),
                    async(flavor = async_trait, #async_options),
                },
                quote! {
                    trait AsyncValue {
                        async fn async_value(&self) -> u32;
                    }
                },
            )
        };

        let default = expand_with(quote!());
        assert!(default.contains(&quote!(#[async_trait::async_trait]).to_string()));
        assert_eq!(default, expand_with(quote!(send = true)));

        let never = expand_with(quote!(send = false));
        assert!(never.contains(&quote!(#[async_trait::async_trait(?Send)]).to_string()));

        let conditional = expand_with(quote!(send = cfg(not(target_arch = "wasm32"))));
        assert!(conditional.contains(
            &quote!(#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]).to_string()
        ));
        assert!(conditional.contains(
            &quote!(#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))])
                .to_string()
        ));
    }
}
