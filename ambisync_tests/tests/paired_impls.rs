//! `#[ambisync::paired]` on impl blocks: a single impl of a paired trait, where each async method
//! gains a mechanically derived synchronous sibling and ordinary members are emitted once.
//!
//! This mirrors the shape of a trait that carries both APIs at once, so implementors cannot use
//! `#[ambisync]` (which would emit two impls of the same trait for the same type).

#![allow(dead_code)]

pub struct Handle;
pub struct AsyncHandle;

impl Handle {
    fn load(&self) -> u32 {
        7
    }
}
impl AsyncHandle {
    async fn load(&self) -> u32 {
        7
    }
}

/// A trait holding both the sync and async API, as `#[ambisync::paired]` produces.
#[ambisync::paired(
    sync(fns("async_{}"), types("Async{}")),
    async(feature = "async", flavor = async_trait),
)]
pub trait Codec: Sync {
    /// Ordinary members are untouched and exist once.
    fn name(&self) -> &'static str;

    async fn async_decoder(&self, handle: &AsyncHandle) -> u32 {
        handle.load().await
    }

    async fn async_encoder(&self, handle: &AsyncHandle, extra: u32) -> u32 {
        self.async_decoder(handle).await + extra
    }
}

pub struct Passthrough;

// The impl header is written once and serves both APIs: the `types(...)` table rewrites only the
// derived sync method signatures and bodies, never `Codec` or `Passthrough`.
#[ambisync::paired(
    sync(fns("async_{}"), types("Async{}")),
    async(feature = "async", flavor = async_trait),
)]
impl Codec for Passthrough {
    fn name(&self) -> &'static str {
        "passthrough"
    }

    async fn async_decoder(&self, handle: &AsyncHandle) -> u32 {
        handle.load().await * 2
    }

    async fn async_encoder(&self, handle: &AsyncHandle, extra: u32) -> u32 {
        self.async_decoder(handle).await + extra
    }
}

#[test]
fn paired_impl_provides_the_sync_api() {
    let codec: &dyn Codec = &Passthrough;
    assert_eq!(codec.name(), "passthrough");
    assert_eq!(codec.decoder(&Handle), 14);
    assert_eq!(codec.encoder(&Handle, 3), 17);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn paired_impl_provides_the_async_api() {
    let codec: &dyn Codec = &Passthrough;
    assert_eq!(codec.name(), "passthrough");
    assert_eq!(codec.async_decoder(&AsyncHandle).await, 14);
    assert_eq!(codec.async_encoder(&AsyncHandle, 3).await, 17);
}

// `#[sync_name]` and `#[sync_signature]` work on paired impl members, as they do on paired traits.
pub struct Renamed;

#[ambisync::paired(
    sync(fns("async_{}"), types("Async{}")),
    async(feature = "async", flavor = async_trait),
)]
pub trait RenameApi: Sync {
    #[sync_name(fetch_blocking)]
    async fn async_fetch(&self, handle: &AsyncHandle) -> u32;

    #[sync_signature(fn borrow<'a>(&'a self, input: &str) -> &'a str)]
    async fn async_borrow<'a>(&'a self, input: &'a str) -> &'a str;
}

#[ambisync::paired(
    sync(fns("async_{}"), types("Async{}")),
    async(feature = "async", flavor = async_trait),
)]
impl RenameApi for Renamed {
    #[sync_name(fetch_blocking)]
    async fn async_fetch(&self, handle: &AsyncHandle) -> u32 {
        handle.load().await
    }

    #[sync_signature(fn borrow<'a>(&'a self, input: &str) -> &'a str)]
    async fn async_borrow<'a>(&'a self, input: &'a str) -> &'a str {
        let _ = input;
        "borrowed"
    }
}

#[test]
fn paired_impl_honours_member_level_overrides() {
    let api: &dyn RenameApi = &Renamed;
    assert_eq!(api.fetch_blocking(&Handle), 7);
    let output;
    {
        let short_lived = String::from("short");
        output = api.borrow(&short_lived);
    }
    assert_eq!(output, "borrowed");
}

#[cfg(feature = "async")]
#[tokio::test]
async fn paired_impl_leaves_async_member_names_unchanged() {
    let api: &dyn RenameApi = &Renamed;
    assert_eq!(api.async_fetch(&AsyncHandle).await, 7);
    assert_eq!(api.async_borrow("input").await, "borrowed");
}

// An inherent impl needs no trait and no `flavor`, and its non-async members pass through.
pub struct Inherent;

#[ambisync::paired(sync(fns("async_{}"), types("Async{}")), async(feature = "async"))]
impl Inherent {
    pub fn label(&self) -> &'static str {
        "inherent"
    }

    pub async fn async_value(&self, handle: &AsyncHandle) -> u32 {
        handle.load().await
    }
}

#[test]
fn paired_inherent_impl_emits_both_apis() {
    assert_eq!(Inherent.label(), "inherent");
    assert_eq!(Inherent.value(&Handle), 7);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn paired_inherent_impl_keeps_the_async_method() {
    assert_eq!(Inherent.async_value(&AsyncHandle).await, 7);
}

// `ambisync::alt!` selects per-variant expressions inside paired impl bodies.
pub struct Divergent;

#[ambisync::paired(
    sync(fns("async_{}"), types("Async{}")),
    async(feature = "async", flavor = async_trait),
)]
impl Codec for Divergent {
    fn name(&self) -> &'static str {
        "divergent"
    }

    async fn async_decoder(&self, handle: &AsyncHandle) -> u32 {
        let base = handle.load().await;
        base + ambisync::alt!(sync => 1, async => 2)
    }

    async fn async_encoder(&self, handle: &AsyncHandle, extra: u32) -> u32 {
        self.async_decoder(handle).await + extra
    }
}

#[test]
fn paired_impl_resolves_alt_in_the_sync_variant() {
    assert_eq!(Divergent.decoder(&Handle), 8);
    assert_eq!(Divergent.encoder(&Handle, 10), 18);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn paired_impl_resolves_alt_in_the_async_variant() {
    assert_eq!(Divergent.async_decoder(&AsyncHandle).await, 9);
    assert_eq!(Divergent.async_encoder(&AsyncHandle, 10).await, 19);
}
