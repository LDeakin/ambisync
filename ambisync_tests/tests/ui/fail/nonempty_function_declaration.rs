#[ambisync::ambisync(
    sync(declaration {
        fn value() -> u32 {
            1
        }
    }),
    async(name = "async_value"),
)]
async fn source() -> u32 {
    1
}

fn main() {}
