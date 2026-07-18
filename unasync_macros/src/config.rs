//! Attribute-argument parsing for `#[unasync::dual]`, `#[unasync::test]`,
//! `#[unasync::scope]` and `unasync::content!`.

use proc_macro2::TokenStream;
use syn::parse::discouraged::Speculative;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Ident, Item, ItemEnum, ItemImpl, ItemStruct, ItemTrait, LitBool, LitStr, Signature,
    Token, Type, Visibility, braced, parenthesized,
};

/// How the async variant of a trait / impl is emitted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Flavor {
    /// Native `async fn` (with `#[allow(async_fn_in_trait)]` on traits).
    #[default]
    AsyncFn,
    /// `#[async_trait::async_trait]`, optionally with the wasm `?Send` pairing.
    AsyncTrait,
}

/// One `sync(types(AsyncSpelling => SyncSpelling))` entry. The left side is the
/// spelling used in the (async-form) source; it is matched syntactically
/// (token-for-token, after normalization) against every type position in the
/// derived sync expansion and replaced by the right side.
#[derive(Clone)]
pub struct TypeSubst {
    pub async_: Type,
    pub sync: Type,
}

impl Parse for TypeSubst {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let async_: Type = input.parse()?;
        input.parse::<Token![=>]>()?;
        let sync: Type = input.parse()?;
        Ok(Self { async_, sync })
    }
}

/// One `sync(calls(async_name => sync_name))` entry. In the derived sync expansion,
/// calls/method-calls named `async_name` are renamed to `sync_name` (the
/// `.await` on the call was written in the source and is stripped anyway).
#[derive(Clone)]
pub struct CallSwap {
    pub async_: Ident,
    pub sync: Ident,
}

/// A complete, body-less Rust function declaration supplied for one variant.
#[derive(Clone)]
pub struct FnDeclaration {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub sig: Signature,
}

impl Parse for FnDeclaration {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis = input.parse()?;
        let sig = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(Self { attrs, vis, sig })
    }
}

/// An empty Rust item shell supplied for one variant. Its attributes and
/// header are emitted exactly as written; rewritten contents from the
/// annotated item are attached after rewriting.
#[derive(Clone)]
pub enum Declaration {
    Fn(FnDeclaration),
    Trait(ItemTrait),
    Impl(ItemImpl),
    Struct(ItemStruct),
    Enum(ItemEnum),
}

impl Declaration {
    pub fn ident(&self) -> Option<&Ident> {
        match self {
            Self::Fn(declaration) => Some(&declaration.sig.ident),
            Self::Trait(declaration) => Some(&declaration.ident),
            Self::Impl(_) => None,
            Self::Struct(declaration) => Some(&declaration.ident),
            Self::Enum(declaration) => Some(&declaration.ident),
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Fn(_) => "function",
            Self::Trait(_) => "trait",
            Self::Impl(_) => "impl",
            Self::Struct(_) => "struct",
            Self::Enum(_) => "enum",
        }
    }

    pub fn span(&self) -> proc_macro2::Span {
        match self {
            Self::Fn(declaration) => declaration.sig.span(),
            Self::Trait(declaration) => declaration.span(),
            Self::Impl(declaration) => declaration.span(),
            Self::Struct(declaration) => declaration.span(),
            Self::Enum(declaration) => declaration.span(),
        }
    }
}

impl Parse for CallSwap {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let async_: Ident = input.parse()?;
        input.parse::<Token![=>]>()?;
        let sync: Ident = input.parse()?;
        if input.peek(Token![.]) {
            return Err(input.error(
                "call swaps map names only: the source is written in async form, so write the \
                 `.await` on the call itself; it is stripped in the sync expansion",
            ));
        }
        Ok(Self { async_, sync })
    }
}

#[derive(Clone, Default)]
pub struct SyncConfig {
    /// Feature gating the sync expansion.
    pub gate: Option<LitStr>,
    /// Exact name for the annotated sync item.
    pub name: Option<Ident>,
    /// Pattern used to derive sync function/method names from async names.
    pub pattern: Option<LitStr>,
    /// Pattern used to derive sync type/trait names from async names.
    pub type_pattern: Option<LitStr>,
    /// Extra documentation appended to the sync variant.
    pub doc: Option<LitStr>,
    /// Attributes appended to the sync variant.
    pub attrs: Vec<Attribute>,
    /// Attribute(s) placed on the sync test variant (default `#[test]`).
    pub test_attr: Option<Vec<Attribute>>,
    /// Complete declaration shell for a generated sync item.
    pub declaration: Option<Declaration>,
    /// Async-to-sync type substitutions.
    pub types: Vec<TypeSubst>,
    /// Async-to-sync call substitutions.
    pub calls: Vec<CallSwap>,
}

