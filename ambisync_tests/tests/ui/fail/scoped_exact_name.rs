#[ambisync::scope(sync(name = "value"))]
mod values {
    #[ambisync]
    async fn async_value() -> u32 {
        1
    }
}

fn main() {}
