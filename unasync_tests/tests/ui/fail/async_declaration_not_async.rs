#[unasync::dual(
    sync(name = "value"),
    async(declaration {
        fn async_value() -> u32;
    }),
)]
async fn source() -> u32 {
    1
}

fn main() {}
