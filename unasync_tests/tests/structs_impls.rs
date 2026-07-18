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

#[dual(async(feature = "async"), rename_types = "Async{}", types(AsyncConn => Conn))]
pub struct AsyncClient {
    conn: AsyncConn,
}

#[dual(
    async(feature = "async"),
    rename = "async_{}",
    types(AsyncClient => Client, AsyncConn => Conn),
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
#[dual(async(feature = "async"), rename_types = "Async{}")]
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
#[dual(async(feature = "async"), rename_types = "Async{}")]
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
    rename = "async_{}",
    types(AsyncWidget => Widget),
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
