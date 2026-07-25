//! Proc-macro implementation for the [`ambisync`] crate.
//!
//! Do not depend on this crate directly; use `ambisync`, which re-exports the macros together with the runtime helpers.
//!
//! [`ambisync`]: https://docs.rs/ambisync

use proc_macro::TokenStream;

mod config;
mod expand;
mod rewrite;
mod subst;

/// Emit an async and a sync variant of the annotated item.
///
/// The item is written in async form (async types, async bodies, `.await`); the sync variant is derived by stripping `async`/`.await` and applying the `sync(...)` naming and substitution options.
/// See the `ambisync` crate documentation for the full option list.
/// Misuse is rejected at compile time.
#[proc_macro_attribute]
pub fn ambisync(args: TokenStream, input: TokenStream) -> TokenStream {
    expand::ambisync(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Add a mechanically derived synchronous sibling for every async method in a mixed trait or impl.
///
/// Unlike [`ambisync`], this keeps a single item and leaves its existing synchronous members
/// untouched. See the `ambisync` crate documentation for details.
#[proc_macro_attribute]
pub fn paired(args: TokenStream, input: TokenStream) -> TokenStream {
    expand::paired(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Emit a sync `#[test]` and a cfg-gated async test (default `#[tokio::test]`, override either variant with `sync(test_attr = #[...])` / `async(test_attr = #[...])`) from one async-written test fn.
#[proc_macro_attribute]
pub fn test(args: TokenStream, input: TokenStream) -> TokenStream {
    expand::test(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Apply shared defaults to every `#[ambisync]`/`#[ambisync::test]`-annotated item inside an inline module.
/// For a file module (`mod foo;`), use [`scoped!`](macro@scoped) inside the file instead.
#[proc_macro_attribute]
pub fn scope(args: TokenStream, input: TokenStream) -> TokenStream {
    expand::scope(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Function-like form of [`scope`](macro@scope): wraps a sequence of items and carries shared defaults in a leading `#![defaults(...)]` inner attribute.
#[proc_macro]
pub fn scoped(input: TokenStream) -> TokenStream {
    expand::scoped(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
