#[ambisync::paired(sync(fns("async_{}")), async(feature = "async"))]
struct Source;

fn main() {}
