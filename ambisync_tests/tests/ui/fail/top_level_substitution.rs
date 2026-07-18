struct Store;
struct AsyncStore;

#[ambisync::dual(
    sync(name = "value"),
    async(name = "async_value"),
    types(AsyncStore => Store),
)]
async fn source() -> AsyncStore {
    AsyncStore
}

fn main() {}
