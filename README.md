# ambisync

Write the async form once; `#[ambisync]` derives the sync variant mechanically.

Libraries that ship both sync and async APIs maintain parallel code paths.
These paths are identical except for `.await` points, item names (for example, `read` / `async_read` and `Store` / `AsyncStore`), test attributes, and feature gates.
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

The source stays real, compilable async code that rust-analyzer and rustfmt understand.
`#[ambisync]` covers `fn`, `trait`, `impl`, `struct`, `enum`, and tests, with:

- explicit naming via exact names or `fn_pattern` / `type_pattern` conventions;
- token-level `types(...)` / `calls(...)` substitution tables that also rewrite doc comments and intra-doc links;
- marker macros (`alt!`, `sync_only!`, `async_only!`) for the minority of code that genuinely diverges;
- explicit outer declarations when one variant needs its own docs, attributes, or signature spelling;
- module-wide shared defaults via `#[ambisync::scope]` / `scoped!`;
- `async_trait` emission with configurable `Send`-ness for wasm targets.

See the [crate documentation](https://docs.rs/ambisync) for the full guide.

## Workspace layout

- [`ambisync/`](ambisync/): facade crate: macro re-exports and marker macros.
- [`ambisync_macros/`](ambisync_macros/): the proc-macro implementation: one `syn::visit_mut`-based rewriter run once per variant over a cloned item.
- [`ambisync_tests/`](ambisync_tests/): integration tests with real `async` and `sync` features, exercising the emitted cfg gates under every feature combination.

## Licence
`ambisync` is licensed under either of
 - the Apache License, Version 2.0 [LICENSE-APACHE](./LICENCE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0> or
 - the MIT license [LICENSE-MIT](./LICENCE-MIT) or <http://opensource.org/licenses/MIT>, at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
