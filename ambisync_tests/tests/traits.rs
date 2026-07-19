//! `#[ambisync]` on traits and trait impls: trait renaming, method renaming, default bodies, trait-impl target renaming via the types table, and the `async_trait` flavor.
//! The source is the async trait; the sync trait is derived.

#![allow(dead_code)]

use ambisync::ambisync;

pub struct Conn;
pub struct AsyncConn;

impl Conn {
    fn get(&self) -> u32 {
        10
    }
}
impl AsyncConn {
    async fn get(&self) -> u32 {
        20
    }
}

#[ambisync(
    async(feature = "async"),
    sync(
        fns("async_{}"),
        types("Async{}"),
        types(AsyncConn => Conn),
    ),
)]
pub trait AsyncFetch {
    fn conn(&self) -> &AsyncConn;

    /// Default bodies follow the method renames of the trait itself.
    async fn async_fetch_value(&self) -> u32 {
        self.conn().get().await
    }

    async fn async_fetch_plus(&self, extra: u32) -> u32 {
        self.async_fetch_value().await + extra
    }
}

pub struct Client {
    conn: Conn,
}
pub struct AsyncClient {
    conn: AsyncConn,
}

#[ambisync(
    async(feature = "async"),
    sync(
        fns("async_{}"),
        types(AsyncFetch => Fetch, AsyncClient => Client, AsyncConn => Conn),
    ),
)]
impl AsyncFetch for AsyncClient {
    fn conn(&self) -> &AsyncConn {
        &self.conn
    }
}

#[test]
fn sync_trait_with_default_bodies() {
    let client = Client { conn: Conn };
    assert_eq!(client.fetch_value(), 10);
    assert_eq!(client.fetch_plus(5), 15);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_trait_with_default_bodies() {
    let client = AsyncClient { conn: AsyncConn };
    assert_eq!(client.async_fetch_value().await, 20);
    assert_eq!(client.async_fetch_plus(5).await, 25);
}

// async(flavor = async_trait): dyn-safe traits via #[async_trait::async_trait].
#[ambisync(
    sync(fns("async_{}"), types("Async{}")),
    async(feature = "async", flavor = async_trait),
)]
pub trait AsyncValue {
    async fn async_value(&self) -> u32;
}

pub struct Fixed;

#[ambisync(
    sync(fns("async_{}"), types(AsyncValue => Value)),
    async(feature = "async", flavor = async_trait),
)]
impl AsyncValue for Fixed {
    async fn async_value(&self) -> u32 {
        42
    }
}

#[test]
fn sync_flavored_trait_object() {
    let obj: Box<dyn Value> = Box::new(Fixed);
    assert_eq!(obj.value(), 42);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_trait_flavor_object() {
    let obj: Box<dyn AsyncValue> = Box::new(Fixed);
    assert_eq!(obj.async_value().await, 42);
}

// async(send = false): always-`?Send` async_trait emission.
// The async body deliberately holds an `Rc` across an await, which plain (`Send`) `#[async_trait]` would reject.
#[ambisync(
    sync(fns("async_{}"), types("Async{}")),
    async(feature = "async", flavor = async_trait, send = false),
)]
pub trait AsyncLocalValue {
    async fn async_local_value(&self) -> u32;
}

#[ambisync(
    sync(fns("async_{}"), types(AsyncLocalValue => LocalValue)),
    async(feature = "async", flavor = async_trait, send = false),
)]
impl AsyncLocalValue for Fixed {
    async fn async_local_value(&self) -> u32 {
        let local = std::rc::Rc::new(41);
        ambisync::async_only! {
            std::future::ready(()).await;
        }
        *local + 1
    }
}

