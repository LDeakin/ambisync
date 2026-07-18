//! Write sync/async-generic Rust: author the async form once, derive the sync
//! variant mechanically.
//!
//! The annotated item is written **entirely as the async variant** — async
//! names, async types, `.await` — so the source is real, compilable async
//! code. The [`macro@dual`] attribute then emits:
//!
//! - the **async variant**: the source as written (helper attributes removed,
//!   markers resolved), optionally gated with `async(feature = "...")`;
//! - the **sync variant**: `async` removed, every `.await` stripped, names
//!   mapped back through the explicit `rename`/`rename_types` patterns and the
//!   `types(...)`/`calls(...)` substitution tables, optionally gated with
//!   `sync(feature = "...")`.
//!
//! Renaming is **always explicit** — `unasync` does not assume an `async_`
//! prefix or any other convention. Give a pattern (`rename = "async_{}"`,
//! `rename = "{}_async"`, `rename_types = "Async{}"`, ...) or `keep_self`.
//!
//! ```
//! struct Store(u32);
//! struct AsyncStore(u32);
//! impl Store { fn get(&self) -> u32 { self.0 } }
//! impl AsyncStore { async fn get(&self) -> u32 { self.0 } }
//!
//! #[unasync::dual(rename = "async_{}", types(AsyncStore => Store))]
//! async fn async_read(store: &AsyncStore) -> u32 {
//!     store.get().await
//! }
//!
//! // Emits the source `async_read` unchanged, plus:
//! //   fn read(store: &Store) -> u32 { store.get() }
//! # assert_eq!(read(&Store(1)), 1);
//! ```
//!
//! # Options
//!
//! | Option | Effect |
//! |---|---|
//! | `sync(feature = "sync")` | gate the sync variant behind `#[cfg(feature = "sync")]` |
//! | `async(feature = "async")` | gate the async variant behind `#[cfg(feature = "async")]` |
//! | `skip_sync` / `skip_async` | emit only one variant |
//! | `keep_self` | no renaming: the sync variant keeps the source names |
//! | `rename = "async_{}"` | fn/method pattern the async names follow; `{}` is the sync name |
//! | `rename_types = "Async{}"` | type/trait pattern, same rules |
//! | `types(A => B, ...)` | async spelling → sync spelling type substitutions |
//! | `calls(f => g, ...)` | async call name → sync call name (the `.await` is written in the source) |
//! | `async_doc = "..."` | extra doc text on the async item |
//! | `async_attr = #[...]` | runtime attribute for [`macro@test`] (default `#[tokio::test]`) |
//! | `flavor = async_fn \| async_trait` | async trait/impl emission style |
//! | `wasm_send` | with `async_trait`: native `Send` / wasm `?Send` cfg pairing |
//!
//! Both variants can be feature-gated, either one, or neither. The attribute
//! is **never wrapped in `cfg_attr`** — it always runs and emits the cfg gates
//! itself, so the expansion stays correct under every feature combination.
//!
//! # Divergent bodies
//!
//! For code that genuinely differs between the variants, use the marker
//! macros, which the rewriter resolves during expansion:
//!
//! ```ignore
//! let label = unasync::alt!(sync => "sync", async => "async");
//! unasync::sync_only! { chunks.into_par_iter().try_for_each(process)?; }
//! unasync::async_only! { stream::iter(chunks).buffer_unordered(n).try_collect().await?; }
//! ```
//!
//! # Per-variant fields, enum variants, and members
//!
//! Struct fields, enum variants, and trait/impl members can be kept in only
//! one expansion with the `#[sync_only]` / `#[async_only]` attributes:
//!
//! ```ignore
//! #[unasync::dual(rename_types = "Async{}")]
//! pub struct AsyncOptions {
//!     pub common: u32,
//!     #[sync_only]
//!     pub threads: usize,
//!     #[async_only]
//!     pub concurrency: usize,
//! }
//! ```
//!
//! # Tests
//!
//! [`macro@test`] emits a sync `#[test]` and an async test from one
//! async-written test fn:
//!
//! ```ignore
//! #[unasync::test(async(feature = "async"), rename = "async_{}", async_attr = #[tokio::test])]
//! async fn async_roundtrip() { /* written once, with .await */ }
//! // → #[test] fn roundtrip()  +  #[cfg(feature = "async")] #[tokio::test] async fn async_roundtrip()
//! ```
//!
//! # Limitations (by design)
//!
//! - Substitution is **syntactic**: paths and types are matched token-for-token
//!   with no name resolution. Aliased spellings must be added to the table.
//! - Inside macro invocations (`format!`, `assert_eq!`, ...) the sync rewrite
//!   is best-effort: `.await` stripping, single-ident type substitutions, and
//!   `name(...)` call renames.
//! - `#[unasync::scope]` needs an *inline* module; for `mod foo;` files use
//!   [`content!`] inside the file.

pub use unasync_macros::{content, dual, scope, test};

/// Expression-level variant selector, resolved by `#[dual]` during expansion.
///
/// `unasync::alt!(sync => expr_a, async => expr_b)` becomes `expr_a` in the
/// sync expansion and `expr_b` in the async expansion. Each arm is only
/// type-checked in its own expansion.
#[macro_export]
macro_rules! alt {
    ($($tt:tt)*) => {
        ::core::compile_error!(
            "`unasync::alt!` is only meaningful inside an item annotated with `#[unasync::dual]` or `#[unasync::test]`"
        )
    };
}

/// Sync-only marker.
///
/// - As a statement macro (`unasync::sync_only! { ... }`) inside a `#[dual]`
///   body: the statements are kept only in the sync expansion.
/// - As an attribute (`#[sync_only]`) on a struct field, enum variant, or
///   trait/impl member of a `#[dual]` item: the element is kept only in the
///   sync expansion.
///
/// Resolved by `#[dual]` during expansion.
#[macro_export]
macro_rules! sync_only {
    ($($tt:tt)*) => {
        ::core::compile_error!(
            "`unasync::sync_only!` is only meaningful inside an item annotated with `#[unasync::dual]` or `#[unasync::test]`"
        )
    };
}

/// Async-only marker.
///
/// - As a statement macro (`unasync::async_only! { ... }`) inside a `#[dual]`
///   body: the statements are kept only in the async expansion.
/// - As an attribute (`#[async_only]`) on a struct field, enum variant, or
///   trait/impl member of a `#[dual]` item: the element is kept only in the
///   async expansion.
///
/// Resolved by `#[dual]` during expansion.
#[macro_export]
macro_rules! async_only {
    ($($tt:tt)*) => {
        ::core::compile_error!(
            "`unasync::async_only!` is only meaningful inside an item annotated with `#[unasync::dual]` or `#[unasync::test]`"
        )
    };
}

/// `Send` on every target except `wasm32`, where it is a no-op bound.
///
/// Use in async trait bounds that must relax on single-threaded wasm.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> MaybeSend for T {}

/// `Send` on every target except `wasm32`, where it is a no-op bound.
///
/// Use in async trait bounds that must relax on single-threaded wasm.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

/// `Sync` on every target except `wasm32`, where it is a no-op bound.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSync: Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync + ?Sized> MaybeSync for T {}

/// `Sync` on every target except `wasm32`, where it is a no-op bound.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSync {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSync for T {}
