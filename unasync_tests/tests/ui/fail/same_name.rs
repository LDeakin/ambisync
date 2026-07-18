#[unasync::dual(sync(name = "value"), async(name = "value"))]
async fn source_value() -> u32 {
    1
}

fn main() {}
