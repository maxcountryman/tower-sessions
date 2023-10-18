//! A session store backed by a diesel connection pool
use std::marker::PhantomData;

use async_trait::async_trait;
use diesel::{
    associations::HasTable,
    backend::Backend,
    deserialize::FromStaticSqlRow,
    dsl::{And, IsNull, Lt, Or},
    expression::{AsExpression, ValidGrouping},
    expression_methods::ExpressionMethods,
    helper_types::{Eq, Filter, Gt, IntoBoxed, SqlTypeOf},
    prelude::{BoolExpressionMethods, Insertable, Queryable},
    query_builder::{
        AsQuery, DeleteStatement, InsertStatement, IntoUpdateTarget, QueryBuilder, QueryFragment,
    },
    query_dsl::methods::{BoxedDsl, ExecuteDsl, FilterDsl, LimitDsl, LoadQuery},
    r2d2::{ConnectionManager, ManageConnection, Pool, R2D2Connection},
    sql_types::{Binary, Bool, Nullable, SingleValue, SqlType, Text, Timestamp},
    BoxableExpression, Column, Expression, OptionalExtension, QueryDsl, RunQueryDsl,
    SelectableExpression, Table,
};

use crate::{session_store::ExpiredDeletion, SessionStore};

/// An error type for diesel stores
#[derive(thiserror::Error, Debug)]
pub enum DieselStoreError {
    /// A pool related error
    #[error("Pool Error: {0}")]
    R2D2Error(#[from] diesel::r2d2::PoolError),
    /// A diesel related error
    #[error("Diesel Error: {0}")]
    DieselError(#[from] diesel::result::Error),
    /// Failed to join a blocking tokio task
    #[error("Failed to join task: {0}")]
    TokioJoinERror(#[from] tokio::task::JoinError),
    /// A variant to map `rmp_serde` encode errors.
    #[error("Failed to serialize session data: {0}")]
    SerializationError(#[from] rmp_serde::encode::Error),
}

/// A Diesel session store
#[derive(Debug)]
pub struct DieselStore<C: R2D2Connection + 'static, T = self::sessions::table> {
    p: PhantomData<T>,
    pool: diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<C>>,
}

// custom impl as we don't want to have `Clone bounds on the types
impl<C: R2D2Connection + 'static, T> Clone for DieselStore<C, T> {
    fn clone(&self) -> Self {
        Self {
            p: self.p,
            pool: self.pool.clone(),
        }
    }
}

diesel::table! {
    /// The session table used by default by the diesel-store implemenattion
    sessions {
        /// `id` column, contains a session id
        id -> Text,
        /// `expiration_time` column, contains an optional expiration timestamp
        expiration_time -> Nullable<Timestamp>,
        /// `data` column, contains serialized session data
        data -> Binary,
    }
}

/// An extension trait for customizing the session table used by the
/// [`DieselStore`]
///
/// Implement this for your `table` type if you want to use a custom table
/// definition
pub trait SessionTable<C>: Copy + Send + Sync + AsQuery + HasTable<Table = Self> + Table
where
    C: R2D2Connection,
{
    /// the `expiration_time` column of your table
    type ExpirationTime: Column<SqlType = Nullable<Timestamp>>
        + Default
        + ValidGrouping<(), IsAggregate = diesel::expression::is_aggregate::No>
        + Send
        + 'static;

    /// Insert a new record into the sessions table
    fn insert(
        conn: &mut C,
        session_record: &crate::session::SessionRecord,
    ) -> Result<(), DieselStoreError>;

    /// An function to optionally create the session table in the database
    fn migrate(_conn: &mut C) -> Result<(), DieselStoreError> {
        Ok(())
    }
}

impl<C> SessionTable<C> for self::sessions::table
where
    <C::Backend as Backend>::QueryBuilder: Default,
    C: diesel::r2d2::R2D2Connection,
    InsertStatement<
        Self,
        <(
            Eq<sessions::id, String>,
            Eq<sessions::expiration_time, Option<time::PrimitiveDateTime>>,
            Eq<sessions::data, Vec<u8>>,
        ) as Insertable<Self>>::Values,
    >: ExecuteDsl<C>,
{
    type ExpirationTime = self::sessions::expiration_time;

    fn insert(
        conn: &mut C,
        session_record: &crate::session::SessionRecord,
    ) -> Result<(), DieselStoreError> {
        diesel::insert_into(sessions::table)
            .values((
                sessions::id.eq(session_record.id().to_string()),
                sessions::expiration_time.eq(session_record
                    .expiration_time()
                    .map(|t| time::PrimitiveDateTime::new(t.date(), t.time()))),
                sessions::data.eq(rmp_serde::to_vec(&session_record.data())?),
            ))
            .execute(conn)?;
        Ok(())
    }

    fn migrate(conn: &mut C) -> Result<(), DieselStoreError> {
        let mut qb = <C::Backend as Backend>::QueryBuilder::default();
        let connection_type = std::any::type_name::<C::Backend>();
        qb.push_sql("CREATE TABLE IF NOT EXISTS ");
        qb.push_identifier("sessions")?;
        qb.push_sql("( ");
        qb.push_identifier(sessions::id::NAME)?;
        // we need these hacks to not depend on all diesel backends on the same time
        if connection_type.ends_with("Mysql") {
            qb.push_sql(" CHAR(36) PRIMARY KEY NOT NULL, ");
        } else {
            qb.push_sql(" TEXT PRIMARY KEY NOT NULL, ");
        }
        qb.push_identifier(sessions::expiration_time::NAME)?;
        qb.push_sql(" TIMESTAMP NULL, ");
        qb.push_identifier(sessions::data::NAME)?;
        // we need these hacks to not depend on all diesel backends on the same time
        if connection_type.ends_with("Pg") {
            qb.push_sql(" BYTEA NOT NULL);");
        } else {
            qb.push_sql("BLOB NOT NULL);");
        }
        let r = conn.batch_execute(&qb.finish());
        if !matches!(
            r,
            Err(diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _,
            ))
        ) {
            // ignore unique violations because of postgres issues:
            // https://www.postgresql.org/message-id/CA+TgmoZAdYVtwBfp1FL2sMZbiHCWT4UPrzRLNnX1Nb30Ku3-gg@mail.gmail.com
            r?;
        }
        Ok(())
    }
}

impl<C> DieselStore<C>
where
    C: R2D2Connection,
{
    /// Create a new diesel store with a provided connection pool.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use diesel::{
    ///     prelude::*,
    ///     r2d2::{ConnectionManager, Pool},
    /// };
    /// use tower_sessions::diesel_store::DieselStore;
    ///
    /// let pool = Pool::builder()
    ///     .build(ConnectionManager::<SqliteConnection>::new(":memory:"))
    ///     .unwrap();
    /// let session_store = DieselStore::new(pool);
    /// ```
    pub fn new(pool: diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<C>>) -> Self {
        Self {
            pool,
            p: PhantomData,
        }
    }
}

impl<C, T> DieselStore<C, T>
where
    C: R2D2Connection,
    T: SessionTable<C>,
    T: FilterDsl<Box<dyn BoxableExpression<T, C::Backend, SqlType = Nullable<Bool>>>>,
    Filter<T, Box<dyn BoxableExpression<T, C::Backend, SqlType = Nullable<Bool>>>>: IntoUpdateTarget,
    DeleteStatement<
        <Filter<T, Box<dyn BoxableExpression<T, C::Backend, SqlType = Nullable<Bool>>>> as HasTable>::Table,
        <Filter<T, Box<dyn BoxableExpression<T, C::Backend, SqlType = Nullable<Bool>>>> as IntoUpdateTarget>::WhereClause,
    >: ExecuteDsl<C>,
   Lt<T::ExpirationTime, diesel::dsl::now>: QueryFragment<C::Backend> + SelectableExpression<T> + Expression<SqlType = Nullable<Bool>>,
{
    /// Create a new diesel store with a provided connection pool.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions::diesel_store::{DieselStore};
    /// use diesel::prelude::*;
    /// use diesel::r2d2::{Pool, ConnectionManager};
    ///
    /// let pool = Pool::builder().build(ConnectionManager::<SqliteConnection>::new(":memory:")).unwrap();
    /// let session_store = DieselStore::with_table(tower_sessions::diesel_store::sessions::table, pool);
    /// ```
    pub fn with_table(
        _table: T,
        pool: diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<C>>,
    ) -> Self {
        Self {
            pool,
            p: PhantomData,
        }
    }

    /// Migrate the session schema.
    pub async fn migrate(&self) -> Result<(), DieselStoreError> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get()?;
            T::migrate(&mut conn)?;
            Ok::<_, DieselStoreError>(())
        })
        .await??;
        Ok(())
    }

    async fn delete_expired(&self) -> Result<(), DieselStoreError> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get()?;
            let filter: Box<dyn BoxableExpression<T, C::Backend, SqlType = Nullable<Bool>>> = Box::new(T::ExpirationTime::default().lt(diesel::dsl::now)) as Box<_>;
            diesel::delete(T::table().filter(filter))
                .execute(&mut conn)?;
            Ok::<_, DieselStoreError>(())
        })
        .await??;
        Ok(())
    }

    /// This function will keep running indefinitely, deleting expired rows and
    /// then waiting for the specified period before deleting again.
    ///
    /// Generally this will be used as a task, for example via
    /// `tokio::task::spawn`.
    ///
    /// # Errors
    ///
    /// This function returns a `Result` that contains an error of type
    /// `sqlx::Error` if the deletion operation fails.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions::diesel_store::DieselStore;
    /// use diesel::r2d2::{ConnectionManager, Pool};
    /// use diesel::prelude::*;
    ///
    /// # #[cfg(all(feature = "diesel/sqlite", feature = "continuously-delete-expired"))]
    /// # {
    /// let pool = Pool::builder().build(ConnectionManager::<SqliteConnection>::new(":memory:")).unwrap();
    /// let session_store = DieselStore::new(pool);
    ///
    /// # tokio_test::block_on(async {
    /// tokio::task::spawn(
    ///     session_store
    ///         .clone()
    ///         .continuously_delete_expired(tokio::time::Duration::from_secs(60)),
    /// );
    /// # })
    /// # }
    /// ```
    #[cfg(feature = "continuously-delete-expired")]
    #[cfg_attr(docsrs, doc(cfg(feature = "continuously-delete-expired")))]
    pub async fn continuously_delete_expired(
        self,
        period: tokio::time::Duration,
    ) -> Result<(), DieselStoreError> {
        let mut interval = tokio::time::interval(period);
        loop {
            self.delete_expired().await?;
            interval.tick().await;
        }
    }
}

