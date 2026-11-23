pub struct Store;

// The sync and async methods would both be inherent methods of `Store`, so `fetch` needs a distinct sync name.
#[ambisync::ambisync(sync(fns("async_{}")))]
impl Store {
    async fn fetch(&self) -> u32 {
        1
    }
}

fn main() {}
