//! `#[unasync::dual]` on traits and trait impls: trait renaming, method
//! renaming, default bodies, trait-impl target renaming via the types table,
//! and the `async_trait` flavor. The source is the async trait; the sync
//! trait is derived.

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
    async(feature = "async"),
    rename = "async_{}",
    rename_types = "Async{}",
    types(AsyncConn => Conn),
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

#[dual(
    async(feature = "async"),
    rename = "async_{}",
    types(AsyncFetch => Fetch, AsyncClient => Client, AsyncConn => Conn),
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

// flavor = async_trait: dyn-safe async traits via #[async_trait::async_trait].
#[dual(
    async(feature = "async"),
    rename = "async_{}",
    rename_types = "Async{}",
    flavor = async_trait,
)]
pub trait AsyncValue {
    async fn async_value(&self) -> u32;
}

pub struct Fixed;

#[dual(
    async(feature = "async"),
    rename = "async_{}",
    flavor = async_trait,
    types(AsyncValue => Value),
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
