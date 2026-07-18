#[ambisync::ambisync(sync(pattern = "async_{} => {} => {}"))]
async fn async_read() {}

#[ambisync::ambisync(sync(pattern = "async_{}{} => sync_{}"))]
async fn async_write() {}

fn main() {}
