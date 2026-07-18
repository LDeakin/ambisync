//! Proc-macro implementation for the [`ambisync`] crate.
//!
//! Do not depend on this crate directly; use `ambisync`, which re-exports the
//! macros together with the runtime helpers.
//!
//! [`ambisync`]: https://docs.rs/ambisync

use proc_macro::TokenStream;

mod config;
mod expand;
mod rewrite;
mod subst;

/// Emit an async and a sync variant of the annotated item.
///
/// The item is written in *async form* (async types, async bodies, `.await`);
/// the async variant is emitted with its configured output name, and the sync variant is
/// derived by stripping `async`/`.await`, selecting its configured exact name
/// or applying the sync naming patterns, and applying the `sync(types(...))`/
/// `sync(calls(...))` substitution tables. Any supported item variant may
/// instead provide a complete empty `declaration { item shell }`. See the
/// `ambisync` crate documentation for the full option list.
#[proc_macro_attribute]
pub fn dual(args: TokenStream, input: TokenStream) -> TokenStream {
    expand::dual(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Emit a sync `#[test]` and a cfg-gated async test (default `#[tokio::test]`,
/// override either variant with `sync(test_attr = #[...])` /
/// `async(test_attr = #[...])`) from one async-written test fn.
#[proc_macro_attribute]
pub fn test(args: TokenStream, input: TokenStream) -> TokenStream {
    expand::test(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Apply shared `dual` defaults to every `#[dual]`/`#[ambisync::test]`-annotated
/// item inside an inline module.
#[proc_macro_attribute]
pub fn scope(args: TokenStream, input: TokenStream) -> TokenStream {
    expand::scope(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Function-like form of [`scope`]: wraps a sequence of items and carries
/// shared defaults in a leading `#![defaults(...)]` inner attribute.
#[proc_macro]
pub fn content(input: TokenStream) -> TokenStream {
    expand::content(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
