//! `#[sync_only]` / `#[async_only]` on generic parameters: a parameter that only one variant needs, such as the
//! future type returned by an async callback, is dropped from the other variant together with its where-clause bounds.

#![allow(dead_code)]

#[cfg(feature = "async")]
use std::future::Future;

/// The callback's future type only exists in the async variant; the sync variant's callback returns the value itself.
#[ambisync::ambisync(sync(fns("async_{}"), types(Fut => u32)), async(feature = "async"))]
async fn async_call_twice<F, #[async_only] Fut>(f: F) -> u32
where
    F: Fn() -> Fut,
    Fut: Future<Output = u32>,
{
    f().await + f().await
}

/// A lifetime only the sync variant uses.
#[ambisync::ambisync(sync(fns("async_{}")), async(feature = "async"))]
async fn async_first<#[sync_only] 'a>(values: &[u32]) -> u32
where
    'a: 'a,
{
    values[0]
}

pub struct Calls;

#[ambisync::ambisync(sync(fns("async_{}"), types(Fut => u32)), async(feature = "async"))]
impl Calls {
    async fn async_call<F: Fn() -> Fut, #[async_only] Fut: Future<Output = u32>>(
        &self,
        f: F,
    ) -> u32 {
        f().await
    }
}

#[test]
fn sync_variant_drops_async_only_params() {
    assert_eq!(call_twice(|| 2), 4);
    assert_eq!(first(&[3]), 3);
    assert_eq!(Calls.call(|| 5), 5);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_variant_keeps_async_only_params() {
    assert_eq!(async_call_twice(|| async { 2 }).await, 4);
    assert_eq!(async_first(&[3]).await, 3);
    assert_eq!(Calls.async_call(|| async { 5 }).await, 5);
}
