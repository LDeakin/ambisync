# unasync

Write sync/async-generic Rust: **author the async form once, derive the sync
variant mechanically**.

## The problem

Libraries that ship both sync and async APIs (e.g. `zarrs`) maintain parallel
code paths that are near-identical except for `.await` points, renamed items
(`retrieve_chunk` / `async_retrieve_chunk`, `Foo` / `AsyncFoo`), substituted
types (`ReadableStorageTraits` / `AsyncReadableStorageTraits`), test attributes
(`#[test]` / `#[tokio::test]`), and feature gates. `unasync` collapses each
such pair into a single item.

## Model

The annotated item is written **entirely as the async variant** — async names,
async types, `.await` — so the source is real, compilable async code that
rust-analyzer understands. `#[unasync::dual]` emits:

- the **async variant**: the source as written (helper attributes removed,
  markers resolved), optionally gated with `async(feature = "...")`;
- the **sync variant**: `async` removed, every `.await` stripped, names mapped
  back through the explicit `rename`/`rename_types` patterns and the
  `types(...)`/`calls(...)` substitution tables, optionally gated with
  `sync(feature = "...")`.

Renaming is **always explicit** — `unasync` does not assume an `async_` prefix
or any other convention (prefix vs suffix is a project decision). Give a
pattern (`rename = "async_{}"`, `rename = "{}_async"`, `rename_types =
"Async{}"`, ...) or `keep_self`; omitting both is a compile error whenever a
rename would be needed.

The attribute is **never wrapped in `cfg_attr`** — it always runs and emits
the cfg gates itself, so the expansion stays correct under every feature
combination (both, either, or neither variant gated).

```rust
#[unasync::dual(
    async(feature = "async"),
    rename = "async_{}",
    types(&Arc<dyn AsyncBytesPartialDecoderTraits> => &Arc<dyn BytesPartialDecoderTraits>),
)]
async fn async_partial_decode<'a>(
    input_handle: &Arc<dyn AsyncBytesPartialDecoderTraits>,
    // ... parameters written once ...
) -> Result<ArrayBytes<'a>, CodecError> {
    let encoded = input_handle
        .partial_decode_many(Box::new(byte_ranges), options)
        .await?; // stripped in the derived sync expansion
    // ...
}
// Emits the async source unchanged (cfg-gated), plus a derived
// `fn partial_decode(input_handle: &Arc<dyn BytesPartialDecoderTraits>, ...)`.
```

## Item coverage

`#[unasync::dual]` applies to `fn`, `trait`, `impl` (inherent and trait impls),
`struct`, and `enum`:

- **Traits**: the sync trait and its methods are derived by un-applying the
  rename patterns; default bodies follow the renames automatically;
  `#[allow(async_fn_in_trait)]` is added to the async trait.
- **Impls**: the self type and trait path are rewritten via the `types(...)`
  table (add `types(AsyncClient => Client, AsyncTrait => Trait)`); `async fn`
  methods are renamed and calls between them are followed automatically.
- **Structs/enums**: the type is renamed and field types are substituted.

## Options

| Option | Effect |
|---|---|
| `sync(feature = "sync")` | gate the sync variant behind `#[cfg(feature = "sync")]` |
| `async(feature = "async")` | gate the async variant behind `#[cfg(feature = "async")]` |
| `skip_sync` / `skip_async` | emit only one variant |
| `keep_self` | no renaming: the sync variant keeps the source names |
| `rename = "async_{}"` | fn/method pattern the async names follow; `{}` captures the sync name |
| `rename_types = "Async{}"` | type/trait pattern, same rules |
| `types(A => B, ...)` | async spelling → sync spelling, token-level, generic-aware |
| `calls(f => g, ...)` | async call name → sync call name (`.await` lives in the source) |
| `async_doc = "..."` | extra doc text on the async item |
| `async_attr = #[...]` | runtime attribute for `#[unasync::test]` (default `#[tokio::test]`) |
| `flavor = async_fn \| async_trait` | async trait/impl emission style |
| `wasm_send` | with `async_trait`: native `Send` / wasm `?Send` cfg pairing |

## Divergent bodies

For the minority of code that genuinely differs between the variants:

```rust
let pd = unasync::alt!(
    sync  => codec.partial_decoder(input, shape, opts)?,
    async => codec.async_partial_decoder(input, shape, opts).await?,
);

unasync::sync_only! { chunks.into_par_iter().try_for_each(process)?; }
unasync::async_only! {
    futures::stream::iter(chunks).map(process).buffer_unordered(n).try_collect().await?;
}
```

These are real (exported) macros so unexpanded code still parses for
rust-analyzer/rustfmt; the `dual` rewriter resolves them during expansion, and
each arm is only type-checked in its own expansion.

## Per-variant fields, enum variants, and members

