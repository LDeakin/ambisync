// `async_trait` above the ambisync attribute expands first, leaving no `async fn` to derive sync methods from.
#[async_trait::async_trait]
#[ambisync::ambisync(sync(name = "Read", fns("async_{}")), async(flavor = async_trait))]
pub trait AsyncRead {
    async fn async_read(&self) -> u32;
}

pub trait Codec {
    fn decode(&self) -> u32;
}

pub struct Passthrough;

#[async_trait::async_trait]
#[ambisync::paired(sync(fns("async_{}")), async(flavor = async_trait))]
impl Codec for Passthrough {
    fn decode(&self) -> u32 {
        1
    }

    async fn async_decode(&self) -> u32 {
        1
    }
}

fn main() {}
