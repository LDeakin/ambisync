#[ambisync::ambisync(sync(name = "value"), async(send = false))]
async fn async_value() -> u32 {
    1
}

fn main() {}
