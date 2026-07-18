#[unasync::dual(sync(pattern = "async_{} => {} => {}"))]
async fn async_read() {}

#[unasync::dual(sync(pattern = "async_{}{} => sync_{}"))]
async fn async_write() {}

fn main() {}