Struct fields, enum variants, and trait/impl members can be kept in only one
expansion with the `#[sync_only]` / `#[async_only]` attributes:

```rust
#[unasync::dual(async(feature = "async"), rename_types = "Async{}")]
pub struct AsyncOptions {
    pub common: u32,
    #[sync_only]
    pub threads: usize,      // only on `Options`
    #[async_only]
    pub concurrency: usize,  // only on `AsyncOptions`
}

#[unasync::dual(async(feature = "async"), rename_types = "Async{}")]
pub enum AsyncMode {
    Shared,
    #[sync_only]
    Blocking { threads: usize },
    #[async_only]
    Concurrent { concurrency: usize },
}
```

## Tests

```rust
#[unasync::test(
    async(feature = "async"),
    rename = "async_{}",
    async_attr = #[tokio::test], // or #[async_std::test], #[wasm_bindgen_test], ...
    types(AsyncObjectStore<InMemory> => MemoryStore),
    calls(async_fixture => fixture),
)]
async fn async_roundtrip() {
    let array = async_fixture().await;
    assert_eq!(array.get().await, expected); // .await stripped inside macros too
}
```

Expands to `#[cfg(feature = "async")] #[tokio::test] async fn async_roundtrip()`
plus a derived `#[test] fn roundtrip()`.

## Shared defaults

Proc macros cannot share state across invocations, so recurring configuration
(rename patterns, type tables) is carried syntactically and stated once:

```rust
#[unasync::scope(async(feature = "async"), rename = "async_{}", types(AsyncStore => Store))]
mod ops {
    #[dual]
    async fn async_get(...) { ... }

    #[unasync::test]
    async fn async_roundtrip() { ... }
}
```

or, for code not wrapped in an inline module (including `mod foo;` files):

```rust
unasync::content! {
    #![defaults(async(feature = "async"), rename = "async_{}", types(AsyncStore => Store))]

    #[dual]
    async fn async_get(...) { ... }
}
```

Item-level options override the defaults; substitution tables concatenate with
item entries taking precedence.

## Variant-specific attributes and docs

- `#[sync_attr(...)]` / `#[async_attr(...)]` on items and methods unwrap into
  the matching variant only.
- The derived sync variant keeps the doc comments verbatim (it is the
  conventional "primary" API); the async variant gets
  ``/// Async variant of [`sync_name`].`` (extendable with `async_doc`), so
  doctests run once, on the sync item.

## wasm and `Send`

The facade exports `MaybeSend`/`MaybeSync` (equal to `Send`/`Sync` except on
`wasm32`, where they are no-op bounds). With `flavor = async_trait, wasm_send`,
async impls/traits get the standard pairing:

```rust
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
```

## Limitations (deliberate)

- **Token-level matching only.** Substitutions match paths/types syntactically
  after normalization — no name resolution, no type inference. Aliased
  spellings must be added to the table.
- **Macro bodies are rewritten best-effort** in the sync expansion: `.await`
  stripping, single-ident type substitutions, and `name(...)` call renames.
  Anything fancier inside `format!`-likes needs `alt!`. The async expansion
  keeps macro bodies verbatim.
- **Whole-item expansion**: `dual` always yields the item's variants — no
  partial magic, no shared-body extraction. The escape hatch is writing the two
  items by hand; the macro is never mandatory.
- `flavor = boxed` (pin-boxed futures for object safety without `async_trait`)
  and `send_bounds` injection are not implemented yet (planned, see PLAN.md
  §3.5).
- `#[unasync::scope]` requires an inline module; for `mod foo;` use
  `unasync::content!` inside the file.

## Deviations from PLAN.md

- §3.1 originally proposed writing sync-spelled names in the source; after
  review the source is the **async spelling throughout** (names and types), so
  the annotated item is valid async Rust and the sync variant is entirely
  derived.
- Rename patterns have **no defaults** (the plan suggested `async_{}` /
  `Async{}` defaults); naming conventions must be stated explicitly.
- Feature gating is symmetric from the start: `sync(feature = ...)` /
  `async(feature = ...)`, plus `skip_sync`/`skip_async` (§3.8's variant model).
- Added `#[sync_only]`/`#[async_only]` gating for struct fields, enum
  variants, and trait/impl members.

## Workspace layout

- [`unasync/`](unasync/) — facade crate: macro re-exports, marker macros,
  `MaybeSend`/`MaybeSync`.
- [`unasync_macros/`](unasync_macros/) — the proc-macro implementation: one
  `syn::visit_mut`-based rewriter run once per variant over a cloned item.
- [`unasync_tests/`](unasync_tests/) — integration tests, with real `async`
  and `sync` features so the emitted cfg gates are exercised under every
  feature combination.

License: MIT OR Apache-2.0. MSRV: 1.91.
