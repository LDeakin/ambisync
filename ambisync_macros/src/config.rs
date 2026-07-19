//! Attribute-argument parsing for `#[ambisync]`, `#[ambisync::test]`, `#[ambisync::scope]` and `ambisync::scoped!`.

use proc_macro2::TokenStream;
use syn::parse::discouraged::Speculative;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{
    braced, parenthesized, Attribute, Ident, Item, ItemEnum, ItemImpl, ItemStruct, ItemTrait,
    LitBool, LitStr, Signature, Token, Type, Visibility,
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

/// One exact `sync(types(AsyncSpelling => SyncSpelling))` entry: the async spelling, matched token-for-token in the sync expansion, and its sync replacement.
/// A bare single-ident entry matching the annotated type/trait also names its sync variant.
#[derive(Clone)]
pub struct TypeSubst {
    pub async_: Type,
    pub sync: Type,
}

impl Parse for TypeSubst {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let async_: Type = input.parse()?;
        if input.peek(syn::token::Brace) {
            return Err(input.error(
                "`{}` placeholders are not valid in type position; write pattern entries as \
                 string literals, e.g. `types(\"Async{}\")` or `types(\"Async{} => Sync{}\")`",
            ));
        }
        input.parse::<Token![=>]>()?;
        let sync: Type = input.parse()?;
        Ok(Self { async_, sync })
    }
}

