//! Trait and impl methods that no `fns(...)` entry matches keep their names when the sync trait or impl is distinct from
//! the async one, as their methods cannot clash.

#![allow(dead_code)]

pub struct File(u32);
pub struct AsyncFile(u32);

#[ambisync::ambisync(sync(types("Async{}")), async(feature = "async"))]
pub trait AsyncRead {
    async fn read(&self) -> u32;

    async fn read_twice(&self) -> u32 {
        self.read().await * 2
    }
}

#[ambisync::ambisync(sync(types("Async{}")), async(feature = "async"))]
impl AsyncRead for AsyncFile {
    async fn read(&self) -> u32 {
        self.0
    }
}

#[ambisync::ambisync(sync(types("Async{}")), async(feature = "async"))]
impl AsyncFile {
    async fn size(&self) -> u32 {
        self.read().await
    }
}

#[test]
fn sync_members_keep_their_names() {
    let file = File(3);
    assert_eq!(file.read(), 3);
    assert_eq!(file.read_twice(), 6);
    assert_eq!(file.size(), 3);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_members_keep_their_names() {
    let file = AsyncFile(3);
    assert_eq!(file.read().await, 3);
    assert_eq!(file.read_twice().await, 6);
    assert_eq!(file.size().await, 3);
}