#[derive(Clone, Default)]
pub struct AsyncConfig {
    /// Feature gating the async expansion.
    pub gate: Option<LitStr>,
    /// Exact name for the annotated async item. The source name is used when omitted.
    pub name: Option<Ident>,
    /// Extra documentation appended to the async variant.
    pub doc: Option<LitStr>,
    /// Attributes appended to the async variant.
    pub attrs: Vec<Attribute>,
    /// Attribute(s) placed on the async test variant (default `#[tokio::test]`).
    pub test_attr: Option<Vec<Attribute>>,
    /// Complete declaration shell for a generated async item.
    pub declaration: Option<Declaration>,
    /// Async trait/impl emission style. `None` means the default `async_fn` flavor.
    pub flavor: Option<Flavor>,
    /// Whether `async_trait` gets the native/wasm `?Send` cfg pairing.
    pub wasm_send: Option<bool>,
}

#[derive(Clone, Default)]
pub struct Config {
    pub sync: SyncConfig,
    pub async_: AsyncConfig,
}

impl Parse for Config {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut cfg = Config::default();
        while !input.is_empty() {
            // `async(...)` first: `async` is a keyword, not an `Ident`.
            if input.peek(Token![async]) {
                input.parse::<Token![async]>()?;
                let content;
                parenthesized!(content in input);
                parse_async_config(&content, &mut cfg.async_)?;
                if !input.is_empty() {
                    input.parse::<Token![,]>()?;
                }
                continue;
            }
            let key: Ident = input.parse()?;
            match key.to_string().as_str() {
                "sync" => {
                    let content;
                    parenthesized!(content in input);
                    parse_sync_config(&content, &mut cfg.sync)?;
                }
                "types" => return Err(moved_substitution_error(&key, "types")),
                "calls" => return Err(moved_substitution_error(&key, "calls")),
                "keep_self" => {
                    return Err(syn::Error::new(
                        key.span(),
                        "`keep_self` has been removed; use `sync(pattern = \"{}\")` for \
                         same-named methods (top-level sync and async items must have distinct names)",
                    ));
                }
                "rename" | "rename_types" | "async_doc" | "async_attr" | "flavor" | "wasm_send"
                | "feature" | "sync_feature" => {
                    return Err(legacy_option_error(&key));
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown option `{key}`; expected `sync(...)` or `async(...)`"),
                    ));
                }
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(cfg)
    }
}

fn parse_sync_config(input: ParseStream, cfg: &mut SyncConfig) -> syn::Result<()> {
    while !input.is_empty() {
        let key: Ident = input.parse()?;
        match key.to_string().as_str() {
            "feature" => set_once(&mut cfg.gate, parse_value(input)?, &key)?,
            "name" => set_once(&mut cfg.name, parse_name(input)?, &key)?,
            "pattern" => set_once(&mut cfg.pattern, parse_pattern_value(input)?, &key)?,
            "type_pattern" => {
                set_once(&mut cfg.type_pattern, parse_pattern_value(input)?, &key)?;
            }
            "doc" => set_once(&mut cfg.doc, parse_value(input)?, &key)?,
            "attr" => cfg.attrs.extend(parse_attribute_value(input)?),
            "test_attr" => {
                let attrs = parse_attribute_value(input)?;
                set_once(&mut cfg.test_attr, attrs, &key)?;
            }
            "declaration" => {
                let declaration = parse_declaration(input, false)?;
                set_once(&mut cfg.declaration, declaration, &key)?;
            }
            "types" => {
                let content;
                parenthesized!(content in input);
                let entries = Punctuated::<TypeSubst, Token![,]>::parse_terminated(&content)?;
                cfg.types.extend(entries);
            }
            "calls" => {
                let content;
                parenthesized!(content in input);
                let entries = Punctuated::<CallSwap, Token![,]>::parse_terminated(&content)?;
                cfg.calls.extend(entries);
            }
            _ => {
                return Err(syn::Error::new(
                    key.span(),
                    "unknown sync option; expected `feature`, `name`, `pattern`, `type_pattern`, \
                     `doc`, `attr`, `test_attr`, `declaration`, `types` or `calls`",
                ));
            }
        }
        parse_optional_comma(input)?;
    }
    Ok(())
}

