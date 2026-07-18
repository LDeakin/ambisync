//! Write the async form once; [`macro@ambisync`] derives the sync variant mechanically.
//!
//! The annotated item is written in **async form** — async types, async bodies, `.await` — so the source is real, compilable async code.
//! `#[ambisync]` emits two items:
//!
//! - the **async variant**: the source as written;
//! - the **sync variant**: `async` removed, every `.await` stripped
//! (`async { ... }` blocks become immediately invoked closures), and names, types, and calls mapped through the `sync(...)` options.
//!
//! Naming is **always explicit** — no `async_` prefix or other convention is assumed.
//! Give an exact sync name, or use `sync(fn_pattern = "async_{}")` / `sync(type_pattern = "Async{}")` where one convention covers methods or multiple scoped items; the `{}` capture is the sync name.
//! The arrow form maps the `{}` stem into a different sync spelling instead: `fn_pattern = "async_{} => sync_{}"` maps `async_read` to `sync_read` (the short form is sugar for `"... => {}"`).
//!
//! ```
//! struct Store(u32);
//! struct AsyncStore(u32);
//! impl Store { fn get(&self) -> u32 { self.0 } }
//! impl AsyncStore { async fn get(&self) -> u32 { self.0 } }
//!
//! #[ambisync::ambisync(sync(name = "read", types(AsyncStore => Store)))]
//! async fn async_read(store: &AsyncStore) -> u32 {
//!   store.get().await
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
//! | `fn_pattern = "async_{}"` | ✓ | — | derive sync fn/method names; `{}` captures the sync name |
//! | `fn_pattern = "async_{} => sync_{}"` | ✓ | — | arrow form: map the async spelling to a sync spelling sharing the `{}` stem |
//! | `type_pattern = "Async{}"` | ✓ | — | derive sync type/trait names; `{}` captures the sync name |
//! | `type_pattern = "Async{} => Sync{}"` | ✓ | — | arrow form: map the async spelling to a sync spelling sharing the `{}` stem |
//! | `types(A => B, ...)` | ✓ | — | type substitutions, token-level, generic-aware; paths may be module-qualified |
//! | `calls(f => g, ...)` | ✓ | — | call renames (the `.await` lives in the source) |
//! | `flavor = async_fn \| async_trait` | — | ✓ | async trait/impl emission style |
//! | `send = true \| false \| cfg(...)` | — | ✓ | with `async_trait`: `Send`-ness of the emission (default `Send`) |
//!
//! Both variants can be feature-gated, either one, or neither.
//! The attribute is **never wrapped in `cfg_attr`** — it always runs and emits the cfg gates itself, so the expansion stays correct under every feature combination.
//!
//! # Explicit declarations
//!
//! Functions, traits, impls, structs, and enums can each supply a complete empty Rust item shell.
//! Its outer attributes and header are emitted exactly as written; the rewritten implementation block, members, fields, or variants are attached afterward:
//!
//! ```
//! # struct Store(u32);
//! # struct AsyncStore(u32);
//! # impl Store { fn get(&self) -> u32 { self.0 } }
//! # impl AsyncStore { async fn get(&self) -> u32 { self.0 } }
//! #[ambisync::ambisync(
//!   sync(declaration {
//!       /// Blocking API.
//!       #[must_use]
//!       pub fn read(store: &Store) -> u32 {}
//!   }),
//! )]
//! /// Async API.
//! pub async fn async_read(store: &AsyncStore) -> u32 {
//!   store.get().await
//! }
//! # assert_eq!(read(&Store(2)), 2);
//! ```
//!
//! Empty `trait`, `impl`, `struct`, and `enum` bodies similarly receive the source members, fields, or variants.
//! Functions also accept the compact body-less `fn name(...);` form.
//! With only a sync declaration, the original async item retains its ordinary docs, attributes, visibility, and header.
//! Both declarations may be explicit.
//!
//! # Divergent bodies
//!
//! For code that genuinely differs between the variants, use the marker macros, which the rewriter resolves during expansion:
//!
//! ```
//! #[ambisync::ambisync(sync(name = "label"))]
//! async fn async_label() -> String {
//!   let mode = ambisync::alt!(sync => "sync", async => "async");
//!   let mut out = String::from(mode);
//!   ambisync::sync_only! { out.push_str(" (blocking)"); }
//!   ambisync::async_only! { out.push_str(" (non-blocking)"); }
//!   out
//! }
//! # assert_eq!(label(), "sync (blocking)");
//! ```
//!
//! Each arm is only type-checked in its own expansion, so the divergent code can use per-variant dependencies (`rayon` iterators in `sync_only!`, stream combinators in `async_only!`, ...).
//!
//! # Per-variant fields, enum variants, and members
//!
//! Struct fields, enum variants, and trait/impl members can be kept in only one expansion with the `#[sync_only]` / `#[async_only]` attributes:
//!
//! ```
//! #[ambisync::ambisync(sync(name = "Options"))]
//! pub struct AsyncOptions {
//!   pub common: u32,
//!   #[sync_only]
//!   pub threads: usize,
//!   #[async_only]
//!   pub concurrency: usize,
//! }
//!
//! #[ambisync::ambisync(sync(name = "Mode"))]
//! pub enum AsyncMode {
//!   Shared,
//!   #[sync_only]
//!   Blocking { threads: usize },
//!   #[async_only]
//!   Concurrent { concurrency: usize },
//! }
//!
//! #[ambisync::ambisync(sync(name = "Fetch", fn_pattern = "async_{}"))]
//! pub trait AsyncFetch {
//!   async fn async_get(&self, key: &str) -> Option<Vec<u8>>;
//!   #[sync_only]
//!   fn threads(&self) -> usize;
//!   #[async_only]
//!   fn concurrency(&self) -> usize;
//! }
//! # let _ = Options { common: 0, threads: 1 };
//! # let _ = AsyncOptions { common: 0, concurrency: 8 };
//! # let _ = Mode::Blocking { threads: 1 };
//! # let _ = AsyncMode::Concurrent { concurrency: 8 };
//! ```
//!
//! # Shared defaults
//!
//! [`macro@scope`] states recurring configuration once for an inline module; items inside opt in with a bare `#[ambisync]` / `#[ambisync::test]`:
//!
//! ```
//! # struct Store(u32);
//! # struct AsyncStore(u32);
//! # impl Store { fn get(&self) -> u32 { self.0 } }
//! # impl AsyncStore { async fn get(&self) -> u32 { self.0 } }
//! #[ambisync::scope(sync(fn_pattern = "async_{}", types(AsyncStore => Store)))]
//! mod ops {
//!   use super::*;
//!
//!   #[ambisync]
//!   pub async fn async_get(store: &AsyncStore) -> u32 {
//!       store.get().await
//!   }
//! }
//! # fn main() { assert_eq!(ops::get(&Store(7)), 7); }
//! ```
//!
//! For file modules (`mod foo;`), use [`scoped!`](macro@scoped) inside the file with a leading `#![defaults(...)]`.
//! Item-level options override the defaults; substitution tables concatenate, with item entries taking precedence.
//!
//! # Docs
//!
//! Both variants keep the source doc comments.
//! Doc text receives the same token-level substitutions as code — `types(...)`, `calls(...)`, and item renames, matched at identifier boundaries — so intra-doc links and prose follow each variant's names (`` [`AsyncStore`] `` → `` [`Store`] ``).
//! Append variant-specific paragraphs with `sync(doc = "...")` / `async(doc = "...")`.
//!
//! # Tests
//!
//! [`macro@test`] emits a sync `#[test]` and an async test from one async-written test fn:
//!
//! ```
//! #[ambisync::test(
//!   sync(name = "roundtrip"),
//!   async(feature = "async", test_attr = #[tokio::test]),
//! )]
//! async fn async_roundtrip() {
//!   assert_eq!(async { 1 + 1 }.await, 2);
//! }
//! // → #[test] fn roundtrip()  +  #[cfg(feature = "async")] #[tokio::test] async fn async_roundtrip()
//! # fn main() {}
//! ```
//!
//! # Limitations (by design)
//!
//! - Substitution is **syntactic**: paths and types are matched token-for-token
//! with no name resolution.
//! Aliased spellings must be added to the table.
//! - Inside macro invocations (`format!`, `assert_eq!`, ...) the sync rewrite
//! is best-effort: `.await` stripping, single-ident type substitutions, and `name(...)` call renames.
//! - `#[ambisync::scope]` needs an *inline* module; for `mod foo;` files use
//! [`scoped!`](macro@scoped) inside the file.

