pub struct Unit;

// A verbatim sync signature states its own qualifiers.
#[ambisync::ambisync(sync(fns("async_{}")))]
impl Unit {
    #[sync_const]
    #[sync_signature(fn double(value: u32) -> u32)]
    async fn async_double(value: u32) -> u32 {
        value * 2
    }
}

fn main() {}
