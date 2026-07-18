//! Complete variant declarations for every item kind supported by `#[ambisync]`.

#![allow(dead_code)]

use ambisync::ambisync;

#[ambisync(
    sync(declaration {
        /// Blocking record declaration.
        #[derive(Debug, PartialEq)]
        pub struct DeclaredRecord<T>
        where
            T: Copy,
        {}
    }),
    async(
        feature = "async",
        declaration {
            /// Async record declaration.
            #[derive(Debug, PartialEq)]
            pub struct AsyncDeclaredRecord<T>
            where
                T: Copy,
            {}
        },
    ),
)]
struct InternalRecord<T> {
    pub value: T,
}

#[ambisync(
    sync(
        fn_pattern = "async_{}",
        declaration {
            impl DeclaredRecord<u32> {}
        },
    ),
    async(
        feature = "async",
        declaration {
            impl AsyncDeclaredRecord<u32> {}
        },
    ),
)]
impl InternalRecord<u32> {
    pub async fn async_double(&self) -> u32 {
        self.value * 2
    }
}

#[test]
fn struct_and_inherent_impl_declarations_are_used() {
    let record = DeclaredRecord { value: 4 };
    assert_eq!(record.double(), 8);
    assert_eq!(record, DeclaredRecord { value: 4 });
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_struct_and_inherent_impl_declarations_are_used() {
    let record = AsyncDeclaredRecord { value: 5 };
    assert_eq!(record.async_double().await, 10);
    assert_eq!(record, AsyncDeclaredRecord { value: 5 });
}

#[ambisync(
    sync(declaration {
        #[derive(Debug, PartialEq)]
        pub struct DeclaredTuple {}
    }),
    async(
        feature = "async",
        declaration {
            #[derive(Debug, PartialEq)]
            pub struct AsyncDeclaredTuple {}
        },
    ),
)]
struct InternalTuple(pub u32);

#[ambisync(
    sync(declaration {
        #[derive(Debug, PartialEq)]
        pub struct DeclaredUnit {}
    }),
    async(
        feature = "async",
        declaration {
            #[derive(Debug, PartialEq)]
            pub struct AsyncDeclaredUnit {}
        },
    ),
)]
struct InternalUnit;

#[test]
fn tuple_and_unit_struct_contents_are_preserved() {
    assert_eq!(DeclaredTuple(6), DeclaredTuple(6));
    assert_eq!(DeclaredUnit, DeclaredUnit);
}

#[cfg(feature = "async")]
#[test]
fn async_tuple_and_unit_struct_contents_are_preserved() {
    assert_eq!(AsyncDeclaredTuple(7), AsyncDeclaredTuple(7));
    assert_eq!(AsyncDeclaredUnit, AsyncDeclaredUnit);
}

#[ambisync(
    sync(declaration {
        /// Blocking choice declaration.
        #[derive(Debug, PartialEq)]
        pub enum DeclaredChoice {}
    }),
    async(feature = "async"),
)]
/// Async choice declaration supplied by the source item itself.
#[derive(Debug, PartialEq)]
pub enum AsyncDeclaredChoice {
    Value(u32),
    Named { value: u32 },
}

#[test]
fn enum_declaration_receives_shared_variants() {
    assert_eq!(DeclaredChoice::Value(3), DeclaredChoice::Value(3));
    assert_eq!(
        DeclaredChoice::Named { value: 7 },
        DeclaredChoice::Named { value: 7 }
    );
}

#[cfg(feature = "async")]
#[test]
fn async_enum_declaration_receives_shared_variants() {
    assert_eq!(AsyncDeclaredChoice::Value(3), AsyncDeclaredChoice::Value(3));
}

// Native async-trait lint suppression is applied after the declaration shell, so replacing the outer attributes cannot discard it.
#[ambisync(
    sync(
        fn_pattern = "async_{}",
        declaration {
            pub trait DeclaredNative {}
        },
    ),
    async(
        feature = "async",
        declaration {
            pub trait AsyncDeclaredNative {}
        },
    ),
)]
trait InternalNative {
    async fn async_ping(&self) {}
}

#[ambisync(
    sync(
        fn_pattern = "async_{}",
        declaration {
            /// Blocking trait declaration.
            pub trait DeclaredRead {}
        },
    ),
    async(
        feature = "async",
        flavor = async_trait,
        declaration {
            /// Async trait declaration.
            pub trait AsyncDeclaredRead {}
        },
    ),
)]
trait InternalRead {
    async fn async_read(&self) -> u32;

    fn shared(&self) -> u32 {
        1
    }
}

pub struct Reader(u32);

#[ambisync(
    sync(
        fn_pattern = "async_{}",
        declaration {
            impl DeclaredRead for Reader {}
        },
    ),
    async(
        feature = "async",
        flavor = async_trait,
        declaration {
            impl AsyncDeclaredRead for Reader {}
        },
    ),
)]
impl InternalRead for Reader {
    async fn async_read(&self) -> u32 {
        self.0
    }
}

#[test]
fn trait_and_trait_impl_declarations_are_used() {
    let reader = Reader(9);
    assert_eq!(DeclaredRead::read(&reader), 9);
    assert_eq!(DeclaredRead::shared(&reader), 1);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_trait_and_trait_impl_declarations_are_used() {
    let reader = Reader(10);
    assert_eq!(AsyncDeclaredRead::async_read(&reader).await, 10);
    assert_eq!(AsyncDeclaredRead::shared(&reader), 1);
    let object: &dyn AsyncDeclaredRead = &reader;
    assert_eq!(object.async_read().await, 10);
}
