#[ambisync::ambisync(
    sync(
        name = "value",
        declaration {
            fn other_value() -> u32;
        },
    ),
    async(name = "async_value"),
)]
async fn source() -> u32 {
    1
}

fn main() {}