#[test]
fn send_false_sync_trait_object() {
    let obj: Box<dyn LocalValue> = Box::new(Fixed);
    assert_eq!(obj.local_value(), 42);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn send_false_accepts_non_send_future() {
    let obj: Box<dyn AsyncLocalValue> = Box::new(Fixed);
    assert_eq!(obj.async_local_value().await, 42);
}

// async(send = cfg(PRED)): `Send` where the predicate holds, `?Send` elsewhere.
// `send = cfg(not(target_arch = "wasm32"))` is the standard native-`Send` / wasm-`?Send` pairing.
#[ambisync(
    sync(fns("async_{}"), types("Async{}")),
    async(
        feature = "async",
        flavor = async_trait,
        send = cfg(not(target_arch = "wasm32")),
    ),
)]
pub trait AsyncCfgValue {
    async fn async_cfg_value(&self) -> u32;
}

#[ambisync(
    sync(fns("async_{}"), types(AsyncCfgValue => CfgValue)),
    async(
        feature = "async",
        flavor = async_trait,
        send = cfg(not(target_arch = "wasm32")),
    ),
)]
impl AsyncCfgValue for Fixed {
    async fn async_cfg_value(&self) -> u32 {
        7
    }
}

#[test]
fn send_cfg_sync_trait_object() {
    let obj: Box<dyn CfgValue> = Box::new(Fixed);
    assert_eq!(obj.cfg_value(), 7);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn send_cfg_is_send_on_native() {
    fn assert_send<F: std::future::Future + Send>(fut: F) -> F {
        fut
    }
    let obj: Box<dyn AsyncCfgValue + Send + Sync> = Box::new(Fixed);
    // The test targets are native, so the predicate holds and the boxed future must be `Send`.
    assert_eq!(assert_send(obj.async_cfg_value()).await, 7);
}

// `#[sync_name(...)]` gives one trait/impl method an exact sync name. It is the only way to
// rename a non-async method that carries an async name (`async_reader` returns the async reader
// rather than a future), and it overrides the `fns(...)` pattern for an async method.
// Calls in other bodies follow the overrides.
pub struct Reader;
pub struct AsyncReader;

impl Reader {
    fn get(&self) -> u32 {
        5
    }
}
impl AsyncReader {
    async fn get(&self) -> u32 {
        5
    }
}

pub struct Repo;

#[ambisync(sync(fns("async_{}"), types("Async{}")), async(feature = "async"))]
pub trait AsyncSource {
    /// Synchronous, but it belongs to the async API: it returns an [`AsyncReader`], not a future.
    #[sync_name(reader)]
    fn async_reader(&self) -> AsyncReader;

    /// `sync_name` beats the `fns("async_{}")` pattern, which would have emitted `read`.
    #[sync_name(read_blocking)]
    async fn async_read(&self) -> u32 {
        self.async_reader().get().await
    }

    async fn async_read_twice(&self) -> u32 {
        self.async_read().await * 2
    }
}

#[ambisync(sync(fns("async_{}"), types("Async{}")), async(feature = "async"))]
impl AsyncSource for Repo {
    #[sync_name(reader)]
    fn async_reader(&self) -> AsyncReader {
        AsyncReader
    }
}

#[test]
fn sync_name_gives_exact_sync_method_names() {
    assert_eq!(Repo.reader().get(), 5);
    assert_eq!(Repo.read_blocking(), 5);
    assert_eq!(Repo.read_twice(), 10);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn sync_name_leaves_async_method_names_unchanged() {
    assert_eq!(Repo.async_reader().get().await, 5);
    assert_eq!(Repo.async_read().await, 5);
    assert_eq!(Repo.async_read_twice().await, 10);
}

// `#[ambisync::paired]` keeps one mixed trait and derives a synchronous sibling for each async
// method. Ordinary synchronous members remain single members of that trait.
#[ambisync::paired(
    sync(fns("async_{}"), types("Async{}")),
    async(feature = "async", flavor = async_trait),
)]
pub trait MixedApi: Sync {
    fn label(&self) -> &'static str {
        "mixed"
    }

    async fn async_read(&self, reader: &AsyncReader) -> u32 {
        reader.get().await
    }
}

impl MixedApi for Fixed {}

