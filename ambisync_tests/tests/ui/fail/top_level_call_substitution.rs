async fn async_value() -> u32 {
    1
}

fn value() -> u32 {
    1
}

#[ambisync::dual(
    sync(name = "read"),
    async(name = "async_read"),
    calls(async_value => value),
)]
async fn source() -> u32 {
    async_value().await
}

fn main() {}