/// A string-literal pattern entry of the `sync(fns(...))` / `sync(types(...))` rename tables.
///
/// The short form names the async spelling and captures the sync name: `"async_{}"` maps `async_read` to `read`.
/// The full form maps the async spelling to a sync spelling built from the same `{}` stem: `"async_{} => sync_{}"` maps `async_read` to `sync_read`.
/// The short form is sugar for `"... => {}"`.
#[derive(Clone)]
pub struct RenamePattern {
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
            async_prefix,
            async_suffix,
            sync_prefix,
            sync_suffix,
        })
    }

    /// The sync ident derived from an async (source) ident, or `None` when the ident does not match the pattern's async side.
    pub fn sync_name(&self, async_ident: &Ident) -> syn::Result<Option<Ident>> {
        let Some(sync) = self.sync_name_str(&async_ident.to_string()) else {
            return Ok(None);
        };
        ident_from_str(&sync, async_ident.span()).map(Some)
    }

    /// String-level pattern application, used by the substitution tables where names may live inside macro tokens or doc text.
    pub fn sync_name_str(&self, name: &str) -> Option<String> {
        let stem = name
            .strip_prefix(&self.async_prefix)?
            .strip_suffix(&self.async_suffix)?;
        if stem.is_empty() {
            return None;
        }
        Some(format!("{}{stem}{}", self.sync_prefix, self.sync_suffix))
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

/// One exact `sync(fns(async_name => sync_name))` entry: fns/methods named `async_name` get the sync name `sync_name`, and calls and method calls to `async_name` are renamed in the sync expansion.
#[derive(Clone)]
pub struct FnRename {
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

/// An empty Rust item shell supplied for one variant.
/// Its attributes and header are emitted exactly as written; rewritten contents from the annotated item are attached after rewriting.
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

impl Parse for FnRename {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(syn::token::Brace) {
            return Err(input.error(
                "`{}` placeholders are not valid in name position; write pattern entries as \
                 string literals, e.g. `fns(\"{}_async\")` or `fns(\"create_async_{} => create_{}\")`",
            ));
        }
        let async_: Ident = input.parse()?;
        if input.peek(syn::token::Brace) {
            return Err(input.error(
                "`{}` placeholders are not valid in name position; write pattern entries as \
                 string literals, e.g. `fns(\"async_{}\")` or `fns(\"async_{} => sync_{}\")`",
            ));
        }
        input.parse::<Token![=>]>()?;
        let sync: Ident = input.parse()?;
        if input.peek(Token![.]) {
            return Err(input.error(
                "fn renames map names only: the source is written in async form, so write the \
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
    /// Exact name for the annotated sync item, overriding the rename tables.
    pub name: Option<Ident>,
    /// Extra documentation appended to the sync variant.
    pub doc: Option<LitStr>,
    /// Attributes appended to the sync variant.
    pub attrs: Vec<Attribute>,
    /// Attribute(s) placed on the sync test variant (default `#[test]`).
    pub test_attr: Option<Vec<Attribute>>,
    /// Complete declaration shell for a generated sync item.
    pub declaration: Option<Declaration>,
    /// Exact entries of the type-namespace rename table: name the annotated type/trait and substitute type references.
    pub types: Vec<TypeSubst>,
    /// Pattern entries of the type-namespace rename table (`types("Async{}")`), consulted after the exact entries.
    pub type_patterns: Vec<RenamePattern>,
    /// Exact entries of the fn-namespace rename table: name the annotated fn/methods and rename calls.
    pub fns: Vec<FnRename>,
    /// Pattern entries of the fn-namespace rename table (`fns("{}_async")`), consulted after the exact entries.
    pub fn_patterns: Vec<RenamePattern>,
}

#[derive(Clone, Default)]
pub struct AsyncConfig {
    /// Feature gating the async expansion.
    pub gate: Option<LitStr>,
    /// Exact name for the annotated async item.
    /// The source name is used when omitted.
    pub name: Option<Ident>,
    /// Extra documentation appended to the async variant.
    pub doc: Option<LitStr>,
    /// Attributes appended to the async variant.
    pub attrs: Vec<Attribute>,
    /// Attribute(s) placed on the async test variant (default `#[tokio::test]`).
    pub test_attr: Option<Vec<Attribute>>,
    /// Complete declaration shell for a generated async item.
    pub declaration: Option<Declaration>,
    /// Async trait/impl emission style.
    /// `None` means the default `async_fn` flavor.
    pub flavor: Option<Flavor>,
    /// `Send`-ness of `async_trait` emission, with the `send` key's span for validation diagnostics.
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
                "fns" => return Err(moved_substitution_error(&key, "fns")),
                "keep_self" => {
                    return Err(syn::Error::new(
                        key.span(),
                        "`keep_self` has been removed; use `sync(fns(\"{}\"))` for \
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
                parse_subst_entries(&content, &mut cfg.types, &mut cfg.type_patterns)?;
            }
            "fns" => {
                let content;
                parenthesized!(content in input);
                parse_subst_entries(&content, &mut cfg.fns, &mut cfg.fn_patterns)?;
            }
            _ => {
                return Err(syn::Error::new(
                    key.span(),
                    "unknown sync option; expected `feature`, `name`, `doc`, `attr`, \
                     `test_attr`, `declaration`, `types` or `fns`",
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

/// Parse the entries of `types(...)` / `fns(...)`: exact `A => B` mappings, plus string-literal
/// pattern entries (`"Async{}"`, `"{}_async => {}"`) that match any name with the given affixes.
fn parse_subst_entries<T: Parse>(
    input: ParseStream,
    exact: &mut Vec<T>,
    patterns: &mut Vec<RenamePattern>,
) -> syn::Result<()> {
    while !input.is_empty() {
        if input.peek(LitStr) {
            patterns.push(RenamePattern::parse(input.parse()?)?);
        } else {
            exact.push(input.parse()?);
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
        "rename" => "`sync(fns(\"...\"))`",
        "rename_types" => "`sync(types(\"...\"))`",
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
    /// Item-level values win; substitution tables are concatenated with the item-level entries first (first match wins during rewriting).
    pub fn merged_over(mut self, defaults: &Config) -> Config {
        merge_option(&mut self.sync.gate, &defaults.sync.gate);
        merge_option(&mut self.sync.name, &defaults.sync.name);
        merge_option(&mut self.sync.doc, &defaults.sync.doc);
        merge_option(&mut self.sync.test_attr, &defaults.sync.test_attr);
        merge_option(&mut self.sync.declaration, &defaults.sync.declaration);
        self.sync.attrs.extend(defaults.sync.attrs.iter().cloned());
        self.sync.types.extend(defaults.sync.types.iter().cloned());
        self.sync
            .type_patterns
            .extend(defaults.sync.type_patterns.iter().cloned());
        self.sync.fns.extend(defaults.sync.fns.iter().cloned());
        self.sync
            .fn_patterns
            .extend(defaults.sync.fn_patterns.iter().cloned());
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
        if let Some((span, _)) = &self.async_.send {
            if self.flavor() != Flavor::AsyncTrait {
                return Err(syn::Error::new(
                    *span,
                    "`async(send = ...)` requires `async(flavor = async_trait)`",
                ));
            }
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

impl SyncConfig {
    /// The sync name of a fn/method: the `name` override (if given for this item), then the `fns(...)` table — exact entries first, then patterns in order.
    pub fn sync_fn_ident(&self, exact: Option<&Ident>, async_ident: &Ident) -> syn::Result<Ident> {
        if let Some(exact) = exact {
            return Ok(exact.clone());
        }
        if let Some(entry) = self.fns.iter().find(|entry| entry.async_ == *async_ident) {
            return Ok(entry.sync.clone());
        }
        derive_from_patterns(&self.fn_patterns, async_ident, "fns", "\"async_{}\"")
    }

    /// The sync name of a type/trait: the `name` override (if given for this item), then the `types(...)` table — exact bare-ident entries first, then patterns in order.
    pub fn sync_type_ident(
        &self,
        exact: Option<&Ident>,
        async_ident: &Ident,
    ) -> syn::Result<Ident> {
        if let Some(exact) = exact {
            return Ok(exact.clone());
        }
        if let Some(sync) = self.types.iter().find_map(|entry| {
            single_ident(&entry.async_)
                .filter(|from| *from == async_ident)
                .and_then(|_| single_ident(&entry.sync))
        }) {
            return Ok(sync.clone());
        }
        derive_from_patterns(&self.type_patterns, async_ident, "types", "\"Async{}\"")
    }
}

/// The ident of a bare single-ident path type (`Store`, not `mod_x::Store` or `Store<T>`), the only type spelling that can name an item.
fn single_ident(ty: &Type) -> Option<&Ident> {
    let Type::Path(path) = ty else {
        return None;
    };
    if path.qself.is_some() || path.path.leading_colon.is_some() || path.path.segments.len() != 1 {
        return None;
    }
    let segment = &path.path.segments[0];
    segment.arguments.is_none().then_some(&segment.ident)
}

fn derive_from_patterns(
    patterns: &[RenamePattern],
    async_ident: &Ident,
    which: &str,
    example: &str,
) -> syn::Result<Ident> {
    for pattern in patterns {
        if let Some(ident) = pattern.sync_name(async_ident)? {
            return Ok(ident);
        }
    }
    Err(syn::Error::new(
        async_ident.span(),
        format!(
            "cannot derive the sync name of `{async_ident}`: add a matching `sync({which}(...))` \
             entry (for example `{which}({example})` or `{which}({async_ident} => ...)`), or \
             `sync(name = \"...\")` on the item"
        ),
    ))
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
    use super::{Config, RenamePattern};
    use proc_macro2::Span;
    use quote::quote;
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
    fn bare_placeholder_entries_suggest_string_patterns() {
        for args in [
            quote!(sync(types(Async{} => {}))),
            quote!(sync(fns({}_async => {}))),
            quote!(sync(fns(create_async_{} => create_{}))),
        ] {
            let err = Config::parse_args(args.clone())
                .err()
                .expect("bare `{}` placeholders should be rejected")
                .to_string();
            assert!(err.contains("string literals"), "`{args}`: {err}");
        }
    }

    #[test]
    fn pattern_entries_parse_alongside_exact_entries() {
        let cfg = Config::parse_args(quote!(sync(
            types("Async{}", AsyncStore => Store),
            fns("{}_async", "create_async_{} => create_{}", async_helper => helper),
        )))
        .expect("valid config");
        assert_eq!(cfg.sync.types.len(), 1);
        assert_eq!(cfg.sync.type_patterns.len(), 1);
        assert_eq!(cfg.sync.fns.len(), 1);
        assert_eq!(cfg.sync.fn_patterns.len(), 2);
    }

    #[test]
    fn item_names_derive_from_the_tables() {
        let cfg = Config::parse_args(quote!(sync(
            types(AsyncStore => Store, "Async{}"),
            fns(async_special => special_blocking, "async_{}"),
        )))
        .expect("valid config");
        let ident = |name: &str| Ident::new(name, Span::call_site());
        // Exact entries name the item; patterns cover the rest; exact wins.
        assert_eq!(
            cfg.sync
                .sync_type_ident(None, &ident("AsyncStore"))
                .unwrap(),
            "Store"
        );
        assert_eq!(
            cfg.sync
                .sync_type_ident(None, &ident("AsyncClient"))
                .unwrap(),
            "Client"
        );
        assert_eq!(
            cfg.sync
                .sync_fn_ident(None, &ident("async_special"))
                .unwrap(),
            "special_blocking"
        );
        assert_eq!(
            cfg.sync.sync_fn_ident(None, &ident("async_read")).unwrap(),
            "read"
        );
        // The `name` override beats the tables; no match at all is an error.
        assert_eq!(
            cfg.sync
                .sync_fn_ident(Some(&ident("exact")), &ident("async_read"))
                .unwrap(),
            "exact"
        );
        assert!(cfg.sync.sync_fn_ident(None, &ident("other")).is_err());
        assert!(cfg.sync.sync_type_ident(None, &ident("Other")).is_err());
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
