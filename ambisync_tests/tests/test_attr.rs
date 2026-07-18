//! `#[ambisync::test]`: one async-written test expands to a sync `#[test]` and a cfg-gated async test with a pluggable runtime attribute.

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

#[ambisync::test(
    async(feature = "async"),
    sync(
        fns("async_{}", async_make_store => make_store),
        types(AsyncStore => Store),
    ),
)]
async fn async_store_roundtrip() {
    let store = async_make_store().await;
    assert_eq!(store.get().await, store.value);
}

// Explicit runtime attribute.
#[ambisync::test(
    sync(
        fns("async_{}", async_make_store => make_store),
        test_attr = #[test],
        types(AsyncStore => Store),
    ),
    async(feature = "async", test_attr = #[tokio::test]),
)]
async fn async_store_roundtrip_explicit_attr() {
    let store = async_make_store().await;
    let expected = ambisync::alt!(sync => 1, async => 2);
    assert_eq!(store.get().await, expected);
}

#[ambisync::test(
    sync(declaration {
        /// Test generated from an explicit blocking declaration.
        fn declared_test();
    }),
    async(feature = "async"),
)]
async fn async_declared_test() {}

// Result-returning tests work in both variants.
#[ambisync::test(sync(fns("async_{}")), async(feature = "async"))]
async fn async_result_test() -> Result<(), String> {
    let value = ambisync::alt!(sync => 1u32, async => 2u32);
    if value == 0 {
        return Err("zero".to_owned());
    }
    Ok(())
}
