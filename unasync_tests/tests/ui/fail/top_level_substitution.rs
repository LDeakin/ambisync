struct Store;
struct AsyncStore;

#[unasync::dual(
    sync(name = "value"),
    async(name = "async_value"),
    types(AsyncStore => Store),
)]
async fn source() -> AsyncStore {
    AsyncStore
}

fn main() {}
