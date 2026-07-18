async fn async_value() -> u32 {
    1
}

fn value() -> u32 {
    1
}

#[ambisync::ambisync(
    sync(name = "read"),
    async(name = "async_read"),
    fns(async_value => value),
)]
async fn source() -> u32 {
    async_value().await
}

fn main() {}
