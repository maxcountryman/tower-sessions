#[cfg(feature = "mysql-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "mysql-store")))]
pub use self::mysql_store::MySqlStore;
#[cfg(feature = "postgres-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "postgres-store")))]
pub use self::postgres_store::PostgresStore;
#[cfg(feature = "sqlite-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlite-store")))]
pub use self::sqlite_store::SqliteStore;
use crate::session::Error;

#[cfg(feature = "sqlite-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlite-store")))]
mod sqlite_store;

#[cfg(feature = "postgres-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "postgres-store")))]
mod postgres_store;

#[cfg(feature = "mysql-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "mysql-store")))]
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

    /// A variant to map `serde_json` errors.
    #[error("JSON serialization/deserialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    /// A variant to map `rmp_serde` encode errors.
    #[error("Rust MsgPack encode error: {0}")]
    RmpSerdeEncode(#[from] rmp_serde::encode::Error),

    /// A variant to map `rmp_serde` decode errors.
    #[error("Rust MsgPack decode error: {0}")]
    RmpSerdeDecode(#[from] rmp_serde::decode::Error),
}