pub use ambisync_macros::{ambisync, scope, scoped, test};

/// Expression-level variant selector, resolved by `#[ambisync]` during
/// expansion.
///
/// `ambisync::alt!(sync => expr_a, async => expr_b)` becomes `expr_a` in the
/// sync expansion and `expr_b` in the async expansion. Each arm is only
/// type-checked in its own expansion.
#[macro_export]
macro_rules! alt {
    ($($tt:tt)*) => {
        ::core::compile_error!(
            "`ambisync::alt!` is only meaningful inside an item annotated with `#[ambisync]` or `#[ambisync::test]`"
        )
    };
}

/// Sync-only marker.
///
/// - As a statement macro (`ambisync::sync_only! { ... }`) inside an
///   `#[ambisync]` body: the statements are kept only in the sync expansion.
/// - As an attribute (`#[sync_only]`) on a struct field, enum variant, or
///   trait/impl member of an `#[ambisync]` item: the element is kept only in
///   the sync expansion.
///
/// Resolved by `#[ambisync]` during expansion.
#[macro_export]
macro_rules! sync_only {
    ($($tt:tt)*) => {
        ::core::compile_error!(
            "`ambisync::sync_only!` is only meaningful inside an item annotated with `#[ambisync]` or `#[ambisync::test]`"
        )
    };
}

/// Async-only marker.
///
/// - As a statement macro (`ambisync::async_only! { ... }`) inside an
///   `#[ambisync]` body: the statements are kept only in the async expansion.
/// - As an attribute (`#[async_only]`) on a struct field, enum variant, or
///   trait/impl member of an `#[ambisync]` item: the element is kept only in
///   the async expansion.
///
/// Resolved by `#[ambisync]` during expansion.
#[macro_export]
macro_rules! async_only {
    ($($tt:tt)*) => {
        ::core::compile_error!(
            "`ambisync::async_only!` is only meaningful inside an item annotated with `#[ambisync]` or `#[ambisync::test]`"
        )
    };
}
