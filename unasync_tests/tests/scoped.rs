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
    sync(pattern = "async_{}", types(AsyncStore => Store)),
)]
mod scoped {
    #![allow(clippy::let_unit_value)]

    use super::*;

    #[dual(sync(calls(async_make_store => make_store)))]
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
    #![defaults(
        sync(pattern = "async_{}", types(AsyncStore => Store)),
        async(feature = "async"),
    )]

    #[dual(sync(calls(async_make_store => make_store)))]
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

// Item-level `async(flavor = async_fn)` must override an `async_trait` scope
// default. The async body deliberately holds an `Rc` across an await, which
// would fail the default async-trait `Send` requirement.
struct Local(std::rc::Rc<()>);

#[unasync::scope(
    sync(pattern = "async_{}", type_pattern = "Async{}"),
    async(feature = "async", flavor = async_trait),
)]
mod flavor_override {
    use std::future::ready;

    use super::*;

    #[dual(async(flavor = async_fn))]
    pub trait AsyncLocalTask {
        async fn async_run(&self);
    }

    #[dual(
        async(flavor = async_fn),
        sync(types(AsyncLocalTask => LocalTask)),
    )]
    impl AsyncLocalTask for Local {
        async fn async_run(&self) {
            let local = self.0.clone();
            unasync::async_only! {
                ready(()).await;
            }
            drop(local);
        }
    }
}

#[test]
fn scoped_async_fn_flavor_overrides_async_trait_default() {
    use flavor_override::LocalTask;

    Local(std::rc::Rc::new(())).run();
}

#[cfg(feature = "async")]
#[tokio::test]
async fn scoped_async_fn_override_accepts_non_send_future() {
    use flavor_override::AsyncLocalTask;

    Local(std::rc::Rc::new(())).async_run().await;
}

// Arrow patterns are ordinary defaults, so they work in `scope` (and
// `content!`) unchanged.
#[unasync::scope(async(feature = "async"), sync(pattern = "async_{} => sync_{}"))]
mod arrow_scoped {
    #[dual]
    pub async fn async_answer() -> u32 {
        42
    }
}

#[test]
fn scope_arrow_pattern_applies() {
    assert_eq!(arrow_scoped::sync_answer(), 42);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn scope_arrow_pattern_keeps_async_name() {
    assert_eq!(arrow_scoped::async_answer().await, 42);
}
