//! Write the async form once; [`macro@dual`] derives the sync variant
//! mechanically.
//!
//! The annotated item is written in **async form** — async types, async
//! bodies, `.await` — so the source is real, compilable async code.
//! `#[dual]` emits two items:
//!
//! - the **async variant**: the source as written;
//! - the **sync variant**: `async` removed, every `.await` stripped
//!   (`async { ... }` blocks become immediately invoked closures), and names,
//!   types, and calls mapped through the `sync(...)` options.
//!
//! Naming is **always explicit** — no `async_` prefix or other convention is
//! assumed. Give an exact sync name, or use `sync(pattern = "async_{}")` /
//! `sync(type_pattern = "Async{}")` where one convention covers methods or
//! multiple scoped items; the `{}` capture is the sync name. The arrow form
//! maps the `{}` stem into a different sync spelling instead:
//! `pattern = "async_{} => sync_{}"` maps `async_read` to `sync_read` (the
//! short form is sugar for `"... => {}"`).
//!
//! ```
//! struct Store(u32);
//! struct AsyncStore(u32);
//! impl Store { fn get(&self) -> u32 { self.0 } }
//! impl AsyncStore { async fn get(&self) -> u32 { self.0 } }
//!
//! #[ambisync::dual(sync(name = "read", types(AsyncStore => Store)))]
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
//! | Option | `sync(...)` | `async(...)` | Effect |
//! |---|:-:|:-:|---|
//! | `feature = "..."` | ✓ | ✓ | cfg-gate the variant |
//! | `name = "..."` | ✓ | ✓ | exact output item name |
//! | `doc = "..."` / `attr = #[...]` | ✓ | ✓ | variant-specific docs and item attributes |
//! | `test_attr = #[...]` | ✓ | ✓ | test attribute (defaults `#[test]` / `#[tokio::test]`) |
//! | `declaration { ... }` | ✓ | ✓ | complete outer declaration for the variant |
//! | `pattern = "async_{}"` | ✓ | — | derive sync fn/method names; `{}` captures the sync name |
//! | `pattern = "async_{} => sync_{}"` | ✓ | — | arrow form: map the async spelling to a sync spelling sharing the `{}` stem |
//! | `type_pattern = "Async{}"` | ✓ | — | derive sync type/trait names; short and arrow forms as above |
//! | `types(A => B, ...)` | ✓ | — | type substitutions, token-level, generic-aware; paths may be module-qualified |
//! | `calls(f => g, ...)` | ✓ | — | call renames (the `.await` lives in the source) |
//! | `flavor = async_fn \| async_trait` | — | ✓ | async trait/impl emission style |
//! | `wasm_send` | — | ✓ | with `async_trait`: native `Send` / wasm `?Send` |
//!
//! Both variants can be feature-gated, either one, or neither. The attribute
//! is **never wrapped in `cfg_attr`** — it always runs and emits the cfg gates
//! itself, so the expansion stays correct under every feature combination.
//!
//! # Explicit declarations
//!
//! Functions, traits, impls, structs, and enums can each supply a complete
//! empty Rust item shell. Its outer attributes and header are emitted exactly
//! as written; the rewritten implementation block, members, fields, or
//! variants are attached afterward:
//!
//! ```ignore
//! #[ambisync::dual(
//!     sync(declaration {
//!         /// Blocking API.
//!         #[must_use]
//!         pub fn read(store: &Store) -> u32 {}
//!     }),
//! )]
//! /// Async API.
//! pub async fn async_read(store: &AsyncStore) -> u32 {
//!     store.get().await
//! }
//! ```
//!
//! Empty `trait`, `impl`, `struct`, and `enum` bodies similarly receive the
//! source members, fields, or variants. Functions also accept the compact
//! body-less `fn name(...);` form. With only a sync declaration, the original
//! async item retains its ordinary docs, attributes, visibility, and header.
//! Both declarations may be explicit.
//!
//! # Divergent bodies
//!
//! For code that genuinely differs between the variants, use the marker
//! macros, which the rewriter resolves during expansion:
//!
//! ```ignore
//! let label = ambisync::alt!(sync => "sync", async => "async");
//! ambisync::sync_only! { chunks.into_par_iter().try_for_each(process)?; }
//! ambisync::async_only! { stream::iter(chunks).buffer_unordered(n).try_collect().await?; }
//! ```
//!
//! # Per-variant fields, enum variants, and members
//!
//! Struct fields, enum variants, and trait/impl members can be kept in only
//! one expansion with the `#[sync_only]` / `#[async_only]` attributes:
//!
//! ```ignore
//! #[ambisync::dual(sync(name = "Options"))]
//! pub struct AsyncOptions {
//!     pub common: u32,
//!     #[sync_only]
//!     pub threads: usize,
//!     #[async_only]
//!     pub concurrency: usize,
//! }
//!
//! #[ambisync::dual(sync(name = "Mode"))]
//! pub enum AsyncMode {
//!     Shared,
//!     #[sync_only]
//!     Blocking { threads: usize },
//!     #[async_only]
//!     Concurrent { concurrency: usize },
//! }
//!
//! #[ambisync::dual(sync(name = "Fetch", pattern = "async_{}"))]
//! pub trait AsyncFetch {
//!     async fn async_get(&self, key: &str) -> Option<Vec<u8>>;
//!     #[sync_only]
//!     fn threads(&self) -> usize;
//!     #[async_only]
//!     fn concurrency(&self) -> usize;
//! }
//! ```
//!
//! # Docs
//!
//! Both variants keep the source doc comments. Doc text receives the same
//! token-level substitutions as code — `types(...)`, `calls(...)`, and item
//! renames, matched at identifier boundaries — so intra-doc links and prose
//! follow each variant's names (`` [`AsyncStore`] `` → `` [`Store`] ``).
//! Append variant-specific paragraphs with `sync(doc = "...")` /
//! `async(doc = "...")`.
//!
//! # Tests
//!
//! [`macro@test`] emits a sync `#[test]` and an async test from one
//! async-written test fn:
//!
//! ```ignore
//! #[ambisync::test(
//!     sync(name = "roundtrip"),
//!     async(feature = "async", test_attr = #[tokio::test]),
//! )]
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
//! - `#[ambisync::scope]` needs an *inline* module; for `mod foo;` files use
//!   [`content!`] inside the file.

pub use ambisync_macros::{content, dual, scope, test};

/// Expression-level variant selector, resolved by `#[dual]` during expansion.
///
/// `ambisync::alt!(sync => expr_a, async => expr_b)` becomes `expr_a` in the
/// sync expansion and `expr_b` in the async expansion. Each arm is only
/// type-checked in its own expansion.
#[macro_export]
macro_rules! alt {
    ($($tt:tt)*) => {
        ::core::compile_error!(
            "`ambisync::alt!` is only meaningful inside an item annotated with `#[ambisync::dual]` or `#[ambisync::test]`"
        )
    };
}

/// Sync-only marker.
///
/// - As a statement macro (`ambisync::sync_only! { ... }`) inside a `#[dual]`
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
            "`ambisync::sync_only!` is only meaningful inside an item annotated with `#[ambisync::dual]` or `#[ambisync::test]`"
        )
    };
}

/// Async-only marker.
///
/// - As a statement macro (`ambisync::async_only! { ... }`) inside a `#[dual]`
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
            "`ambisync::async_only!` is only meaningful inside an item annotated with `#[ambisync::dual]` or `#[ambisync::test]`"
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
