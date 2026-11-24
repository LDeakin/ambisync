# ambisync

Write the async form once; `#[ambisync]` derives the sync variant mechanically.

Libraries that ship both sync and async APIs maintain parallel code paths.
These paths are often identical except for `.await` points, item names (for example, `read` / `async_read` and `Store` / `AsyncStore`), test attributes, and feature gates.
`ambisync` can collapse a sync and async `fn`, `trait`, `impl`, `struct`, `enum`, or test into a single async-written item.
For example:

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

`#[ambisync]` covers `fn`, `trait`, `impl`, `struct`, `enum`, and tests, with:

- explicit naming and token-level substitution via one pair of rename tables — `fns(...)` for the fn namespace, `types(...)` for the type namespace — holding exact `A => B` entries and `"Async{}"` / `"{}_async"`-style pattern entries, applied to item names, type references, calls, doc comments, and intra-doc links;
- `#[ambisync::paired]` for mixed traits and impls where sync and async methods coexist in a single item;
- marker macros (`alt!`, `sync_only!`, `async_only!`) and helper attributes (`#[sync_only]` / `#[async_only]`, `#[sync_bounds]` / `#[async_bounds]`, `#[sync_name]`, `#[sync_signature]`, `#[sync_const]`, `#[sync_attr]` / `#[async_attr]`) for the minority of code, generics, or signatures that genuinely diverge;
- explicit outer declarations when one variant needs its own docs, attributes, or signature spelling;
- module-wide shared defaults via `#[ambisync::scope]` / `scoped!`;
- `async_trait` emission with configurable `Send`-ness for wasm targets.

See the [crate documentation](https://docs.rs/ambisync) for the full guide.

## Licence
`ambisync` is licensed under either of
 - the Apache License, Version 2.0 [LICENSE-APACHE](./LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0> or
 - the MIT license [LICENSE-MIT](./LICENSE-MIT) or <http://opensource.org/licenses/MIT>, at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
