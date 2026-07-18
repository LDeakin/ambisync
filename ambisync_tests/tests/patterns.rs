//! The `sync(types(...))` / `sync(fns(...))` rename tables: exact `A => B` entries and
//! string-literal pattern entries (`"Async{}"`, `"{}_async"`), naming the annotated items
//! and rewriting matching types/calls, with exact entries taking precedence.

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

fn create_store() -> Store {
    Store { value: 3 }
}
async fn create_async_store() -> AsyncStore {
    AsyncStore { value: 5 }
}

fn helper_value() -> u32 {
    8
}
async fn helper_value_async() -> u32 {
    9
}

ambisync::scoped! {
    #![defaults(
        sync(
            types("Async{}"),
            fns(
                "{}_async",
                "async_{}",
                "create_async_{} => create_{}",
            ),
        ),
        async(feature = "async"),
    )]

    #[ambisync]
    pub async fn combined_async() -> u32 {
        let made: AsyncStore = async_make_store().await;
        let created: AsyncStore = create_async_store().await;
        let literal = AsyncStore { value: 100 };
        made.get().await + created.get().await + helper_value_async().await + literal.value
    }

    // Pattern type entries follow into generic arguments and macro tokens.
    #[ambisync]
    pub async fn stores_async() -> Vec<AsyncStore> {
        vec![AsyncStore { value: 7 }]
    }
}

#[cfg(feature = "sync")]
#[test]
fn pattern_entries_apply_sync() {
    assert_eq!(combined(), 1 + 3 + 8 + 100);
    assert_eq!(stores()[0].value, 7);
    let _: Vec<Store> = stores();
}

#[cfg(feature = "async")]
#[tokio::test]
async fn pattern_entries_apply_async() {
    assert_eq!(combined_async().await, 2 + 5 + 9 + 100);
    let _: Vec<AsyncStore> = stores_async().await;
}
