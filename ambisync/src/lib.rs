//! Write the async form once; [`macro@ambisync`] derives the sync variant mechanically.
//!
//! Libraries that ship both sync and async APIs maintain parallel code paths.
//! These paths are identical except for `.await` points, item names (for example, `read` / `async_read` and `Store` / `AsyncStore`), test attributes, and feature gates.
//! `ambisync` collapses each such pair into a single async-written item:
//!
//! ```
//! struct Store(u32);
//! struct AsyncStore(u32);
//! impl Store { fn get(&self) -> u32 { self.0 } }
//! impl AsyncStore { async fn get(&self) -> u32 { self.0 } }
//!
//! #[ambisync::ambisync(
//!     sync(name = "read", types(AsyncStore => Store)),
//!     async(feature = "async"),
//! )]
//! async fn async_read(store: &AsyncStore) -> u32 {
//!     store.get().await
//! }
//!
//! // Emits `async_read` as written under #[cfg(feature = "async")], plus:
//! //   fn read(store: &Store) -> u32 { store.get() }
//! # assert_eq!(read(&Store(1)), 1);
//! ```
//!
//! # How it works
//!
//! Write the annotated item in async form, using async types, async bodies, and `.await`.
//! The source is therefore real, compilable async code that rust-analyzer and rustfmt understand.
//! `#[ambisync]` clones the item and runs a syntactic rewriter once per variant:
//!
//! - **async variant**: the source as written.
//!   Helper attributes are removed and markers are resolved.
//! - **sync variant**: derived mechanically:
//!   - `async fn` → `fn`, and every `.await` is stripped.
//!   - `async { ... }` / `async move { ... }` blocks become immediately invoked closures.
//!     The transformation preserves the block's `return`/`?` boundary and `move` captures.
//!   - Async closures become ordinary closures.
//!   - Names are mapped through `sync(name/fn_pattern/type_pattern)`.
//!   - Types and calls are mapped through the `sync(types(...))` / `sync(calls(...))` tables.
//!
//! Naming is **always explicit**.
//! No `async_` prefix or other convention is assumed.
//! Use an exact name (`sync(name = "read")`) or a pattern when one convention covers methods or multiple items.
//! A pattern names the async spelling and the `{}` capture is the sync name: `fn_pattern = "async_{}"` maps `async_read` to `read`.
//! The arrow form maps the `{}` stem into a different sync spelling.
//! For example, `fn_pattern = "async_{} => sync_{}"` maps `async_read` to `sync_read`.
//! Similarly, `type_pattern = "Async{} => Sync{}"` maps `AsyncClient` to `SyncClient`.
//! `fn_pattern = "{} => {}_blocking"` adds an affix that the async spelling never had.
//! The short form is sugar for `"... => {}"`.
//!
//! The attribute always runs and emits the `#[cfg(feature = ...)]` gates itself — never wrap it in `cfg_attr` — so the expansion stays correct whether both, either, or neither variant is feature-gated.
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
//! | `types(A => B, ...)` | ✓ | — | type substitutions, token-level, generic-aware; paths may be module-qualified (`mod_x::A => mod_y::B`) |
//! | `calls(f => g, ...)` | ✓ | — | call renames (the `.await` lives in the source) |
//! | `flavor = async_fn \| async_trait` | — | ✓ | async trait/impl emission style |
//! | `send = true \| false \| cfg(...)` | — | ✓ | with `async_trait`: `Send`-ness of the emission (default `Send`) |
//!
//! `types` and `calls` are sync-only because they describe how to rewrite the async-written source; the async expansion uses it verbatim.
//!
//! # Item coverage
//!
//! `#[ambisync]` applies to `fn`, `trait`, `impl` (inherent and trait impls), `struct`, and `enum`.
//!
//! - **Traits**: Rename the trait via `name`/`type_pattern` and methods via `fn_pattern`.
//!   Default bodies follow the renames.
//!   The async trait gets `#[allow(async_fn_in_trait)]`, or you can use `async(flavor = async_trait)`.
//! - **Impls**: Rewrite the self type and trait path via `types(...)`.
//!   Rename methods via `fn_pattern`.
//!   Calls between renamed methods follow automatically.
//!   The identity pattern `"{}"` keeps method names unchanged when the sync and async types already provide separate namespaces.
//! - **Structs / Enums**: The type is renamed and field types are substituted.
//!
//! # Divergent code
//!
//! For the minority of code that genuinely differs between variants, use the marker macros:
//!
//! ```
//! #[ambisync::ambisync(sync(name = "label"))]
//! async fn async_label() -> String {
//!     let mode = ambisync::alt!(sync => "sync", async => "async");
//!     let mut out = String::from(mode);
//!     ambisync::sync_only! { out.push_str(" (blocking)"); }
//!     ambisync::async_only! { out.push_str(" (non-blocking)"); }
//!     out
//! }
//! # assert_eq!(label(), "sync (blocking)");
//! ```
//!
//! These are real exported macros, so unexpanded code still parses for rust-analyzer/rustfmt; `#[ambisync]` resolves them during expansion, and each arm is only type-checked in its own expansion, so divergent code can use per-variant dependencies (`rayon` iterators in [`sync_only!`], stream combinators in [`async_only!`], ...).
//!
//! Struct fields, enum variants, and trait/impl members can be kept in only one expansion with `#[sync_only]` / `#[async_only]`:
//!
//! ```
//! #[ambisync::ambisync(sync(name = "Options"))]
//! pub struct AsyncOptions {
//!     pub common: u32,
//!     #[sync_only]
//!     pub threads: usize,      // only on `Options`
//!     #[async_only]
//!     pub concurrency: usize,  // only on `AsyncOptions`
//! }
//!
//! #[ambisync::ambisync(sync(name = "Mode"))]
//! pub enum AsyncMode {
//!     Shared,
//!     #[sync_only]
//!     Blocking { threads: usize },       // only on `Mode`
//!     #[async_only]
//!     Concurrent { concurrency: usize }, // only on `AsyncMode`
//! }
//!
//! #[ambisync::ambisync(sync(name = "Fetch", fn_pattern = "async_{}"))]
//! pub trait AsyncFetch {
//!     async fn async_get(&self, key: &str) -> Option<Vec<u8>>;
//!     #[sync_only]
//!     fn threads(&self) -> usize;        // only on `Fetch`
//!     #[async_only]
//!     fn concurrency(&self) -> usize;    // only on `AsyncFetch`
//! }
//! # let _ = Options { common: 0, threads: 1 };
//! # let _ = AsyncOptions { common: 0, concurrency: 8 };
//! # let _ = Mode::Blocking { threads: 1 };
//! # let _ = AsyncMode::Concurrent { concurrency: 8 };
//! ```
//!
//! # Tests
//!
//! [`macro@test`] emits a sync `#[test]` plus an async test from one async-written test fn:
//!
//! ```
//! #[ambisync::test(
//!     sync(name = "roundtrip"),
//!     async(feature = "async", test_attr = #[tokio::test]),
//! )]
//! async fn async_roundtrip() {
//!     assert_eq!(async { 1 + 1 }.await, 2); // .await stripped inside macros too
//! }
//! // → #[test] fn roundtrip()  +  #[cfg(feature = "async")] #[tokio::test] async fn async_roundtrip()
//! # fn main() {}
//! ```
//!
//! The full option set of `#[ambisync]` applies, so fixtures can be mapped with `sync(calls(...))` / `sync(types(...))`.
//!
//! # Explicit declarations
//!
//! Either variant of any supported item can supply its complete outer declaration.
//! This is an ordinary Rust item with an empty body.
//! The shell provides the docs, attributes, visibility, name, generics, and bounds exactly as written.
//! The macro attaches the rewritten shared contents: a function's body, a trait/impl's members, a struct's fields, or an enum's variants.
//! Functions also accept the compact body-less `fn name(...);` form.
//!
//! ```
//! # struct Store(u32);
//! # struct AsyncStore(u32);
//! # impl Store { fn get(&self) -> u32 { self.0 } }
//! # impl AsyncStore { async fn get(&self) -> u32 { self.0 } }
//! #[ambisync::ambisync(
//!     sync(declaration {
//!         /// Read using blocking I/O.
//!         #[must_use]
//!         pub fn read(store: &Store) -> u32 {}
//!     }),
//! )]
//! /// Read using asynchronous I/O.
//! pub async fn async_read(store: &AsyncStore) -> u32 {
//!     store.get().await
//! }
//! # assert_eq!(read(&Store(2)), 2);
//! ```
//!
//! The declaration header is emitted verbatim.
//! It is not passed through `types(...)` or renaming.
//! It owns that variant's name, docs, and attributes.
//! It therefore cannot be combined with `name`/`doc`/`attr` for the same variant.
//! With only a sync declaration, the async item keeps its own header.
//! Both declarations may be explicit.
//!
//! # Shared defaults
//!
//! [`macro@scope`] states recurring configuration (patterns, type tables, gates) once for a module:
//!
//! ```
//! # struct Store(u32);
//! # struct AsyncStore(u32);
//! # impl Store { fn get(&self) -> u32 { self.0 } }
//! # impl AsyncStore { async fn get(&self) -> u32 { self.0 } }
//! #[ambisync::scope(sync(fn_pattern = "async_{}", types(AsyncStore => Store)))]
//! mod ops {
//!     use super::*;
//!
//!     #[ambisync]
//!     pub async fn async_get(store: &AsyncStore) -> u32 {
//!         store.get().await
//!     }
//! }
//! # fn main() { assert_eq!(ops::get(&Store(7)), 7); }
//! ```
//!
//! Items opt in with a bare `#[ambisync]` / `#[ambisync::test]`.
//! `scope` requires an inline module.
//! For file modules (`mod foo;`), use [`scoped!`](macro@scoped) inside the file with a leading `#![defaults(...)]`.
//! Item-level options override the defaults.
//! Substitution tables concatenate, with item entries taking precedence.
//!
//! # Docs
//!
//! Both variants keep the source doc comments.
//! Doc text receives the same token-level substitutions as code.
//! These include `types(...)`, `calls(...)`, and item renames matched at identifier boundaries.
//! Intra-doc links and prose therefore follow each variant's names: \[`AsyncStore`\] becomes \[`Store`\], \[`mod_x::AsyncThing`\] becomes \[`mod_y::Thing`\], and \[`async_read`\] becomes \[`read`\].
//! Append variant-specific paragraphs with `sync(doc = "...")` / `async(doc = "...")`.
//! Use `#[sync_attr(...)]` / `#[async_attr(...)]` for attributes that belong to one variant only.
//!
//! # wasm and `Send`
//!
//! With `async(flavor = async_trait)`, the `send` option sets the `Send`-ness of the emitted async traits/impls.
//! Use `send = true` (the default), `send = false` (`?Send`), or `send = cfg(PRED)` (`Send` where the predicate holds and `?Send` elsewhere).
//! The standard native-`Send` / wasm-`?Send` pairing is:
//!
//! ```
//! #[ambisync::ambisync(
//!     sync(name = "Fetch", fn_pattern = "async_{}"),
//!     async(flavor = async_trait, send = cfg(not(target_arch = "wasm32"))),
//! )]
//! pub trait AsyncFetch {
//!     async fn async_get(&self) -> u32;
//! }
//!
//! // The async trait is emitted with:
//! //   #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
//! //   #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
//! ```
//!
//! The predicate is yours, so thread-capable wasm targets (e.g. `wasm32-wasip1-threads`) and feature-driven policies (`send = cfg(feature = "send")`) need no special support.
//!
//! For hand-written bounds that must relax on single-threaded wasm, define the usual marker traits in your own crate:
//!
//! ```
//! #[cfg(not(target_arch = "wasm32"))]
//! pub trait MaybeSend: Send {}
//! #[cfg(not(target_arch = "wasm32"))]
//! impl<T: Send + ?Sized> MaybeSend for T {}
//!
//! #[cfg(target_arch = "wasm32")]
//! pub trait MaybeSend {}
//! #[cfg(target_arch = "wasm32")]
//! impl<T: ?Sized> MaybeSend for T {}
//! ```
//!
//! `ambisync` deliberately does not export these: in a public bound they would make `ambisync` a semver-relevant dependency of your API, and the cfg policy should stay yours (e.g. `wasm32-wasip1-threads` has real threads and may want a real `Send`).
//!
//! # Limitations
//!
//! - **Syntactic matching only.** Substitutions match paths and types token-for-token
//!   - There is no name resolution or type inference, so aliased spellings must be added to the table.
//! - **Macro bodies are rewritten best-effort** in the sync expansion: `.await` stripping, single-ident type substitutions, and `name(...)` call renames.
//!   - Anything fancier inside `format!`-likes needs [`alt!`].
//!
//! ## Licence
//! `ambisync` is licensed under either of
//!  - the Apache License, Version 2.0 [LICENSE-APACHE](https://docs.rs/crate/ambisync/latest/source/LICENCE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0> or
//!  - the MIT license [LICENSE-MIT](https://docs.rs/crate/ambisync/latest/source/LICENCE-MIT) or <http://opensource.org/licenses/MIT>, at your option.
//!
//! Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

pub use ambisync_macros::{ambisync, scope, scoped, test};

/// Expression-level variant selector, resolved by `#[ambisync]` during expansion.
///
/// `ambisync::alt!(sync => expr_a, async => expr_b)` becomes `expr_a` in the sync expansion and `expr_b` in the async expansion.
/// Each arm is only type-checked in its own expansion.
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
/// - As a statement macro (`ambisync::sync_only! { ... }`) inside an `#[ambisync]` body: the statements are kept only in the sync expansion.
/// - As an attribute (`#[sync_only]`) on a struct field, enum variant, or trait/impl member of an `#[ambisync]` item: the element is kept only in the sync expansion.
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
/// - As a statement macro (`ambisync::async_only! { ... }`) inside an `#[ambisync]` body: the statements are kept only in the async expansion.
/// - As an attribute (`#[async_only]`) on a struct field, enum variant, or trait/impl member of an `#[ambisync]` item: the element is kept only in the async expansion.
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
