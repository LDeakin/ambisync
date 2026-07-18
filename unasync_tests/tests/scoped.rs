//! Shared defaults: `#[unasync::scope]` on an inline module and
//! `unasync::content! { #![defaults(...)] ... }`. The rename patterns and
//! substitution tables are stated once per module.

#![allow(dead_code)]

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

#[unasync::scope(
    async(feature = "async"),
    rename = "async_{}",
    types(AsyncStore => Store),
)]
mod scoped {
    use super::*;

    #[dual(calls(async_make_store => make_store))]
    pub async fn async_scoped_value() -> u32 {
        let store: AsyncStore = async_make_store().await;
        store.get().await
    }

    #[unasync::test]
    async fn async_scoped_test() {
        let store: AsyncStore = unasync::alt!(
            sync => make_store(),
            async => async_make_store().await,
        );
        let expected = unasync::alt!(sync => 1, async => 2);
        assert_eq!(store.get().await, expected);
    }
}

#[test]
fn scope_applies_defaults_sync() {
    assert_eq!(scoped::scoped_value(), 1);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn scope_applies_defaults_async() {
    assert_eq!(scoped::async_scoped_value().await, 2);
}

unasync::content! {
    #![defaults(async(feature = "async"), rename = "async_{}", types(AsyncStore => Store))]

    #[dual(calls(async_make_store => make_store))]
    pub async fn async_content_value() -> u32 {
        let store: AsyncStore = async_make_store().await;
        store.get().await
    }

    // Unannotated items pass through untouched.
    pub fn passthrough_item() -> u32 {
        99
    }
}

#[test]
fn content_applies_defaults_sync() {
    assert_eq!(content_value(), 1);
    assert_eq!(passthrough_item(), 99);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn content_applies_defaults_async() {
    assert_eq!(async_content_value().await, 2);
}
