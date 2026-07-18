#[ambisync::dual(sync(name = "sync_value"))]
fn async_value() -> u32 {
    1
}

fn main() {}
