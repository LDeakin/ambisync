//! `#[sync_const]` makes the derived sync variant of a function or method `const`, which an `async fn` cannot be.

#![allow(dead_code)]

#[ambisync::ambisync(sync(fns("async_{}")), async(feature = "async"))]
#[sync_const]
async fn async_answer() -> u32 {
    42
}

pub struct Unit;

#[ambisync::ambisync(sync(fns("async_{}")), async(feature = "async"))]
impl Unit {
    #[sync_const]
    async fn async_double(value: u32) -> u32 {
        value * 2
    }
}

pub struct Paired;

#[ambisync::paired(sync(fns("async_{}")), async(feature = "async"))]
impl Paired {
    #[sync_const]
    async fn async_triple(value: u32) -> u32 {
        value * 3
    }

    #[sync_only]
    #[sync_const]
    async fn async_quadruple(value: u32) -> u32 {
        value * 4
    }
}

const ANSWER: u32 = answer();
const DOUBLED: u32 = Unit::double(21);
const TRIPLED: u32 = Paired::triple(14);
const QUADRUPLED: u32 = Paired::quadruple(10);

#[test]
fn sync_variants_are_const() {
    assert_eq!(ANSWER, 42);
    assert_eq!(DOUBLED, 42);
    assert_eq!(TRIPLED, 42);
    assert_eq!(QUADRUPLED, 40);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_variants_are_unchanged() {
    assert_eq!(async_answer().await, 42);
    assert_eq!(Unit::async_double(21).await, 42);
    assert_eq!(Paired::async_triple(14).await, 42);
}
