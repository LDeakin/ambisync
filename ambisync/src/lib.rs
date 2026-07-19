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
//!   - Names, type references and calls are all mapped through `sync(name = ...)` and the
//!     `sync(fns(...))` / `sync(types(...))` rename tables.
//!
//! Naming is **always explicit**.
//! No `async_` prefix or other convention is assumed.
//! Use an exact name (`sync(name = "read")`) or the two rename tables: `fns(...)` for the fn namespace (functions, methods, calls) and `types(...)` for the type namespace (types, traits).
//! Entries are exact mappings (`async_read => read`, `AsyncStore => Store`) or string-literal patterns.
//! A pattern names the async spelling and the `{}` capture is the sync name: `fns("async_{}")` maps `async_read` to `read`.
//! The arrow form maps the `{}` stem into a different sync spelling.
//! For example, `fns("async_{} => sync_{}")` maps `async_read` to `sync_read`.
//! Similarly, `types("Async{} => Sync{}")` maps `AsyncClient` to `SyncClient`.
//! `fns("{} => {}_blocking")` adds an affix that the async spelling never had.
//! The short form is sugar for `"... => {}"`.
//! One table drives everything: the entry that names the annotated item also renames matching references — types, calls, paths, and doc mentions — in the sync expansion.
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
//! | `types(A => B, ...)` | ✓ | — | type-namespace rename table: names the annotated type/trait and substitutes type references (token-level, generic-aware; paths may be module-qualified, `mod_x::A => mod_y::B`) |
//! | `types("Async{}", ...)` | ✓ | — | pattern entry: covers every matching type ident (`AsyncStore` → `Store`) |
//! | `fns(f => g, ...)` | ✓ | — | fn-namespace rename table: names the annotated fn/methods and renames calls (the `.await` lives in the source) |
//! | `fns("{}_async", ...)` | ✓ | — | pattern entry: covers every matching fn name (`read_async` → `read`) |
//! | `#[sync_name(name)]` | ✓ | — | exact sync name for one trait/impl method, including a non-async method in an async API |
//! | `#[sync_signature(fn ...)]` | ✓ | — | complete verbatim sync signature for one trait/impl method; the body remains shared |
//! | `#[sync_bounds(...)]` / `#[async_bounds(...)]` | ✓ | ✓ | replace a generic type parameter's bounds for one variant; an empty list removes all bounds |
//! | `flavor = async_fn \| async_trait` | — | ✓ | async trait/impl emission style |
//! | `send = true \| false \| cfg(...)` | — | ✓ | with `async_trait`: `Send`-ness of the emission (default `Send`) |
//!
//! `types` and `fns` are sync-only because they describe how to rewrite the async-written source; the async expansion uses it verbatim.
//!
//! A pattern matches whole identifiers only: a bare type name, the leading segment of a path (`AsyncStore::new`), a fn/method name at its declaration or a call site, or the corresponding doc-text mentions.
//! Exact entries always win over patterns; among patterns, the first match wins.
//! An identity entry (`fns(async_probe => async_probe)`) therefore pins a name that a pattern would otherwise rewrite.
//!
//! # Item coverage
//!
//! `#[ambisync]` applies to `fn`, `trait`, `impl` (inherent and trait impls), `struct`, and `enum`.
//!
//! - **Traits**: Rename the trait via `name`/`types(...)` and methods via `fns(...)`.
//!   Default bodies follow the renames.
//!   Use `#[sync_name(name)]` for an exact per-method override, including methods that are not themselves `async fn`.
//!   The async trait gets `#[allow(async_fn_in_trait)]`, or you can use `async(flavor = async_trait)`.
//! - **Impls**: Rewrite the self type and trait path via `types(...)`.
//!   Rename methods via `fns(...)`.
//!   `#[sync_name(name)]` overrides `fns(...)` for one method and rewrites calls to that method.
//!   Calls between renamed methods follow automatically.
//!   The identity pattern `fns("{}")` keeps method names unchanged when the sync and async types already provide separate namespaces.
//! - **Structs / Enums**: The type is renamed and field types are substituted.
//!
//! When sync and async entry points must coexist on the same public trait, use
//! [`macro@paired`] instead. It leaves ordinary trait members alone and emits a synchronous
//! sibling beside every async method:
//!
//! ```
//! # struct Reader; struct AsyncReader;
//! # impl Reader { fn get(&self) -> u32 { 1 } }
//! # impl AsyncReader { async fn get(&self) -> u32 { 1 } }
//! #[ambisync::paired(
//!     sync(fns("async_{}"), types("Async{}")),
//!     async(feature = "async", flavor = async_trait),
//! )]
//! trait ReadApi: Sync {
//!     fn label(&self) -> &'static str { "reader" }
//!
//!     async fn async_read(&self, reader: &AsyncReader) -> u32 {
//!         reader.get().await
//!     }
//! }
//! ```
//!
//! This produces `read(&Reader)` and the feature-gated `async_read(&AsyncReader)` in `ReadApi`.
//! The `sync(fns(...))` and `sync(types(...))` tables, `async(feature/flavor/send)`,
//! `sync(feature)`, and method-level `#[sync_name]`, `#[sync_signature]`, `#[sync_attr]`, and
//! `#[async_attr]` options work as they do with [`macro@ambisync`]. Item-level renames,
//! declarations, docs, and attributes do not apply because [`macro@paired`] emits one trait.
//!
//! `#[sync_name(name)]` states one method's exact sync name at the method itself, taking precedence over the `fns(...)` table.
//! It also covers methods that are not `async fn`, which the table never renames: a synchronous method may still carry an async name, for example when it returns an async-capable reader rather than a future.
//! Calls and doc mentions follow the override:
//!
//! ```
//! # use ambisync::ambisync;
//! # pub struct Store; pub struct AsyncStore;
//! # pub struct Reader; pub struct AsyncReader;
//! # impl Reader { fn get(&self) -> u32 { 7 } }
//! # impl AsyncReader { async fn get(&self) -> u32 { 7 } }
//! #[ambisync(sync(fns("async_{}"), types("Async{}")))]
//! impl AsyncStore {
//!     // Synchronous, but it belongs to the async API: it returns the async reader, not a future.
//!     #[sync_name(reader)]
//!     fn async_reader(&self) -> AsyncReader {
//!         AsyncReader
//!     }
//!
//!     async fn async_read(&self) -> u32 {
//!         self.async_reader().get().await
//!     }
//! }
//!
//! // Emits the impl as written, plus:
//! //   impl Store {
//! //       fn reader(&self) -> Reader { Reader }
//! //       fn read(&self) -> u32 { self.reader().get() }
//! //   }
//! # assert_eq!(Store.reader().get(), 7);
//! # assert_eq!(Store.read(), 7);
//! ```
//!
//! When a method's sync and async signatures differ beyond naming and asyncness, use
//! `#[sync_signature(fn ...)]`. The declared sync signature is emitted verbatim while the method
//! body is still derived from the async-written source. This is useful for APIs whose async
//! futures require input lifetimes to be tied together even though the sync call does not:
//!
//! ```
//! # use ambisync::ambisync;
//! #[ambisync(sync(types("Async{}")), async(feature = "async"))]
//! trait AsyncBorrow {
//!     #[sync_signature(fn borrow<'a>(&'a self, input: &str) -> &'a str)]
//!     async fn borrow<'a>(&'a self, input: &'a str) -> &'a str;
//! }
//! ```
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
//! #[ambisync::ambisync(sync(name = "Fetch", fns("async_{}")))]
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
//! The full option set of `#[ambisync]` applies, so fixtures can be mapped with `sync(fns(...))` / `sync(types(...))`.
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
//! #[ambisync::scope(sync(fns("async_{}"), types(AsyncStore => Store)))]
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
//! Rename tables concatenate, with item entries taking precedence.
//!
//! # Docs
//!
//! Both variants keep the source doc comments.
//! Doc text receives the same token-level substitutions as code.
//! These include the `types(...)` / `fns(...)` tables and item renames, matched at identifier boundaries.
//! Intra-doc links and prose therefore follow each variant's names: \[`AsyncStore`\] becomes \[`Store`\], \[`mod_x::AsyncThing`\] becomes \[`mod_y::Thing`\], and \[`async_read`\] becomes \[`read`\].
//! Append variant-specific paragraphs with `sync(doc = "...")` / `async(doc = "...")`.
//! Use `#[sync_attr(...)]` / `#[async_attr(...)]` for attributes that belong to one variant only.
//! Generic type parameter bounds can likewise diverge without duplicating an item. The source bounds
//! are used unless that variant has an explicit override; an empty override removes all bounds:
//!
//! ```
//! # use ambisync::ambisync;
//! # trait MaybeSend {}
//! #[ambisync(sync(fns("async_{}")))]
//! async fn async_store<#[sync_bounds()] T: MaybeSend>(value: T) {
//!     let _ = value;
//! }
//! # store(String::new());
//! ```
//!
//! # wasm and `Send`
//!
//! With `async(flavor = async_trait)`, the `send` option sets the `Send`-ness of the emitted async traits/impls.
//! Use `send = true` (the default), `send = false` (`?Send`), or `send = cfg(PRED)` (`Send` where the predicate holds and `?Send` elsewhere).
//! The standard native-`Send` / wasm-`?Send` pairing is:
//!
//! ```
//! #[ambisync::ambisync(
//!     sync(name = "Fetch", fns("async_{}")),
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

pub use ambisync_macros::{ambisync, paired, scope, scoped, test};

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