fn parse_async_config(input: ParseStream, cfg: &mut AsyncConfig) -> syn::Result<()> {
    while !input.is_empty() {
        let key: Ident = input.parse()?;
        match key.to_string().as_str() {
            "feature" => set_once(&mut cfg.gate, parse_value(input)?, &key)?,
            "name" => set_once(&mut cfg.name, parse_name(input)?, &key)?,
            "doc" => set_once(&mut cfg.doc, parse_value(input)?, &key)?,
            "attr" => cfg.attrs.extend(parse_attribute_value(input)?),
            "test_attr" => {
                let attrs = parse_attribute_value(input)?;
                set_once(&mut cfg.test_attr, attrs, &key)?;
            }
            "declaration" => {
                let declaration = parse_declaration(input, true)?;
                set_once(&mut cfg.declaration, declaration, &key)?;
            }
            "flavor" => {
                input.parse::<Token![=]>()?;
                let flavor: Ident = input.parse()?;
                let flavor = parse_flavor(flavor)?;
                set_once(&mut cfg.flavor, flavor, &key)?;
            }
            "wasm_send" => {
                let enabled = if input.peek(Token![=]) {
                    input.parse::<Token![=]>()?;
                    input.parse::<LitBool>()?.value
                } else {
                    true
                };
                set_once(&mut cfg.wasm_send, enabled, &key)?;
            }
            _ => {
                return Err(syn::Error::new(
                    key.span(),
                    "unknown async option; expected `feature`, `name`, `doc`, `attr`, \
                     `test_attr`, `declaration`, `flavor` or `wasm_send`",
                ));
            }
        }
        parse_optional_comma(input)?;
    }
    Ok(())
}

fn parse_attribute_value(input: ParseStream) -> syn::Result<Vec<Attribute>> {
    input.parse::<Token![=]>()?;
    let attrs = input.call(Attribute::parse_outer)?;
    if attrs.is_empty() {
        return Err(input.error("expected an attribute, e.g. `#[inline]`"));
    }
    Ok(attrs)
}

fn parse_declaration(input: ParseStream, expect_async: bool) -> syn::Result<Declaration> {
    let content;
    braced!(content in input);
    let fork = content.fork();
    match fork.parse::<Item>() {
        Ok(Item::Verbatim(_)) | Err(_) => {
            let declaration: FnDeclaration = content.parse()?;
            if !content.is_empty() {
                return Err(content.error("unexpected tokens after function declaration"));
            }
            validate_function_declaration(&declaration, expect_async)?;
            Ok(Declaration::Fn(declaration))
        }
        Ok(item) => {
            if !fork.is_empty() {
                return Err(fork.error("a declaration must contain exactly one item shell"));
            }
            content.advance_to(&fork);
            declaration_from_item(item, expect_async)
        }
    }
}

fn declaration_from_item(item: Item, expect_async: bool) -> syn::Result<Declaration> {
    match item {
        Item::Fn(item) => {
            if !item.block.stmts.is_empty() {
                return Err(syn::Error::new_spanned(
                    &item.block,
                    "a function declaration shell must have an empty body",
                ));
            }
            let declaration = FnDeclaration {
                attrs: item.attrs,
                vis: item.vis,
                sig: item.sig,
            };
            validate_function_declaration(&declaration, expect_async)?;
            Ok(Declaration::Fn(declaration))
        }
        Item::Trait(item) => {
            if !item.items.is_empty() {
                return Err(syn::Error::new_spanned(
                    &item.items[0],
                    "a trait declaration shell must have an empty body",
                ));
            }
            Ok(Declaration::Trait(item))
        }
        Item::Impl(item) => {
            if !item.items.is_empty() {
                return Err(syn::Error::new_spanned(
                    &item.items[0],
                    "an impl declaration shell must have an empty body",
                ));
            }
            Ok(Declaration::Impl(item))
        }
        Item::Struct(item) => {
            if !item.fields.is_empty() {
                return Err(syn::Error::new_spanned(
                    &item.fields,
                    "a struct declaration shell must have no fields",
                ));
            }
            Ok(Declaration::Struct(item))
        }
        Item::Enum(item) => {
            if !item.variants.is_empty() {
                return Err(syn::Error::new_spanned(
                    &item.variants[0],
                    "an enum declaration shell must have no variants",
                ));
            }
            Ok(Declaration::Enum(item))
        }
        other => Err(syn::Error::new_spanned(
            other,
            "a declaration must be a function, trait, impl, struct or enum shell",
        )),
    }
}

