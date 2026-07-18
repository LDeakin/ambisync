# ambisync

Write the async form once; `#[ambisync]` derives the sync variant
mechanically.

Libraries that ship both sync and async APIs maintain parallel code paths.
These paths are identical except for `.await` points, item names (for example,
`read` / `async_read` and `Store` / `AsyncStore`), test attributes, and feature
gates.
`ambisync` collapses each such pair into a single async-written item:

```rust
use ambisync::ambisync;

#[ambisync(
    sync(name = "read", types(AsyncStore => Store)),
    async(feature = "async"),
)]
pub async fn async_read(store: &AsyncStore, key: &str) -> Option<Vec<u8>> {
    store.get(key).await
}
```

which expands to:

```rust
#[cfg(feature = "async")]
pub async fn async_read(store: &AsyncStore, key: &str) -> Option<Vec<u8>> {
    store.get(key).await
}

pub fn read(store: &Store, key: &str) -> Option<Vec<u8>> {
    store.get(key)
}
```

## How it works

Write the annotated item in async form, using async types, async bodies, and
`.await`.
The source is therefore real, compilable async code that rust-analyzer and
rustfmt understand.
`#[ambisync]` clones the item and runs a syntactic rewriter once per variant:

- **async variant** — the source as written.
  Helper attributes are removed and markers are resolved.
- **sync variant** — derived mechanically:
  - `async fn` → `fn`, and every `.await` is stripped.
  - `async { ... }` / `async move { ... }` blocks become immediately invoked
    closures.
    The transformation preserves the block's `return`/`?` boundary and `move`
    captures.
  - Async closures become ordinary closures.
  - Names are mapped through `sync(name/pattern/type_pattern)`.
  - Types and calls are mapped through the `sync(types(...))` /
    `sync(calls(...))` tables.

Naming is **always explicit**.
No `async_` prefix or other convention is assumed.
Use an exact name (`sync(name = "read")`) or a pattern when one convention
covers methods or multiple items.
A pattern names the async
spelling and the `{}` capture is the sync name: `pattern = "async_{}"` maps
`async_read` to `read`.
The arrow form maps the `{}` stem into a different sync spelling.
For example, `pattern = "async_{} => sync_{}"` maps `async_read` to `sync_read`.
Similarly, `type_pattern = "Async{} => Sync{}"` maps `AsyncClient` to
`SyncClient`.
`pattern = "{} => {}_blocking"` adds an affix that the async spelling never had.
The short form is sugar for `"... => {}"`.

The attribute always runs and emits the `#[cfg(feature = ...)]` gates itself —
never wrap it in `cfg_attr` — so the expansion stays correct whether both,
either, or neither variant is feature-gated.

## Options

| Option | `sync(...)` | `async(...)` | Effect |
|---|:-:|:-:|---|
| `feature = "..."` | ✓ | ✓ | cfg-gate the variant |
| `name = "..."` | ✓ | ✓ | exact output item name |
| `doc = "..."` / `attr = #[...]` | ✓ | ✓ | variant-specific docs and item attributes |
| `test_attr = #[...]` | ✓ | ✓ | test attribute (defaults `#[test]` / `#[tokio::test]`) |
| `declaration { ... }` | ✓ | ✓ | complete outer declaration for the variant |
| `pattern = "async_{}"` | ✓ | — | derive sync fn/method names; `{}` captures the sync name |
| `pattern = "async_{} => sync_{}"` | ✓ | — | arrow form: map the async spelling to a sync spelling sharing the `{}` stem |
| `type_pattern = "Async{}"` | ✓ | — | derive sync type/trait names; short and arrow forms as above |
| `types(A => B, ...)` | ✓ | — | type substitutions, token-level, generic-aware; paths may be module-qualified (`mod_x::A => mod_y::B`) |
| `calls(f => g, ...)` | ✓ | — | call renames (the `.await` lives in the source) |
| `flavor = async_fn \| async_trait` | — | ✓ | async trait/impl emission style |
| `send = true \| false \| cfg(...)` | — | ✓ | with `async_trait`: `Send`-ness of the emission (default `Send`) |

`types` and `calls` are sync-only because they describe how to rewrite the
async-written source; the async expansion uses it verbatim.

## Item coverage

`#[ambisync]` applies to `fn`, `trait`, `impl` (inherent and trait impls),
`struct`, and `enum`.

- **Traits** — Rename the trait via `name`/`type_pattern` and methods via
  `pattern`.
  Default bodies follow the renames.
  The async trait gets `#[allow(async_fn_in_trait)]`, or you can use
  `async(flavor = async_trait)`.
- **Impls** — Rewrite the self type and trait path via `types(...)`.
  Rename methods via `pattern`.
  Calls between renamed methods follow automatically.
  The identity pattern `"{}"` keeps method names unchanged when the sync and
  async types already provide separate namespaces.
- **Structs / enums** — The type is renamed and field types are substituted.

## Divergent code

For the minority of code that genuinely differs between variants, use the
marker macros:

```rust
let workers = ambisync::alt!(
    sync  => rayon::current_num_threads(),
    async => options.concurrency(),
);

ambisync::sync_only! { items.into_par_iter().try_for_each(process)?; }
ambisync::async_only! {
    stream::iter(items).map(process).buffer_unordered(n).try_collect().await?;
}
```

These are real exported macros, so unexpanded code still parses for
rust-analyzer/rustfmt; `#[ambisync]` resolves them during expansion, and each arm
is only type-checked in its own expansion.

Struct fields, enum variants, and trait/impl members can be kept in only one
expansion with `#[sync_only]` / `#[async_only]`:

