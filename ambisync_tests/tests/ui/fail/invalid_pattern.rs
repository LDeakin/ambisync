#[ambisync::dual(sync(pattern = "async_{} => {} => {}"))]
async fn async_read() {}

#[ambisync::dual(sync(pattern = "async_{}{} => sync_{}"))]
async fn async_write() {}

fn main() {}