#[test]
fn paired_trait_has_sync_and_common_methods() {
    let api: &dyn MixedApi = &Fixed;
    assert_eq!(api.label(), "mixed");
    assert_eq!(api.read(&Reader), 5);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn paired_trait_has_async_and_common_methods() {
    let api: &dyn MixedApi = &Fixed;
    assert_eq!(api.label(), "mixed");
    assert_eq!(api.async_read(&AsyncReader).await, 5);
}

// `#[sync_signature(fn ...)]` keeps the method body shared while allowing lifetime relationships
// in the sync API to differ from the async API. Its method name also participates in call mapping.
pub struct BorrowSource(&'static str);

#[ambisync(sync(fns("async_{}"), types("Async{}")), async(feature = "async"))]
pub trait AsyncBorrowApi {
    #[sync_signature(fn borrow<'a>(&'a self, input: &str) -> &'a str)]
    async fn async_borrow<'a>(&'a self, input: &'a str) -> &'a str;

    async fn async_len(&self, input: &str) -> usize {
        self.async_borrow(input).await.len()
    }
}

#[ambisync(
    sync(fns("async_{}"), types(AsyncBorrowApi => BorrowApi)),
    async(feature = "async"),
)]
impl AsyncBorrowApi for BorrowSource {
    #[sync_signature(fn borrow<'a>(&'a self, input: &str) -> &'a str)]
    async fn async_borrow<'a>(&'a self, input: &'a str) -> &'a str {
        let _ = input;
        self.0
    }
}

#[test]
fn sync_signature_can_relax_an_input_lifetime() {
    let source = BorrowSource("borrowed from self");
    let output;
    {
        let short_lived = String::from("short");
        output = source.borrow(&short_lived);
        assert_eq!(source.len(&short_lived), output.len());
    }
    assert_eq!(output, "borrowed from self");
}

#[cfg(feature = "async")]
#[tokio::test]
async fn sync_signature_preserves_the_async_signature() {
    let source = BorrowSource("borrowed from self");
    let input = String::from("short");
    assert_eq!(source.async_borrow(&input).await, "borrowed from self");
    assert_eq!(source.async_len(&input).await, "borrowed from self".len());
}

// Generic trait paths in trait-impl targets retain/substitute their generic arguments rather than being limited to bare identifiers.
pub struct GenericHolder<T>(T);
pub struct AsyncGenericHolder<T>(T);

#[ambisync(sync(name = "GenericTake", fns("async_{}")), async(feature = "async"))]
pub trait AsyncGenericTake<T> {
    async fn async_take(&self) -> T;
}

#[ambisync(
    sync(
        fns("async_{}"),
        types(
            AsyncGenericTake<T> => GenericTake<T>,
            AsyncGenericHolder<T> => GenericHolder<T>,
        ),
    ),
    async(feature = "async"),
)]
impl<T: Copy> AsyncGenericTake<T> for AsyncGenericHolder<T> {
    async fn async_take(&self) -> T {
        self.0
    }
}

#[test]
fn sync_generic_trait_impl_target_is_substituted() {
    assert_eq!(GenericHolder(7_u32).take(), 7);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_generic_trait_impl_is_unchanged() {
    assert_eq!(AsyncGenericHolder(8_u32).async_take().await, 8);
}

// Per-variant trait members: default-bodied helpers kept in one variant only.
#[ambisync(sync(fns("async_{}"), types("Async{}")), async(feature = "async"))]
pub trait AsyncTuning {
    async fn async_size(&self) -> u32;

    #[sync_only]
    fn threads(&self) -> u32 {
        4
    }

    #[async_only]
    fn concurrency(&self) -> u32 {
        8
    }
}

pub struct Tuned;

impl Tuning for Tuned {
    fn size(&self) -> u32 {
        1
    }
}

#[cfg(feature = "async")]
impl AsyncTuning for Tuned {
    async fn async_size(&self) -> u32 {
        2
    }
}

#[test]
fn sync_only_trait_members_kept() {
    assert_eq!(Tuned.size() + Tuned.threads(), 5);
    // `Tuning` has no `concurrency`: it is async-only.
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_only_trait_members_kept() {
    assert_eq!(Tuned.async_size().await + Tuned.concurrency(), 10);
    // `AsyncTuning` has no `threads`: it is sync-only.
}
