//! Attribute-argument parsing for `#[unasync::dual]`, `#[unasync::test]`,
//! `#[unasync::scope]` and `unasync::content!`.

use proc_macro2::TokenStream;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Attribute, Ident, LitStr, Token, Type, parenthesized};

/// How the async variant of a trait / impl is emitted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Flavor {
    /// Native `async fn` (with `#[allow(async_fn_in_trait)]` on traits).
    #[default]
    AsyncFn,
    /// `#[async_trait::async_trait]`, optionally with the wasm `?Send` pairing.
    AsyncTrait,
}

/// One `types(AsyncSpelling => SyncSpelling)` entry. The left side is the
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

/// One `calls(async_name => sync_name)` entry. In the derived sync expansion,
/// calls/method-calls named `async_name` are renamed to `sync_name` (the
/// `.await` on the call was written in the source and is stripped anyway).
#[derive(Clone)]
pub struct CallSwap {
    pub async_: Ident,
    pub sync: Ident,
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
pub struct Config {
    /// Feature gating the sync expansion (`sync(feature = "...")`).
    pub sync_gate: Option<LitStr>,
    /// Feature gating the async expansion (`async(feature = "...")`).
    pub async_gate: Option<LitStr>,
    /// Emit only the async expansion.
    pub skip_sync: bool,
    /// Emit only the sync expansion.
    pub skip_async: bool,
    /// Do not rename: the sync expansion keeps the source names.
    pub keep_self: bool,
    /// Rename pattern for fns/methods, e.g. `async_{}`. The source (async)
    /// name must match the pattern; the sync name is what `{}` captures.
    /// Required whenever a fn/method rename is needed — there is no default.
    pub rename: Option<LitStr>,
    /// Rename pattern for types/traits, e.g. `Async{}`. Same rules as
    /// `rename`.
    pub rename_types: Option<LitStr>,
    pub types: Vec<TypeSubst>,
    pub calls: Vec<CallSwap>,
    /// Extra doc text appended to the generated async stub doc.
    pub async_doc: Option<LitStr>,
    /// Attribute(s) placed on the async test variant (default `#[tokio::test]`).
    pub async_attr: Option<Vec<Attribute>>,
    pub flavor: Flavor,
    /// With `flavor = async_trait`: emit the native/wasm `?Send` cfg_attr pair.
    pub wasm_send: bool,
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
                cfg.async_gate = Some(parse_gate(&content)?);
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
                    cfg.sync_gate = Some(parse_gate(&content)?);
                }
                "skip_sync" => cfg.skip_sync = true,
                "skip_async" => cfg.skip_async = true,
                "keep_self" => cfg.keep_self = true,
                "wasm_send" => cfg.wasm_send = true,
                "rename" => {
                    input.parse::<Token![=]>()?;
                    cfg.rename = Some(parse_pattern(input)?);
                }
                "rename_types" => {
                    input.parse::<Token![=]>()?;
                    cfg.rename_types = Some(parse_pattern(input)?);
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
                "async_doc" => {
                    input.parse::<Token![=]>()?;
                    cfg.async_doc = Some(input.parse()?);
                }
                "async_attr" => {
                    input.parse::<Token![=]>()?;
                    let attrs = input.call(Attribute::parse_outer)?;
                    if attrs.is_empty() {
                        return Err(input.error("expected an attribute, e.g. `#[tokio::test]`"));
                    }
                    cfg.async_attr = Some(attrs);
                }
                "flavor" => {
                    input.parse::<Token![=]>()?;
                    let flavor: Ident = input.parse()?;
                    cfg.flavor = match flavor.to_string().as_str() {
                        "async_fn" => Flavor::AsyncFn,
                        "async_trait" => Flavor::AsyncTrait,
                        "boxed" => {
                            return Err(syn::Error::new(
                                flavor.span(),
                                "`flavor = boxed` is not implemented yet; use `async_fn` or `async_trait`",
                            ));
                        }
                        _ => {
                            return Err(syn::Error::new(
                                flavor.span(),
                                "unknown flavor; expected `async_fn` or `async_trait`",
                            ));
                        }
                    };
                }
                "feature" | "sync_feature" => {
                    return Err(syn::Error::new(
                        key.span(),
                        "feature gates are per-variant: use `async(feature = \"...\")` and/or \
                         `sync(feature = \"...\")`",
                    ));
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown option `{key}`; expected one of `sync(feature = ...)`, \
                             `async(feature = ...)`, `skip_sync`, `skip_async`, `keep_self`, \
                             `rename`, `rename_types`, `types(...)`, `calls(...)`, `async_doc`, \
                             `async_attr`, `flavor`, `wasm_send`"
                        ),
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

/// Parse the contents of `sync(...)` / `async(...)`: currently `feature = "..."`.
fn parse_gate(input: ParseStream) -> syn::Result<LitStr> {
    let key: Ident = input.parse()?;
    if key != "feature" {
        return Err(syn::Error::new(
            key.span(),
            "expected `feature = \"...\"` inside `sync(...)`/`async(...)`",
        ));
    }
    input.parse::<Token![=]>()?;
    let lit: LitStr = input.parse()?;
    if input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
    }
    if !input.is_empty() {
        return Err(input.error("unexpected tokens after `feature = \"...\"`"));
    }
    Ok(lit)
}

fn parse_pattern(input: ParseStream) -> syn::Result<LitStr> {
    let lit: LitStr = input.parse()?;
    if !lit.value().contains("{}") {
        return Err(syn::Error::new(
            lit.span(),
            "rename pattern must contain `{}` as the placeholder for the sync name, \
             e.g. `async_{}` or `{}_async`",
        ));
    }
    Ok(lit)
}

impl Config {
    pub fn parse_args(args: TokenStream) -> syn::Result<Self> {
        syn::parse2(args)
    }

