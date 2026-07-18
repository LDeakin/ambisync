//! Signature-only impl members (`pub async fn f(...);`): renamed, rewritten, and kept body-less
//! in both expansions, so `#[ambisync]` composes with the `inherent` crate's convention of
//! exposing trait defaults as inherent methods via body-less entries.

#![allow(dead_code)]

use ambisync::ambisync;
use inherent::inherent;

pub struct Store(u32);
impl Store {
    fn get(&self) -> u32 {
        self.0
    }
}

pub struct AsyncStore(u32);
impl AsyncStore {
    async fn get(&self) -> u32 {
        self.0
    }
}

pub struct Cached<S> {
    store: S,
}

#[ambisync(
    sync(name = "ReadOps", fns("async_{}"), types(AsyncStore => Store)),
    async(feature = "async"),
)]
pub trait AsyncReadOps {
    async fn async_retrieve(&self) -> u32;

    async fn async_retrieve_double(&self) -> u32 {
        self.async_retrieve().await * 2
    }
}

// The defaulted trait method is exposed inherently through a signature-only member, mirroring
// how the `inherent` crate is used with defaulted sync/async operation traits.
#[ambisync(
    sync(fns("async_{}"), types(AsyncStore => Store, AsyncReadOps => ReadOps)),
    async(feature = "async"),
)]
#[inherent]
impl AsyncReadOps for Cached<AsyncStore> {
    pub async fn async_retrieve(&self) -> u32 {
        self.store.get().await
    }

    pub async fn async_retrieve_double(&self) -> u32;
}

#[cfg(feature = "sync")]
#[test]
fn signature_only_members_expose_trait_defaults_inherently() {
    let cached = Cached { store: Store(3) };
    assert_eq!(cached.retrieve(), 3);
    assert_eq!(cached.retrieve_double(), 6);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_signature_only_members_expose_trait_defaults_inherently() {
    let cached = Cached {
        store: AsyncStore(4),
    };
    assert_eq!(cached.async_retrieve().await, 4);
    assert_eq!(cached.async_retrieve_double().await, 8);
}
