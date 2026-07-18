//! Attribute-argument parsing for `#[ambisync]`, `#[ambisync::test]`,
//! `#[ambisync::scope]` and `ambisync::scoped!`.

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
    /// `#[async_trait::async_trait]`, with `Send`-ness set by `async(send = ...)`.
    AsyncTrait,
}

/// `Send`-ness of `async_trait`-flavored emission: `async(send = ...)`.
#[derive(Clone, Default)]
pub enum SendPolicy {
    /// `send = true`: plain `#[async_trait::async_trait]` (the default).
    #[default]
    Always,
    /// `send = false`: `#[async_trait::async_trait(?Send)]`.
    Never,
    /// `send = cfg(PRED)`: `Send` where `PRED` holds, `?Send` elsewhere.
    Cfg(TokenStream),
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

/// A `sync(pattern = ...)` / `sync(type_pattern = ...)` rename pattern.
///
/// The short form names the async spelling and captures the sync name:
/// `"async_{}"` maps `async_read` to `read`. The full form maps the async
/// spelling to a sync spelling built from the same `{}` stem:
/// `"async_{} => sync_{}"` maps `async_read` to `sync_read`. The short form
/// is sugar for `"... => {}"`.
#[derive(Clone)]
pub struct RenamePattern {
    lit: LitStr,
    async_prefix: String,
    async_suffix: String,
    sync_prefix: String,
    sync_suffix: String,
}

impl RenamePattern {
    pub fn parse(lit: LitStr) -> syn::Result<Self> {
        let value = lit.value();
        let (async_side, sync_side) = match value.split_once("=>") {
            Some((async_side, sync_side)) if !sync_side.contains("=>") => {
                (async_side.trim(), sync_side.trim())
            }
            Some(_) => {
                return Err(syn::Error::new(
                    lit.span(),
                    "a rename pattern has at most one `=>`",
                ));
            }
            None => (value.trim(), "{}"),
        };
        let (async_prefix, async_suffix) = split_pattern_side(&lit, async_side)?;
        let (sync_prefix, sync_suffix) = split_pattern_side(&lit, sync_side)?;
        Ok(Self {
            lit,
            async_prefix,
            async_suffix,
            sync_prefix,
            sync_suffix,
        })
    }

    /// The sync ident derived from an async (source) ident, or `None` when
    /// the ident does not match the pattern's async side.
    pub fn sync_name(&self, async_ident: &Ident) -> syn::Result<Option<Ident>> {
        let name = async_ident.to_string();
        let Some(stem) = name
            .strip_prefix(&self.async_prefix)
            .and_then(|name| name.strip_suffix(&self.async_suffix))
        else {
            return Ok(None);
        };
        if stem.is_empty() {
            return Ok(None);
        }
        let sync = format!("{}{stem}{}", self.sync_prefix, self.sync_suffix);
        ident_from_str(&sync, async_ident.span()).map(Some)
    }

    pub fn value(&self) -> String {
        self.lit.value()
    }
}

fn split_pattern_side(lit: &LitStr, side: &str) -> syn::Result<(String, String)> {
    let (prefix, suffix) = match side.split_once("{}") {
        Some((prefix, suffix)) if !suffix.contains("{}") => (prefix, suffix),
        _ => {
            return Err(syn::Error::new(
                lit.span(),
                "each side of a name pattern must contain exactly one `{}` placeholder, \
                 e.g. `async_{}`, `{}_async` or `async_{} => sync_{}`",
            ));
        }
    };
    Ok((prefix.to_string(), suffix.to_string()))
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
    pub pattern: Option<RenamePattern>,
    /// Pattern used to derive sync type/trait names from async names.
    pub type_pattern: Option<RenamePattern>,
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
    /// `Send`-ness of `async_trait` emission, with the `send` key's span for
    /// validation diagnostics.
    pub send: Option<(proc_macro2::Span, SendPolicy)>,
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
                | "send" | "feature" | "sync_feature" => {
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
            "send" => {
                input.parse::<Token![=]>()?;
                let policy = parse_send_policy(input)?;
                set_once(&mut cfg.send, (key.span(), policy), &key)?;
            }
            "wasm_send" => {
                return Err(syn::Error::new(
                    key.span(),
                    "`wasm_send` has been removed; use \
                     `async(send = cfg(not(target_arch = \"wasm32\")))`",
                ));
            }
            _ => {
                return Err(syn::Error::new(
                    key.span(),
                    "unknown async option; expected `feature`, `name`, `doc`, `attr`, \
                     `test_attr`, `declaration`, `flavor` or `send`",
                ));
            }
        }
        parse_optional_comma(input)?;
    }
    Ok(())
}

