//! Item-level expansion: emit the async variant (the source, minus helper
//! attributes and resolved markers) and the derived sync variant, plus the
//! `scope`/`scoped!` shared-defaults wrappers.

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::visit_mut::VisitMut;
use syn::{
    AttrStyle, Attribute, Fields, Ident, ImplItem, Item, ItemMod, LitStr, Meta, TraitItem,
    parse_quote,
};

use crate::config::{Config, Declaration, Flavor, SendPolicy, async_ident, sync_ident};
use crate::rewrite::{Rewriter, Variant};
use crate::subst::SubstTable;

pub fn ambisync(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
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
        if let Some(declaration) = declaration
            && declaration.kind_name() != expected
        {
            return Err(syn::Error::new(
                declaration.span(),
                format!(
                    "`{variant}(declaration {{ ... }})` for this item must declare a {expected}, \
                     not a {}",
                    declaration.kind_name()
                ),
            ));
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
                sync_ident(
                    cfg.sync.name.as_ref(),
                    cfg.sync.pattern.as_ref(),
                    "pattern",
                    &async_name,
                )?
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
                sync_ident(
                    cfg.sync.name.as_ref(),
                    cfg.sync.type_pattern.as_ref(),
                    "type_pattern",
                    &async_trait_ident,
                )?
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
                    if method.sig.asyncness.is_none() {
                        continue;
                    }
                    if variant.is_sync() {
                        method.sig.asyncness = None;
                        let sync_name = sync_ident(
                            None,
                            cfg.sync.pattern.as_ref(),
                            "pattern",
                            &method.sig.ident,
                        )?;
                        table.add_auto_call(&method.sig.ident, &sync_name);
                        method.sig.ident = sync_name;
                    }
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
                     use `sync(types(...))` for its trait/self types and `sync(pattern = ...)` \
                     for methods",
                ));
            }
            passthrough_attrs(&mut item_impl.attrs, variant)?;
            filter_impl_items(&mut item_impl.items, variant);
            for impl_item in &mut item_impl.items {
                if let ImplItem::Fn(method) = impl_item {
                    passthrough_attrs(&mut method.attrs, variant)?;
                    if method.sig.asyncness.is_none() {
                        continue;
                    }
                    if variant.is_sync() {
                        method.sig.asyncness = None;
                        let sync_name = sync_ident(
                            None,
                            cfg.sync.pattern.as_ref(),
                            "pattern",
                            &method.sig.ident,
                        )?;
                        table.add_auto_call(&method.sig.ident, &sync_name);
                        method.sig.ident = sync_name;
                    }
                }
            }
            if variant.is_sync()
                && let Some((_, trait_path, _)) = &mut item_impl.trait_
            {
                table.subst_path(trait_path, false);
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
                sync_ident(
                    cfg.sync.name.as_ref(),
                    cfg.sync.type_pattern.as_ref(),
                    "type_pattern",
                    &async_name,
                )?
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
                sync_ident(
                    cfg.sync.name.as_ref(),
                    cfg.sync.type_pattern.as_ref(),
                    "type_pattern",
                    &async_name,
                )?
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

    apply_variant_metadata(&mut item, cfg, variant, is_test)?;

    let gate = if variant.is_sync() {
        &cfg.sync.gate
    } else {
        &cfg.async_.gate
    };
    if let Some(feature) = gate
        && let Some(attrs) = item_attrs_mut(&mut item)
    {
        attrs.insert(0, parse_quote!(#[cfg(feature = #feature)]));
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

/// Handle a `#[sync_only]`/`#[async_only]` gating attribute on a field, enum
/// variant, or trait/impl item: returns whether the element is kept in this
/// variant, removing the marker attribute either way.
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

/// Negate a cfg predicate, unwrapping a top-level `not(...)` instead of
/// stacking `not(not(...))`.
fn negate_cfg_predicate(predicate: &TokenStream) -> TokenStream {
    if let Ok(Meta::List(list)) = syn::parse2::<Meta>(predicate.clone())
        && list.path.is_ident("not")
    {
        return list.tokens;
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

    /// Sync doc comments receive the same substitutions as code: `types(...)`
    /// (including module-qualified paths), `calls(...)`, and the item's own
    /// rename. Matching stops at identifier boundaries and path prefixes.
    #[test]
    fn sync_docs_follow_substitutions() {
        let out = expand(
            quote! {
                sync(
                    name = "read",
                    types(AsyncStore => Store, mod_x::AsyncThing => mod_y::Thing),
                    calls(async_helper => helper),
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

    /// An exact async output name rewrites doc self-references in the async
    /// variant, mirroring the code rewrite of recursive calls.
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

    /// The three `async(send = ...)` policies control `async_trait` emission:
    /// plain (default / `true`), `?Send` (`false`), and the `cfg_attr` pairing
    /// (`cfg(PRED)`), with a top-level `not(...)` predicate unwrapped rather
    /// than double-negated.
    #[test]
    fn send_policies_control_async_trait_emission() {
        let expand_with = |async_options: proc_macro2::TokenStream| {
            expand(
                quote! {
                    sync(name = "Value", pattern = "async_{}"),
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
        assert!(
            conditional.contains(
                &quote!(#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))])
                    .to_string()
            )
        );
    }
}
