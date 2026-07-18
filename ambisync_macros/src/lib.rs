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
/// The item is written in *async form* (async types, async bodies, `.await`);
/// the async variant is emitted with its configured output name, and the sync variant is
/// derived by stripping `async`/`.await`, selecting its configured exact name
/// or applying the sync naming patterns, and applying the `sync(types(...))`/
/// `sync(calls(...))` substitution tables. Any supported item variant may
/// instead provide a complete empty `declaration { item shell }`. See the
/// `ambisync` crate documentation for the full option list.
///
/// # Errors
///
/// Misuse is rejected at compile time. The error cases are:
///
/// - the annotated item is not a `fn`, `trait`, `impl`, `struct`, or `enum`,
///   or a function/method meant to be rewritten is not written in async form;
/// - the sync variant has no name: no `sync(name = ...)`, no matching
///   `sync(pattern = ...)` / `sync(type_pattern = ...)`, and no declaration
///   (or a pattern that fails to match the async spelling);
/// - the sync and async variants would emit the same name;
/// - a naming pattern is malformed (no `{}` capture, or a capture that is not
///   a valid identifier when applied);
/// - a `declaration { ... }` has a non-empty body, declares a different item
///   kind than the annotated item, or is combined with `name`/`doc`/`attr`
///   for the same variant;
/// - `test_attr` is used outside [`macro@test`].
#[proc_macro_attribute]
pub fn ambisync(args: TokenStream, input: TokenStream) -> TokenStream {
    expand::ambisync(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Emit a sync `#[test]` and a cfg-gated async test (default `#[tokio::test]`,
/// override either variant with `sync(test_attr = #[...])` /
/// `async(test_attr = #[...])`) from one async-written test fn.
///
/// # Errors
///
/// Fails to compile when applied to anything other than an async-written
/// function, plus the naming and declaration error cases of
/// [`macro@ambisync`].
#[proc_macro_attribute]
pub fn test(args: TokenStream, input: TokenStream) -> TokenStream {
    expand::test(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Apply shared defaults to every `#[ambisync]`/`#[ambisync::test]`-annotated
/// item inside an inline module.
///
/// # Errors
///
/// Fails to compile on a non-inline module (`mod foo;` — use
/// [`scoped!`](macro@scoped) inside the file instead), or when the defaults
/// contain item-specific options (`name = ...` or `declaration { ... }`).
#[proc_macro_attribute]
pub fn scope(args: TokenStream, input: TokenStream) -> TokenStream {
    expand::scope(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Function-like form of [`scope`](macro@scope): wraps a sequence of items and
/// carries shared defaults in a leading `#![defaults(...)]` inner attribute.
///
/// # Errors
///
/// Fails to compile on inner attributes other than `#![defaults(...)]`, and on
/// the same defaults restrictions as [`scope`](macro@scope).
#[proc_macro]
pub fn scoped(input: TokenStream) -> TokenStream {
    expand::scoped(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
