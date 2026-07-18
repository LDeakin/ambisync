#[ambisync::ambisync(sync(fns("async_{} => {} => {}")))]
async fn async_read() {}

#[ambisync::ambisync(sync(fns("async_{}{} => sync_{}")))]
async fn async_write() {}

fn main() {}