/// Parse the value of `async(send = ...)`: `true`, `false`, or `cfg(PRED)`.
fn parse_send_policy(input: ParseStream) -> syn::Result<SendPolicy> {
    if input.peek(LitBool) {
        let value: LitBool = input.parse()?;
        return Ok(if value.value {
            SendPolicy::Always
        } else {
            SendPolicy::Never
        });
    }
    let ident: Ident = input
        .parse()
        .map_err(|_| input.error("expected `send = true`, `send = false`, or `send = cfg(...)`"))?;
    if ident != "cfg" {
        return Err(syn::Error::new(
            ident.span(),
            "expected `send = true`, `send = false`, or `send = cfg(...)`",
        ));
    }
    let predicate;
    parenthesized!(predicate in input);
    let predicate: TokenStream = predicate.parse()?;
    if predicate.is_empty() {
        return Err(syn::Error::new(
            ident.span(),
            "`send = cfg(...)` requires a cfg predicate, e.g. \
             `send = cfg(not(target_arch = \"wasm32\"))`",
        ));
    }
    Ok(SendPolicy::Cfg(predicate))
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

fn parse_pattern_value(input: ParseStream) -> syn::Result<RenamePattern> {
    RenamePattern::parse(parse_value(input)?)
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
        "wasm_send" | "send" => "`async(send = ...)`",
        _ => "`sync(feature = \"...\")` / `async(feature = \"...\")`",
    };
    syn::Error::new(
        key.span(),
        format!("`{key}` is now variant-specific; use {replacement}"),
    )
}

impl Config {
    pub fn parse_args(args: TokenStream) -> syn::Result<Self> {
        syn::parse2(args)
    }

    /// Merge an item-level config over shared defaults (from `scope`/`scoped!`).
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
        merge_option(&mut self.async_.send, &defaults.async_.send);
        self.async_
            .attrs
            .extend(defaults.async_.attrs.iter().cloned());
        self
    }

    pub fn flavor(&self) -> Flavor {
        self.async_.flavor.unwrap_or_default()
    }

    pub fn send_policy(&self) -> SendPolicy {
        self.async_
            .send
            .as_ref()
            .map_or(SendPolicy::Always, |(_, policy)| policy.clone())
    }

    pub fn validate_effective(&self) -> syn::Result<()> {
        if let Some((span, _)) = &self.async_.send
            && self.flavor() != Flavor::AsyncTrait
        {
            return Err(syn::Error::new(
                *span,
                "`async(send = ...)` requires `async(flavor = async_trait)`; native \
                 `async fn` infers the future's `Send`-ness from the body",
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

/// Compute the sync ident for a source (async) ident, erroring with guidance
/// when no pattern was configured or the name does not match it.
pub fn sync_ident(
    exact: Option<&Ident>,
    pattern: Option<&RenamePattern>,
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
    pattern.sync_name(async_ident)?.ok_or_else(|| {
        let value = pattern.value();
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

#[cfg(test)]
mod tests {
    use super::RenamePattern;
    use proc_macro2::Span;
    use syn::{Ident, LitStr};

    fn sync_name(pattern: &str, async_name: &str) -> Option<String> {
        let pattern =
            RenamePattern::parse(LitStr::new(pattern, Span::call_site())).expect("valid pattern");
        pattern
            .sync_name(&Ident::new(async_name, Span::call_site()))
            .expect("valid ident")
            .map(|ident| ident.to_string())
    }

    #[test]
    fn short_form_strips_the_affix() {
        assert_eq!(sync_name("async_{}", "async_read"), Some("read".into()));
        assert_eq!(sync_name("{}_async", "read_async"), Some("read".into()));
        assert_eq!(sync_name("{}", "read"), Some("read".into()));
        assert_eq!(sync_name("async_{}", "read"), None);
        assert_eq!(sync_name("async_{}", "async_"), None);
    }

    #[test]
    fn arrow_form_maps_the_stem() {
        assert_eq!(
            sync_name("async_{} => sync_{}", "async_read"),
            Some("sync_read".into())
        );
        assert_eq!(
            sync_name("{}_async => {}_blocking", "read_async"),
            Some("read_blocking".into())
        );
        assert_eq!(sync_name("{} => {}_sync", "read"), Some("read_sync".into()));
        assert_eq!(
            sync_name("Async{} => Sync{}", "AsyncClient"),
            Some("SyncClient".into())
        );
        assert_eq!(sync_name("async_{} => sync_{}", "other"), None);
    }

    #[test]
    fn invalid_patterns_are_rejected() {
        for bad in [
            "async",
            "a_{}_{}",
            "async_{} => ",
            "async_{} => {} => {}",
            " => {}",
        ] {
            let lit = LitStr::new(bad, Span::call_site());
            assert!(
                RenamePattern::parse(lit).is_err(),
                "`{bad}` should be rejected"
            );
        }
    }
}
