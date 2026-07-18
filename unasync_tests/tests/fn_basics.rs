//! `#[unasync::dual]` on free functions: the source is written entirely in
//! async form (async names, async types, `.await`); the sync variant is
//! derived. Covers renaming, gating, `types(...)`, `calls(...)`, markers, and
//! macro-token rewriting.

#![allow(dead_code)]

use unasync::dual;

pub struct Store {
    pub value: u32,
}
pub struct AsyncStore {
    pub value: u32,
}

impl Store {
    fn get(&self) -> u32 {
        self.value
    }
}
impl AsyncStore {
    async fn get(&self) -> u32 {
        self.value
    }
}

fn make_store() -> Store {
    Store { value: 1 }
}
async fn async_make_store() -> AsyncStore {
    AsyncStore { value: 2 }
}

#[dual(
    async(feature = "async"),
    rename = "async_{}",
    types(AsyncStore => Store),
    calls(async_make_store => make_store),
)]
async fn async_read_value() -> u32 {
    let store: AsyncStore = async_make_store().await;
    store.get().await
}

#[test]
fn sync_fn_is_derived() {
    assert_eq!(read_value(), 1);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_fn_is_source_verbatim() {
    assert_eq!(async_read_value().await, 2);
}

// Path substitution in expression position (`AsyncStore::build`-style
// associated fns and struct literals).
impl Store {
    fn build() -> Store {
        Store { value: 10 }
    }
}
impl AsyncStore {
    fn build() -> AsyncStore {
        AsyncStore { value: 20 }
    }
}

#[dual(async(feature = "async"), rename = "async_{}", types(AsyncStore => Store))]
async fn async_build_and_read() -> u32 {
    let a = AsyncStore::build();
    let b = AsyncStore { value: a.value };
    b.get().await
}

#[test]
fn sync_expr_paths_substituted() {
    assert_eq!(build_and_read(), 10);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_expr_paths_untouched() {
    assert_eq!(async_build_and_read().await, 20);
}

// Markers: alt!, sync_only!, async_only!.
#[dual(async(feature = "async"), rename = "async_{}")]
async fn async_which_variant() -> String {
    let mut parts: Vec<&str> = Vec::new();
    unasync::sync_only! {
        parts.push("sync-only");
    }
    unasync::async_only! {
        parts.push("async-only");
    }
    let label = unasync::alt!(sync => "sync", async => "async");
    parts.push(label);
    parts.join(",")
}

#[test]
fn sync_markers_resolve() {
    assert_eq!(which_variant(), "sync-only,sync");
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_markers_resolve() {
    assert_eq!(async_which_variant().await, "async-only,async");
}

// Best-effort rewriting inside macro token streams: `.await` stripped, type
// idents substituted, and call names renamed for the sync variant; the async
// variant keeps macro bodies verbatim.
#[dual(
    async(feature = "async"),
    rename = "async_{}",
    types(AsyncStore => Store),
    calls(async_make_store => make_store),
)]
async fn async_describe() -> String {
    format!("value={}", async_make_store().await.get().await)
}

#[test]
fn sync_macro_tokens_rewritten() {
    assert_eq!(describe(), "value=1");
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_macro_tokens_verbatim() {
    assert_eq!(async_describe().await, "value=2");
}

// Both variants independently feature-gated.
#[dual(sync(feature = "sync"), async(feature = "async"), rename = "async_{}")]
async fn async_gated_both() -> u32 {
    11
}

#[cfg(feature = "sync")]
#[test]
fn sync_gate_applies() {
    assert_eq!(gated_both(), 11);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_gate_applies() {
    assert_eq!(async_gated_both().await, 11);
}

// skip_sync + keep_self: async-only emission under the source name.
#[dual(async(feature = "async"), skip_sync, keep_self)]
async fn only_async_value() -> u32 {
    3
}

#[cfg(feature = "async")]
#[tokio::test]
async fn skip_sync_keeps_source_name() {
    assert_eq!(only_async_value().await, 3);
}

// skip_async: derive the sync variant only (the async source is dropped).
#[dual(skip_async, rename = "{}_async")]
async fn suffix_styled_async() -> u32 {
    7
}

#[test]
fn skip_async_and_suffix_pattern() {
    assert_eq!(suffix_styled(), 7);
}

// The packbits-style case from PLAN.md §3.2: a generic-aware whole-type
// substitution (`&Arc<dyn Trait>`), impossible with single-ident renames.
use std::sync::Arc;

pub trait Decode {
    fn decode(&self) -> u32;
}
pub trait AsyncDecode {
    fn decode(&self) -> u32;
}

struct Dec;
impl Decode for Dec {
    fn decode(&self) -> u32 {
        5
    }
}
impl AsyncDecode for Dec {
    fn decode(&self) -> u32 {
        6
    }
}

#[dual(
    async(feature = "async"),
    rename = "async_{}",
    types(&Arc<dyn AsyncDecode> => &Arc<dyn Decode>),
    calls(async_make_store => make_store),
)]
async fn async_partial_decode(input_handle: &Arc<dyn AsyncDecode>, offset: u32) -> u32 {
    let store = async_make_store().await;
    input_handle.decode() + offset + store.get().await
}

#[test]
fn sync_trait_object_param_substituted() {
    let handle: Arc<dyn Decode> = Arc::new(Dec);
    assert_eq!(partial_decode(&handle, 100), 106);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_trait_object_param_verbatim() {
    let handle: Arc<dyn AsyncDecode> = Arc::new(Dec);
    assert_eq!(async_partial_decode(&handle, 100).await, 108);
}

// sync_attr/async_attr passthrough.
#[dual(async(feature = "async"), rename = "async_{}")]
#[sync_attr(allow(clippy::unused_unit))]
#[async_attr(allow(clippy::unused_async))]
async fn async_passthrough() -> u32 {
    4
}

#[test]
fn passthrough_compiles() {
    assert_eq!(passthrough(), 4);
}
