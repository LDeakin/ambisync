//! `#[unasync::dual]` on free functions: the source is written entirely in
//! async form (async names, async types, `.await`); the sync variant is
//! derived. Covers renaming, gating, `sync(types(...))`, `sync(calls(...))`,
//! markers, declarations, and macro-token rewriting.

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
    sync(
        pattern = "async_{}",
        types(AsyncStore => Store),
        calls(async_make_store => make_store),
    ),
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

#[dual(
    sync(pattern = "async_{}", types(AsyncStore => Store)),
    async(feature = "async"),
)]
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
#[dual(sync(pattern = "async_{}"), async(feature = "async"))]
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
    sync(
        pattern = "async_{}",
        types(AsyncStore => Store),
        calls(async_make_store => make_store),
    ),
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
#[dual(sync(feature = "sync", pattern = "async_{}"), async(feature = "async"))]
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

// Suffix-style patterns are supported as well as prefixes.
#[dual(sync(pattern = "{}_async"), async(feature = "async"))]
async fn suffix_styled_async() -> u32 {
    7
}

#[test]
fn suffix_pattern() {
    assert_eq!(suffix_styled(), 7);
}

// A generic-aware whole-type substitution (`&Arc<dyn Trait>`), impossible
// with single-ident renames.
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
    sync(
        pattern = "async_{}",
        types(&Arc<dyn AsyncDecode> => &Arc<dyn Decode>),
        calls(async_make_store => make_store),
    ),
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

// Module-qualified substitutions: the async and sync spellings may live in
// different modules (`mod_x::AsyncThing => mod_y::Thing`), in type position
// and in expression paths.
pub mod mod_x {
    pub struct AsyncThing(pub u32);
    impl AsyncThing {
        pub async fn value(&self) -> u32 {
            self.0
        }
    }
}
pub mod mod_y {
    pub struct Thing(pub u32);
    impl Thing {
        pub fn value(&self) -> u32 {
            self.0 + 1
        }
    }
}

#[dual(
    sync(name = "module_qualified", types(mod_x::AsyncThing => mod_y::Thing)),
    async(feature = "async"),
)]
async fn async_module_qualified(seed: u32) -> u32 {
    let thing: mod_x::AsyncThing = mod_x::AsyncThing(seed);
    thing.value().await
}

#[test]
fn sync_module_qualified_types_substituted() {
    assert_eq!(module_qualified(5), 6);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_module_qualified_types_verbatim() {
    assert_eq!(async_module_qualified(5).await, 5);
}

// sync_attr/async_attr passthrough.
#[dual(sync(pattern = "async_{}"), async(feature = "async"))]
#[sync_attr(allow(clippy::unused_unit))]
#[async_attr(allow(clippy::unused_async))]
async fn async_passthrough() -> u32 {
    4
}

#[test]
fn passthrough_compiles() {
    assert_eq!(passthrough(), 4);
}

// Async blocks and closures become synchronous closure calls/closures. The
// `return` in the block must stay scoped to that block rather than returning
// from the generated sync function.
#[dual(sync(name = "nested_async_forms"), async(feature = "async"))]
async fn async_nested_async_forms(seed: u32) -> u32 {
    let from_block = async move {
        if seed == 0 {
            return 4;
        }
        seed + 1
    }
    .await;
    let increment = async move |value: u32| value + 1;
    from_block + increment(seed).await
}

#[test]
fn sync_nested_async_forms_are_derived() {
    assert_eq!(nested_async_forms(0), 5);
    assert_eq!(nested_async_forms(2), 6);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_nested_forms_remain_async() {
    assert_eq!(async_nested_async_forms(0).await, 5);
    assert_eq!(async_nested_async_forms(2).await, 6);
}

// Exact names allow the source to use a neutral internal name while both
// public output names remain explicit.
#[dual(
    sync(name = "explicit_sync_name"),
    async(feature = "async", name = "explicit_async_name")
)]
async fn internal_neutral_name() -> u32 {
    12
}

#[test]
fn exact_sync_name_is_used() {
    assert_eq!(explicit_sync_name(), 12);
}

// Variant docs and arbitrary item attributes are symmetric.
#[dual(
    sync(
        name = "metadata_sync",
        doc = "Blocking-specific details.",
        attr = #[inline],
        attr = #[allow(clippy::let_and_return)],
    ),
    async(
        feature = "async",
        name = "metadata_async",
        doc = "Async-specific details.",
        attr = #[inline],
    ),
)]
async fn internal_metadata() -> u32 {
    13
}

#[test]
fn sync_metadata_options_compile() {
    assert_eq!(metadata_sync(), 13);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_metadata_options_compile() {
    assert_eq!(metadata_async().await, 13);
}

pub struct DeclaredSyncInput(u32);
pub struct DeclaredAsyncInput(u32);
pub struct RewrittenDeclaredSyncInput;

impl DeclaredSyncInput {
    fn value(&self) -> u32 {
        self.0
    }
}

impl DeclaredAsyncInput {
    async fn value(&self) -> u32 {
        self.0
    }
}

// A declaration replaces the derived signature after the body is rewritten.
// In particular, its `DeclaredSyncInput` type must not be passed through the
// substitution machinery.
#[dual(
    sync(declaration {
        /// Complete blocking declaration.
        #[must_use]
        pub fn declared_sync<F>(input: &DeclaredSyncInput, transform: F) -> u32
        where
            F: FnOnce(u32) -> u32;
        },
        types(DeclaredSyncInput => RewrittenDeclaredSyncInput),
    ),
    async(feature = "async"),
)]
async fn async_declared<F>(input: &DeclaredAsyncInput, transform: F) -> u32
where
    F: FnOnce(u32) -> u32,
{
    transform(input.value().await)
}

#[test]
fn sync_declaration_supplies_the_complete_signature() {
    assert_eq!(declared_sync(&DeclaredSyncInput(21), |value| value + 1), 22);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn source_supplies_the_async_signature() {
    assert_eq!(
        async_declared(&DeclaredAsyncInput(22), |value| value + 1).await,
        23
    );
}

// Both output signatures can be explicit while the shared implementation
// keeps an internal source name.
#[dual(
    sync(declaration {
        pub fn both_declared_sync(input: &DeclaredSyncInput) -> u32;
    }),
    async(
        feature = "async",
        declaration {
            /// Complete async declaration.
            #[must_use]
            pub async fn both_declared_async(input: &DeclaredAsyncInput) -> u32;
        },
    ),
)]
async fn internal_both_declared(input: &DeclaredAsyncInput) -> u32 {
    input.value().await
}

#[test]
fn both_declarations_supply_output_signatures() {
    assert_eq!(both_declared_sync(&DeclaredSyncInput(31)), 31);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn explicit_async_declaration_is_used() {
    assert_eq!(both_declared_async(&DeclaredAsyncInput(32)).await, 32);
}

// Functions also accept an ordinary empty-body item shell in addition to the
// compact body-less declaration form.
#[dual(
    sync(declaration {
        pub fn braced_declared_sync(input: &DeclaredSyncInput) -> u32 {}
    }),
    async(feature = "async"),
)]
async fn async_braced_declared(input: &DeclaredAsyncInput) -> u32 {
    input.value().await
}

#[test]
fn braced_function_declaration_receives_the_shared_body() {
    assert_eq!(braced_declared_sync(&DeclaredSyncInput(41)), 41);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn braced_function_declaration_keeps_async_source() {
    assert_eq!(async_braced_declared(&DeclaredAsyncInput(42)).await, 42);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn exact_async_name_is_used() {
    assert_eq!(explicit_async_name().await, 12);
}
