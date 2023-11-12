pub use sqlx;
use tower_sessions_core::session::Error;

#[cfg(feature = "mysql")]
#[cfg_attr(docsrs, doc(cfg(feature = "mysql")))]
pub use self::mysql_store::MySqlStore;
#[cfg(feature = "postgres")]
#[cfg_attr(docsrs, doc(cfg(feature = "postgres")))]
pub use self::postgres_store::PostgresStore;
#[cfg(feature = "sqlite")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlite")))]
pub use self::sqlite_store::SqliteStore;

#[cfg(feature = "sqlite")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlite")))]
mod sqlite_store;

#[cfg(feature = "postgres")]
#[cfg_attr(docsrs, doc(cfg(feature = "postgres")))]
mod postgres_store;

#[cfg(feature = "mysql")]
#[cfg_attr(docsrs, doc(cfg(feature = "mysql")))]
mod mysql_store;

/// An error type for SQLx stores.
#[derive(thiserror::Error, Debug)]
pub enum SqlxStoreError {
    /// A variant to map session errors.
    #[error(transparent)]
    Session(#[from] Error),

    /// A variant to map `sqlx` errors.
    #[error("SQLx error: {0}")]
    Sqlx(#[from] sqlx::Error),

    /// A variant to map `rmp_serde` encode errors.
    #[error("Rust MsgPack encode error: {0}")]
    RmpSerdeEncode(#[from] rmp_serde::encode::Error),

    /// A variant to map `rmp_serde` decode errors.
    #[error("Rust MsgPack decode error: {0}")]
    RmpSerdeDecode(#[from] rmp_serde::decode::Error),
}