    /// Merge an item-level config over shared defaults (from `scope`/`content!`).
    /// Item-level values win; substitution tables are concatenated with the
    /// item-level entries first (first match wins during rewriting).
    pub fn merged_over(mut self, defaults: &Config) -> Config {
        if self.sync_gate.is_none() {
            self.sync_gate = defaults.sync_gate.clone();
        }
        if self.async_gate.is_none() {
            self.async_gate = defaults.async_gate.clone();
        }
        if self.rename.is_none() {
            self.rename = defaults.rename.clone();
        }
        if self.rename_types.is_none() {
            self.rename_types = defaults.rename_types.clone();
        }
        if self.async_doc.is_none() {
            self.async_doc = defaults.async_doc.clone();
        }
        if self.async_attr.is_none() {
            self.async_attr = defaults.async_attr.clone();
        }
        if self.flavor == Flavor::default() {
            self.flavor = defaults.flavor;
        }
        self.skip_sync |= defaults.skip_sync;
        self.skip_async |= defaults.skip_async;
        self.keep_self |= defaults.keep_self;
        self.wasm_send |= defaults.wasm_send;
        self.types.extend(defaults.types.iter().cloned());
        self.calls.extend(defaults.calls.iter().cloned());
        self
    }
}

/// Derive the sync name from an async (source) name by matching it against a
/// rename pattern: `unapply_pattern("async_{}", async_read)` → `read`.
pub fn unapply_pattern(pattern: &str, ident: &Ident) -> Option<Ident> {
    let (prefix, suffix) = pattern.split_once("{}")?;
    let name = ident.to_string();
    let core = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if core.is_empty() {
        return None;
    }
    Some(Ident::new(core, ident.span()))
}

/// Compute the sync ident for a source (async) ident, erroring with guidance
/// when no pattern was configured or the name does not match it.
pub fn sync_ident(pattern: &Option<LitStr>, which: &str, ident: &Ident) -> syn::Result<Ident> {
    let Some(pattern) = pattern else {
        return Err(syn::Error::new(
            ident.span(),
            format!(
                "cannot derive the sync name of `{ident}`: unasync does not assume a naming \
                 convention; add `{which} = \"...\"` (e.g. `{which} = \"async_{{}}\"` if async \
                 items carry an `async_` prefix) or `keep_self`"
            ),
        ));
    };
    let value = pattern.value();
    unapply_pattern(&value, ident).ok_or_else(|| {
        syn::Error::new(
            ident.span(),
            format!("`{ident}` does not match the `{which}` pattern `{value}`"),
        )
    })
}
