//! `#[unasync::dual]` on structs, enums and impl blocks: field-type twinning,
//! self-type substitution, constructor rewriting, method renames, automatic
//! call-following, and `#[sync_only]`/`#[async_only]` member gating.

#![allow(dead_code)]

use unasync::dual;

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

#[dual(
    sync(type_pattern = "Async{}", types(AsyncConn => Conn)),
    async(feature = "async"),
)]
pub struct AsyncClient {
    conn: AsyncConn,
}

#[dual(
    async(feature = "async"),
    sync(
        pattern = "async_{}",
        types(AsyncClient => Client, AsyncConn => Conn),
    ),
)]
#[allow(clippy::new_without_default)]
impl AsyncClient {
    pub fn new() -> AsyncClient {
        AsyncClient { conn: AsyncConn }
    }

    pub async fn async_fetch(&self) -> u32 {
        self.conn.get().await
    }

    /// An async method calling another dual method of the same impl: the
    /// rename is followed automatically in the derived sync variant.
    pub async fn async_fetch_twice(&self) -> u32 {
        self.async_fetch().await + self.async_fetch().await
    }
}

#[test]
fn sync_struct_and_impl() {
    let client = Client::new();
    assert_eq!(client.fetch(), 10);
    assert_eq!(client.fetch_twice(), 20);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_struct_and_impl() {
    let client = AsyncClient::new();
    assert_eq!(client.async_fetch().await, 20);
    assert_eq!(client.async_fetch_twice().await, 40);
}

// Per-variant struct fields.
#[dual(sync(type_pattern = "Async{}"), async(feature = "async"))]
pub struct AsyncOptions {
    pub common: u32,
    #[sync_only]
    pub threads: u32,
    #[async_only]
    pub concurrency: u32,
}

#[test]
fn sync_only_fields_kept() {
    let options = Options {
        common: 1,
        threads: 4,
    };
    assert_eq!(options.common + options.threads, 5);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_only_fields_kept() {
    let options = AsyncOptions {
        common: 1,
        concurrency: 8,
    };
    assert_eq!(options.common + options.concurrency, 9);
}

// Per-variant enum variants (and per-variant fields inside variants).
#[dual(sync(type_pattern = "Async{}"), async(feature = "async"))]
pub enum AsyncMode {
    Shared,
    #[sync_only]
    Blocking {
        threads: u32,
    },
    #[async_only]
    Concurrent {
        concurrency: u32,
    },
}

#[test]
fn sync_only_enum_variants_kept() {
    let mode = Mode::Blocking { threads: 4 };
    match mode {
        Mode::Shared => panic!("wrong variant"),
        Mode::Blocking { threads } => assert_eq!(threads, 4),
    }
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_only_enum_variants_kept() {
    let mode = AsyncMode::Concurrent { concurrency: 8 };
    match mode {
        AsyncMode::Shared => panic!("wrong variant"),
        AsyncMode::Concurrent { concurrency } => assert_eq!(concurrency, 8),
    }
}

// Per-variant impl members: an async-only helper method.
pub struct Widget;
pub struct AsyncWidget;

#[dual(
    async(feature = "async"),
    sync(pattern = "async_{}", types(AsyncWidget => Widget)),
)]
impl AsyncWidget {
    pub fn size(&self) -> u32 {
        1
    }

    #[async_only]
    pub fn concurrency_hint(&self) -> u32 {
        16
    }
}

#[test]
fn impl_member_gating_sync() {
    assert_eq!(Widget.size(), 1);
    // `Widget` has no `concurrency_hint`: it is async-only.
}

#[cfg(feature = "async")]
#[tokio::test]
async fn impl_member_gating_async() {
    assert_eq!(AsyncWidget.size(), 1);
    assert_eq!(AsyncWidget.concurrency_hint(), 16);
}

// Exact output names rewrite self-references in both generated variants.
#[dual(sync(name = "Node"), async(feature = "async", name = "AsyncNode"))]
pub struct InternalNode {
    next: Option<Box<InternalNode>>,
}

#[test]
fn exact_struct_name_rewrites_sync_self_references() {
    let node = Node {
        next: Some(Box::new(Node { next: None })),
    };
    assert!(node.next.is_some());
}

#[cfg(feature = "async")]
#[tokio::test]
async fn exact_struct_name_rewrites_async_self_references() {
    let node = AsyncNode {
        next: Some(Box::new(AsyncNode { next: None })),
    };
    assert!(node.next.is_some());
}

pub struct SameNamed;
pub struct AsyncSameNamed;

#[dual(
    sync(pattern = "{}", types(AsyncSameNamed => SameNamed)),
    async(feature = "async"),
)]
impl AsyncSameNamed {
    pub async fn value(&self) -> u32 {
        13
    }
}

#[test]
fn identity_pattern_supports_same_named_methods() {
    assert_eq!(SameNamed.value(), 13);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn identity_pattern_keeps_async_method_name() {
    assert_eq!(AsyncSameNamed.value().await, 13);
}

// Arrow patterns on impls and types: `async_{} => sync_{}` method names and
// an `Async{} => Sync{}` type pattern, with call-following between methods.
#[dual(sync(type_pattern = "Async{} => Sync{}"), async(feature = "async"))]
pub struct AsyncPort {
    pub value: u32,
}

#[dual(
    sync(
        pattern = "async_{} => sync_{}",
        types(AsyncPort => SyncPort),
    ),
    async(feature = "async"),
)]
impl AsyncPort {
    pub async fn async_read(&self) -> u32 {
        self.value
    }

    pub async fn async_read_twice(&self) -> u32 {
        self.async_read().await + self.async_read().await
    }
}

#[test]
fn arrow_patterns_rename_type_and_methods() {
    let port = SyncPort { value: 7 };
    assert_eq!(port.sync_read(), 7);
    assert_eq!(port.sync_read_twice(), 14);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn arrow_patterns_keep_the_async_spellings() {
    let port = AsyncPort { value: 7 };
    assert_eq!(port.async_read().await, 7);
    assert_eq!(port.async_read_twice().await, 14);
}
