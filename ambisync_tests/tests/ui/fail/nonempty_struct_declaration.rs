#[ambisync::ambisync(
    sync(declaration {
        struct Value {
            value: u32,
        }
    }),
    async(name = "AsyncValue"),
)]
struct Source {
    value: u32,
}

fn main() {}