impl<DB> Queryable<(Text, Nullable<Timestamp>, Binary), DB> for crate::session::Session
where
    DB: Backend,
    (String, Option<time::PrimitiveDateTime>, Vec<u8>):
        FromStaticSqlRow<(Text, Nullable<Timestamp>, Binary), DB>,
{
    type Row = (String, Option<time::PrimitiveDateTime>, Vec<u8>);

    fn build((id, expiration_time, data): Self::Row) -> diesel::deserialize::Result<Self> {
        let expiration_time = expiration_time.map(|t| t.assume_utc());
        let session_id = crate::session::SessionId::try_from(id)?;
        let session_record = crate::session::SessionRecord::new(
            session_id,
            expiration_time,
            rmp_serde::from_slice(&data)?,
        );
        Ok(session_record.into())
    }
}

#[async_trait::async_trait]
impl<C, T> SessionStore for DieselStore<C, T>
where
    T: SessionTable<C> + 'static,
    String: AsExpression<SqlTypeOf<T::PrimaryKey>>,
    T::PrimaryKey: Default,
    <T::PrimaryKey as Expression>::SqlType: SqlType + SingleValue,
    DeleteStatement<T, Eq<T::PrimaryKey, String>>: ExecuteDsl<C>,
    T: FilterDsl<Eq<T::PrimaryKey, String>> + BoxedDsl<'static, C::Backend>,
    IntoBoxed<'static, T, C::Backend>: LimitDsl<Output = IntoBoxed<'static, T, C::Backend>>,
    IntoBoxed<'static, T, C::Backend>: FilterDsl<
        And<
            Eq<T::PrimaryKey, String>,
            Or<IsNull<T::ExpirationTime>, Gt<T::ExpirationTime, diesel::dsl::now>, Nullable<Bool>>,
            Nullable<Bool>,
        >,
        Output = IntoBoxed<'static, T, C::Backend>,
    >,
    Filter<T, Eq<T::PrimaryKey, String>>: IntoUpdateTarget,
    DeleteStatement<
        <Filter<T, Eq<T::PrimaryKey, String>> as HasTable>::Table,
        <Filter<T, Eq<T::PrimaryKey, String>> as IntoUpdateTarget>::WhereClause,
    >: ExecuteDsl<C>,
    Eq<T::PrimaryKey, String>: BoolExpressionMethods<SqlType = Bool>,
    for<'a> IntoBoxed<'static, T, C::Backend>: LoadQuery<'a, C, crate::Session>,
    Pool<ConnectionManager<C>>: Clone,
    ConnectionManager<C>: ManageConnection<Connection = C>,
    C: R2D2Connection,
{
    type Error = DieselStoreError;

    async fn save(&self, session_record: &crate::SessionRecord) -> Result<(), Self::Error> {
        let pool = self.pool.clone();
        let record = session_record.clone();
        tokio::task::spawn_blocking(move || {
            let conn: &mut diesel::r2d2::PooledConnection<diesel::r2d2::ConnectionManager<C>> =
                &mut pool.get()?;
            T::insert(conn, &record)
        })
        .await??;
        Ok(())
    }

    async fn load(
        &self,
        session_id: &crate::session::SessionId,
    ) -> Result<Option<crate::Session>, Self::Error> {
        let session_id = session_id.to_string();
        let pool = self.pool.clone();
        let res = tokio::task::spawn_blocking(move || {
            let conn: &mut diesel::r2d2::PooledConnection<diesel::r2d2::ConnectionManager<C>> =
                &mut pool.get()?;

            let q = T::table()
                .into_boxed()
                .limit(1)
                .filter(
                    T::PrimaryKey::default().eq(session_id.to_string()).and(
                        T::ExpirationTime::default()
                            .is_null()
                            .or(T::ExpirationTime::default().gt(diesel::dsl::now)),
                    ),
                )
                .get_result(conn)
                .optional()?;
            Ok::<_, DieselStoreError>(q)
        })
        .await??;

        Ok(res)
    }

    async fn delete(&self, session_id: &crate::session::SessionId) -> Result<(), Self::Error> {
        let session_id = session_id.to_string();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let conn: &mut diesel::r2d2::PooledConnection<diesel::r2d2::ConnectionManager<C>> =
                &mut pool.get()?;
            diesel::delete(T::table().filter(T::PrimaryKey::default().eq(session_id.to_string())))
                .execute(conn)?;
            Ok::<_, DieselStoreError>(())
        })
        .await??;
        Ok(())
    }
}
