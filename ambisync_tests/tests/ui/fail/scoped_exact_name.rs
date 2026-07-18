#[ambisync::scope(sync(name = "value"))]
mod values {
    #[dual]
    async fn async_value() -> u32 {
        1
    }
}

fn main() {}
