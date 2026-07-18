# ambisync

Write the async form once; `#[ambisync::dual]` derives the sync variant
mechanically.

Libraries that ship both sync and async APIs maintain parallel code paths that
are identical except for `.await` points, item names (e.g. `read` / `async_read`,
`Store` / `AsyncStore`), test attributes, and feature gates. `ambisync`
collapses each such pair into a single async-written item:

```rust
#[ambisync::dual(
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

The annotated item is written in async form — async types, async bodies,
`.await` — so the source is real, compilable async code that rust-analyzer and
rustfmt understand. `#[dual]` clones the item and runs a syntactic rewriter
once per variant:

- **async variant** — the source as written, with helper attributes removed
  and markers resolved.
- **sync variant** — derived mechanically:
  - `async fn` → `fn`, and every `.await` is stripped;
  - `async { ... }` / `async move { ... }` blocks become immediately invoked
    closures (preserving the block's `return`/`?` boundary and `move`
    captures); async closures become ordinary closures;
  - names are mapped through `sync(name/pattern/type_pattern)`, and types and
    calls through the `sync(types(...))` / `sync(calls(...))` tables.

Naming is **always explicit** — no `async_` prefix or other convention is
assumed. Use an exact name (`sync(name = "read")`), or a pattern when one
convention covers methods or multiple items. A pattern names the async
spelling and the `{}` capture is the sync name: `pattern = "async_{}"` maps
`async_read` to `read`. The arrow form maps the `{}` stem into a different
sync spelling instead: `pattern = "async_{} => sync_{}"` maps `async_read` to
`sync_read`, `type_pattern = "Async{} => Sync{}"` maps `AsyncClient` to
`SyncClient`, and `pattern = "{} => {}_blocking"` adds an affix the async
spelling never had. The short form is sugar for `"... => {}"`.

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
| `wasm_send` | — | ✓ | with `async_trait`: native `Send` / wasm `?Send` |

`types` and `calls` are sync-only because they describe how to rewrite the
async-written source; the async expansion uses it verbatim.

## Item coverage

`#[dual]` applies to `fn`, `trait`, `impl` (inherent and trait impls),
`struct`, and `enum`:

- **Traits** — the trait is renamed via `name`/`type_pattern`, methods via
  `pattern`; default bodies follow the renames. The async trait gets
  `#[allow(async_fn_in_trait)]`, or use `async(flavor = async_trait)`.
- **Impls** — the self type and trait path are rewritten via `types(...)`;
  methods are renamed via `pattern` and calls between them follow
  automatically. The identity pattern `"{}"` keeps method names unchanged when
  the sync and async types already provide separate namespaces.
- **Structs / enums** — the type is renamed and field types substituted.

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
rust-analyzer/rustfmt; `#[dual]` resolves them during expansion, and each arm
is only type-checked in its own expansion.

Struct fields, enum variants, and trait/impl members can be kept in only one
expansion with `#[sync_only]` / `#[async_only]`:

```rust
#[ambisync::dual(sync(name = "Options"), async(feature = "async"))]
pub struct AsyncOptions {
    pub common: u32,
    #[sync_only]
    pub threads: usize,      // only on `Options`
    #[async_only]
    pub concurrency: usize,  // only on `AsyncOptions`
}

#[ambisync::dual(sync(name = "Mode"), async(feature = "async"))]
pub enum AsyncMode {
    Shared,
    #[sync_only]
    Blocking { threads: usize },       // only on `Mode`
    #[async_only]
    Concurrent { concurrency: usize }, // only on `AsyncMode`
}

#[ambisync::dual(
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

Either variant of any supported item can supply its complete outer
declaration: an ordinary Rust item with an empty body. The shell provides the
docs, attributes, visibility, name, generics, and bounds exactly as written;
the macro attaches the rewritten shared contents (a function's body, a
trait/impl's members, a struct's fields, an enum's variants):

```rust
#[ambisync::dual(
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

The declaration header is emitted verbatim — it is not passed through
`types(...)` or renaming — and it owns that variant's name, docs, and
attributes, so it cannot be combined with `name`/`doc`/`attr` for the same
variant. With only a sync declaration, the async item keeps its own header.

## Shared defaults

Recurring configuration (patterns, type tables, gates) can be stated once for
a module:

```rust
#[ambisync::scope(
    sync(pattern = "async_{}", types(AsyncStore => Store)),
    async(feature = "async"),
)]
mod ops {
    #[dual]
    async fn async_get(store: &AsyncStore, key: &str) -> Option<Vec<u8>> { ... }

    #[ambisync::test]
    async fn async_roundtrip() { ... }
}
```

`scope` requires an inline module; for file modules (`mod foo;`) use
`ambisync::content! { #![defaults(...)] ... }` inside the file. Item-level
options override the defaults; substitution tables concatenate, with item
entries taking precedence.

## Docs

Both variants keep the source doc comments. Doc text receives the same
token-level substitutions as code — `types(...)`, `calls(...)`, and item
renames, matched at identifier boundaries — so intra-doc links and prose
follow each variant's names: [`AsyncStore`] becomes [`Store`],
[`mod_x::AsyncThing`] becomes [`mod_y::Thing`], [`async_read`] becomes
[`read`]. Append variant-specific paragraphs with `sync(doc = "...")` /
`async(doc = "...")`, and use `#[sync_attr(...)]` / `#[async_attr(...)]` on
items and methods for attributes that belong to one variant only.

## wasm and `Send`

The facade exports `MaybeSend`/`MaybeSync` — equal to `Send`/`Sync` except on
`wasm32`, where they are no-op bounds. With
`async(flavor = async_trait, wasm_send)`, async traits/impls get the standard
pairing:

```rust
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
```

## Limitations (by design)

- **Syntactic matching only.** Substitutions match paths and types
  token-for-token — no name resolution, no type inference. Aliased spellings
  must be added to the table.
- **Macro bodies are rewritten best-effort** in the sync expansion: `.await`
  stripping, single-ident type substitutions, and `name(...)` call renames.
  Anything fancier inside `format!`-likes needs `alt!`.
- **Whole-item expansion.** `#[dual]` always yields both variants of the whole
  item — no partial extraction. The escape hatch is writing the two items by
  hand; the macro is never mandatory.

## Workspace layout

- [`ambisync/`](ambisync/) — facade crate: macro re-exports, marker macros,
  `MaybeSend`/`MaybeSync`.
- [`ambisync_macros/`](ambisync_macros/) — the proc-macro implementation: one
  `syn::visit_mut`-based rewriter run once per variant over a cloned item.
- [`ambisync_tests/`](ambisync_tests/) — integration tests with real `async` and
  `sync` features, exercising the emitted cfg gates under every feature
  combination.

License: MIT OR Apache-2.0. MSRV: 1.91.