fn validate_function_declaration(
    declaration: &FnDeclaration,
    expect_async: bool,
) -> syn::Result<()> {
    if expect_async && declaration.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            &declaration.sig,
            "an `async(declaration { ... })` function must be `async fn`",
        ));
    }
    if !expect_async && declaration.sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            &declaration.sig,
            "a `sync(declaration { ... })` function must not be `async fn`",
        ));
    }
    Ok(())
}

fn moved_substitution_error(key: &Ident, option: &str) -> syn::Error {
    syn::Error::new(
        key.span(),
        format!("`{option}(...)` is sync-specific; move it inside `sync({option}(...))`"),
    )
}

fn parse_flavor(flavor: Ident) -> syn::Result<Flavor> {
    match flavor.to_string().as_str() {
        "async_fn" => Ok(Flavor::AsyncFn),
        "async_trait" => Ok(Flavor::AsyncTrait),
        "boxed" => Err(syn::Error::new(
            flavor.span(),
            "`flavor = boxed` is not implemented yet; use `async_fn` or `async_trait`",
        )),
        _ => Err(syn::Error::new(
            flavor.span(),
            "unknown flavor; expected `async_fn` or `async_trait`",
        )),
    }
}

fn parse_value<T: Parse>(input: ParseStream) -> syn::Result<T> {
    input.parse::<Token![=]>()?;
    input.parse()
}

fn parse_name(input: ParseStream) -> syn::Result<Ident> {
    let lit: LitStr = parse_value(input)?;
    ident_from_str(&lit.value(), lit.span())
}

fn parse_pattern_value(input: ParseStream) -> syn::Result<LitStr> {
    let lit: LitStr = parse_value(input)?;
    validate_pattern(&lit)?;
    Ok(lit)
}

fn parse_optional_comma(input: ParseStream) -> syn::Result<()> {
    if !input.is_empty() {
        input.parse::<Token![,]>()?;
    }
    Ok(())
}

fn set_once<T>(slot: &mut Option<T>, value: T, key: &Ident) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new(
            key.span(),
            format!("duplicate `{key}` option"),
        ));
    }
    *slot = Some(value);
    Ok(())
}

fn legacy_option_error(key: &Ident) -> syn::Error {
    let replacement = match key.to_string().as_str() {
        "rename" => "`sync(pattern = \"...\")`",
        "rename_types" => "`sync(type_pattern = \"...\")`",
        "async_doc" => "`async(doc = \"...\")`",
        "async_attr" => "`async(attr = #[...])`",
        "flavor" => "`async(flavor = ...)`",
        "wasm_send" => "`async(wasm_send)`",
        _ => "`sync(feature = \"...\")` / `async(feature = \"...\")`",
    };
    syn::Error::new(
        key.span(),
        format!("`{key}` is now variant-specific; use {replacement}"),
    )
}

fn validate_pattern(lit: &LitStr) -> syn::Result<()> {
    if lit.value().matches("{}").count() != 1 {
        return Err(syn::Error::new(
            lit.span(),
            "name pattern must contain exactly one `{}` placeholder, e.g. `async_{}` or `{}_async`",
        ));
    }
    Ok(())
}

impl Config {
    pub fn parse_args(args: TokenStream) -> syn::Result<Self> {
        syn::parse2(args)
    }

    /// Merge an item-level config over shared defaults (from `scope`/`content!`).
    /// Item-level values win; substitution tables are concatenated with the
    /// item-level entries first (first match wins during rewriting).
    pub fn merged_over(mut self, defaults: &Config) -> Config {
        merge_option(&mut self.sync.gate, &defaults.sync.gate);
        merge_option(&mut self.sync.name, &defaults.sync.name);
        merge_option(&mut self.sync.pattern, &defaults.sync.pattern);
        merge_option(&mut self.sync.type_pattern, &defaults.sync.type_pattern);
        merge_option(&mut self.sync.doc, &defaults.sync.doc);
        merge_option(&mut self.sync.test_attr, &defaults.sync.test_attr);
        merge_option(&mut self.sync.declaration, &defaults.sync.declaration);
        self.sync.attrs.extend(defaults.sync.attrs.iter().cloned());
        self.sync.types.extend(defaults.sync.types.iter().cloned());
        self.sync.calls.extend(defaults.sync.calls.iter().cloned());
        merge_option(&mut self.async_.gate, &defaults.async_.gate);
        merge_option(&mut self.async_.name, &defaults.async_.name);
        merge_option(&mut self.async_.doc, &defaults.async_.doc);
        merge_option(&mut self.async_.test_attr, &defaults.async_.test_attr);
        merge_option(&mut self.async_.declaration, &defaults.async_.declaration);
        merge_option(&mut self.async_.flavor, &defaults.async_.flavor);
        merge_option(&mut self.async_.wasm_send, &defaults.async_.wasm_send);
        self.async_
            .attrs
            .extend(defaults.async_.attrs.iter().cloned());
        self
    }

