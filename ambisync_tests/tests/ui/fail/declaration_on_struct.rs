#[ambisync::ambisync(
    sync(declaration {
        fn value() -> u32;
    }),
    async(name = "AsyncValue"),
)]
struct Source;

fn main() {}