```rust
#[ambisync(sync(name = "Options"), async(feature = "async"))]
pub struct AsyncOptions {
    pub common: u32,
    #[sync_only]
    pub threads: usize,      // only on `Options`
    #[async_only]
    pub concurrency: usize,  // only on `AsyncOptions`
}

#[ambisync(sync(name = "Mode"), async(feature = "async"))]
pub enum AsyncMode {
    Shared,
    #[sync_only]
    Blocking { threads: usize },       // only on `Mode`
    #[async_only]
    Concurrent { concurrency: usize }, // only on `AsyncMode`
}

#[ambisync(
    sync(name = "Fetch", pattern = "async_{}"),
    async(feature = "async"),
)]
pub trait AsyncFetch {
    async fn async_get(&self, key: &str) -> Option<Vec<u8>>;
    #[sync_only]
    fn threads(&self) -> usize;        // only on `Fetch`
    #[async_only]
    fn concurrency(&self) -> usize;    // only on `AsyncFetch`
}
```

## Tests

`#[ambisync::test]` emits a sync `#[test]` plus an async test from one
async-written test fn:

```rust
#[ambisync::test(
    sync(name = "roundtrip", calls(async_fixture => fixture)),
    async(feature = "async", test_attr = #[tokio::test]),
)]
async fn async_roundtrip() {
    let store = async_fixture().await;
    assert_eq!(store.get("k").await, expected); // .await stripped inside macros too
}
```

## Explicit declarations

Either variant of any supported item can supply its complete outer declaration.
This is an ordinary Rust item with an empty body.
The shell provides the docs, attributes, visibility, name, generics, and
bounds exactly as written.
The macro attaches the rewritten shared contents: a function's body, a
trait/impl's members, a struct's fields, or an enum's variants.

```rust
#[ambisync(
    sync(declaration {
        /// Read using blocking I/O.
        #[must_use]
        pub fn read(store: &Store, key: &str) -> Option<Vec<u8>> {}
    }),
    async(feature = "async"),
)]
/// Read using asynchronous I/O.
pub async fn async_read(store: &AsyncStore, key: &str) -> Option<Vec<u8>> {
    store.get(key).await
}
```

The declaration header is emitted verbatim.
It is not passed through `types(...)` or renaming.
It owns that variant's name, docs, and attributes.
It therefore cannot be combined with `name`/`doc`/`attr` for the same variant.
With only a sync declaration, the async item keeps its own header.

## Shared defaults

Recurring configuration (patterns, type tables, gates) can be stated once for
a module:

```rust
#[ambisync::scope(
    sync(pattern = "async_{}", types(AsyncStore => Store)),
    async(feature = "async"),
)]
mod ops {
    #[ambisync]
    async fn async_get(store: &AsyncStore, key: &str) -> Option<Vec<u8>> { ... }

    #[ambisync::test]
    async fn async_roundtrip() { ... }
}
```

`scope` requires an inline module.
For file modules (`mod foo;`), use
`ambisync::scoped! { #![defaults(...)] ... }` inside the file.
Item-level options override the defaults.
Substitution tables concatenate, with item entries taking precedence.

## Docs

Both variants keep the source doc comments.
Doc text receives the same token-level substitutions as code.
These include `types(...)`, `calls(...)`, and item renames matched at
identifier boundaries.
Intra-doc links and prose therefore follow each variant's names:
[`AsyncStore`] becomes [`Store`], [`mod_x::AsyncThing`] becomes
[`mod_y::Thing`], and [`async_read`] becomes [`read`].
Append variant-specific paragraphs with `sync(doc = "...")` /
`async(doc = "...")`.
Use `#[sync_attr(...)]` / `#[async_attr(...)]` for attributes that belong to
one variant only.

## wasm and `Send`

With `async(flavor = async_trait)`, the `send` option sets the `Send`-ness of
the emitted async traits/impls.
Use `send = true` (the default), `send = false` (`?Send`), or
`send = cfg(PRED)` (`Send` where the predicate holds and `?Send` elsewhere).
The standard native-`Send` / wasm-`?Send` pairing is:

```rust
#[ambisync(
    sync(name = "Fetch", pattern = "async_{}"),
    async(flavor = async_trait, send = cfg(not(target_arch = "wasm32"))),
)]
```

which emits:

```rust
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
```

The predicate is yours, so thread-capable wasm targets (e.g.
`wasm32-wasip1-threads`) and feature-driven policies
(`send = cfg(feature = "send")`) need no special support.

For hand-written bounds that must relax on single-threaded wasm, define the
usual marker traits in your own crate:

```rust
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> MaybeSend for T {}

#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}
```

`ambisync` deliberately does not export these: in a public bound they would
make `ambisync` a semver-relevant dependency of your API, and the cfg policy
should stay yours (e.g.
`wasm32-wasip1-threads` has real threads and may want
a real `Send`).

## Limitations (by design)

- **Syntactic matching only.** Substitutions match paths and types token-for-token — no name resolution, no type inference.
Aliased spellings must be added to the table.
- **Macro bodies are rewritten best-effort** in the sync expansion: `.await` stripping, single-ident type substitutions, and `name(...)` call renames.
Anything fancier inside `format!`-likes needs `alt!`.
- **Whole-item expansion.** `#[ambisync]` always yields both variants of the whole item — no partial extraction.
The escape hatch is writing the two items by hand; the macro is never mandatory.

## Workspace layout

- [`ambisync/`](ambisync/) — facade crate: macro re-exports and marker macros.
- [`ambisync_macros/`](ambisync_macros/) — the proc-macro implementation: one `syn::visit_mut`-based rewriter run once per variant over a cloned item.
- [`ambisync_tests/`](ambisync_tests/) — integration tests with real `async` and `sync` features, exercising the emitted cfg gates under every feature combination.

License: MIT OR Apache-2.0.
MSRV: 1.91.