    pub fn flavor(&self) -> Flavor {
        self.async_.flavor.unwrap_or_default()
    }

    pub fn wasm_send(&self) -> bool {
        self.async_.wasm_send.unwrap_or(false)
    }

    pub fn validate_effective(&self) -> syn::Result<()> {
        if self.wasm_send() && self.flavor() != Flavor::AsyncTrait {
            let span = self
                .async_
                .name
                .as_ref()
                .map_or_else(proc_macro2::Span::call_site, Ident::span);
            return Err(syn::Error::new(
                span,
                "`async(wasm_send)` requires `async(flavor = async_trait)`",
            ));
        }
        validate_declaration_options(
            "sync",
            self.sync.declaration.as_ref(),
            self.sync.name.as_ref(),
            self.sync.doc.as_ref(),
            &self.sync.attrs,
        )?;
        validate_declaration_options(
            "async",
            self.async_.declaration.as_ref(),
            self.async_.name.as_ref(),
            self.async_.doc.as_ref(),
            &self.async_.attrs,
        )?;
        Ok(())
    }
}

fn validate_declaration_options(
    variant: &str,
    declaration: Option<&Declaration>,
    name: Option<&Ident>,
    doc: Option<&LitStr>,
    attrs: &[Attribute],
) -> syn::Result<()> {
    let Some(declaration) = declaration else {
        return Ok(());
    };
    let conflicting = if name.is_some() {
        Some("name")
    } else if doc.is_some() {
        Some("doc")
    } else if !attrs.is_empty() {
        Some("attr")
    } else {
        None
    };
    if let Some(option) = conflicting {
        return Err(syn::Error::new(
            declaration.span(),
            format!(
                "`{variant}(declaration {{ ... }})` owns the item name, docs and item \
                 attributes, so it cannot be combined with `{variant}({option} = ...)`"
            ),
        ));
    }
    Ok(())
}

fn merge_option<T: Clone>(item: &mut Option<T>, default: &Option<T>) {
    if item.is_none() {
        *item = default.clone();
    }
}

/// Derive the sync name from an async (source) name by matching it against a
/// rename pattern: `unapply_pattern("async_{}", async_read)` → `read`.
pub fn unapply_pattern(pattern: &str, ident: &Ident) -> syn::Result<Option<Ident>> {
    let Some((prefix, suffix)) = pattern.split_once("{}") else {
        return Ok(None);
    };
    let name = ident.to_string();
    let Some(core) = name
        .strip_prefix(prefix)
        .and_then(|name| name.strip_suffix(suffix))
    else {
        return Ok(None);
    };
    if core.is_empty() {
        return Ok(None);
    }
    ident_from_str(core, ident.span()).map(Some)
}

/// Compute the sync ident for a source (async) ident, erroring with guidance
/// when no pattern was configured or the name does not match it.
pub fn sync_ident(
    exact: Option<&Ident>,
    pattern: Option<&LitStr>,
    which: &str,
    async_ident: &Ident,
) -> syn::Result<Ident> {
    if let Some(exact) = exact {
        return Ok(exact.clone());
    }
    let Some(pattern) = pattern else {
        return Err(syn::Error::new(
            async_ident.span(),
            format!(
                "cannot derive the sync name of `{async_ident}`: add `sync(name = \"...\")` \
                 for this item or `sync({which} = \"...\")` (for example \
                 `sync({which} = \"async_{{}}\")`)"
            ),
        ));
    };
    let value = pattern.value();
    unapply_pattern(&value, async_ident)?.ok_or_else(|| {
        syn::Error::new(
            async_ident.span(),
            format!("`{async_ident}` does not match the `sync({which} = ...)` pattern `{value}`"),
        )
    })
}

pub fn async_ident(configured: Option<&Ident>, source: &Ident) -> Ident {
    configured.cloned().unwrap_or_else(|| source.clone())
}

fn ident_from_str(value: &str, span: proc_macro2::Span) -> syn::Result<Ident> {
    let mut ident = syn::parse_str::<Ident>(value)
        .map_err(|_| syn::Error::new(span, format!("`{value}` is not a valid Rust identifier")))?;
    ident.set_span(span);
    Ok(ident)
}
