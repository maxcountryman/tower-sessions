//! A session store backed by a diesel connection pool
use std::marker::PhantomData;

use async_trait::async_trait;
use diesel::{
    associations::HasTable,
    backend::Backend,
    deserialize::FromStaticSqlRow,
    dsl::{And, Lt},
    expression::{AsExpression, ValidGrouping},
    expression_methods::ExpressionMethods,
    helper_types::{Eq, Filter, Gt, IntoBoxed, SqlTypeOf},
    prelude::{BoolExpressionMethods, Connection, Insertable, Queryable},
    query_builder::{
        AsQuery, DeleteStatement, InsertStatement, IntoUpdateTarget, QueryBuilder, QueryFragment,
        UpdateStatement,
    },
    query_dsl::methods::{BoxedDsl, ExecuteDsl, FilterDsl, LimitDsl, LoadQuery},
    sql_types::{Binary, Bool, SingleValue, SqlType, Text, Timestamp},
    AsChangeset, BoxableExpression, Column, Expression, OptionalExtension, QueryDsl, RunQueryDsl,
    SelectableExpression, Table,
};

use crate::{session_store::ExpiredDeletion, SessionStore};

/// An error type for diesel stores
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum DieselStoreError {
    /// A pool related error
    #[cfg(feature = "diesel-r2d2")]
    #[error("Pool Error: {0}")]
    R2D2Error(#[from] diesel::r2d2::PoolError),
    /// A diesel related error
    #[error("Diesel Error: {0}")]
    DieselError(#[from] diesel::result::Error),
    /// Failed to join a blocking tokio task
    #[cfg(feature = "diesel-r2d2")]
    #[error("Failed to join task: {0}")]
    TokioJoinError(#[from] tokio::task::JoinError),
    /// A variant to map `rmp_serde` encode errors.
    #[error("Failed to serialize session data: {0}")]
    SerializationError(#[from] rmp_serde::encode::Error),
    #[cfg(feature = "diesel-deadpool")]
    #[error("Failed to interact with deadpool: {0}")]
    /// A variant that indicates that we cannot interact with deadpool
    InteractError(String),
    #[cfg(feature = "diesel-deadpool")]
    #[error("Failed to get a connection from deadpool: {0}")]
    /// A variant that indicates that we cannot get a connection from deadpool
    DeadpoolError(#[from] deadpool_diesel::PoolError),
}

/// A Diesel session store
#[derive(Debug, Clone)]
pub struct DieselStore<P, T = self::sessions::table> {
    p: PhantomData<T>,
    pool: P,
}

diesel::table! {
    /// The session table used by default by the diesel-store implemenattion
    sessions {
        /// `id` column, contains a session id
        id -> Text,
        /// `expiry_date` column, contains a required expiry timestamp
        expiry_date -> Timestamp,
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
    C: diesel::Connection,
{
    /// the `expiration_time` column of your table
    type ExpiryDate: Column<SqlType = Timestamp>
        + Default
        + ValidGrouping<(), IsAggregate = diesel::expression::is_aggregate::No>
        + Send
        + 'static;

    /// Insert a new record into the sessions table
    fn insert(
        conn: &mut C,
        session_record: &crate::session::Session,
    ) -> Result<(), DieselStoreError>;

    /// An function to optionally create the session table in the database
    fn migrate(_conn: &mut C) -> Result<(), DieselStoreError> {
        Ok(())
    }
}

/// A helper trait to abstract over different pooling solutions for diesel
#[async_trait::async_trait]
pub trait DieselPool: Clone + Sync + Send + 'static {
    /// The underlying diesel connection type used by this pool
    type Connection: diesel::Connection + 'static;

    /// Interact with a connection from that pool
    async fn interact<R, E, F>(&self, c: F) -> Result<R, DieselStoreError>
    where
        R: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&mut Self::Connection) -> Result<R, E> + Send + 'static,
        DieselStoreError: From<E>;
}

#[cfg(feature = "diesel-r2d2")]
#[async_trait::async_trait]
impl<C> DieselPool for diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<C>>
where
    C: diesel::r2d2::R2D2Connection + 'static,
{
    type Connection = C;

    async fn interact<R, E, F>(&self, c: F) -> Result<R, DieselStoreError>
    where
        F: FnOnce(&mut Self::Connection) -> Result<R, E> + Send + 'static,
        DieselStoreError: From<E>,
        R: Send + 'static,
        E: Send + 'static,
    {
        let pool = self.clone();
        Ok(tokio::task::spawn_blocking(move || {
            let mut conn = pool.get()?;
            let r = c(&mut *conn)?;
            Ok::<_, DieselStoreError>(r)
        })
        .await??)
    }
}

#[cfg(feature = "diesel-deadpool")]
#[async_trait::async_trait]
impl<C> DieselPool for deadpool_diesel::Pool<deadpool_diesel::Manager<C>>
where
    C: Connection + 'static,
    deadpool_diesel::Manager<C>: deadpool::managed::Manager<Type = deadpool_diesel::Connection<C>>,
    <deadpool_diesel::Manager<C> as deadpool::managed::Manager>::Type: Send + Sync,
    <deadpool_diesel::Manager<C> as deadpool::managed::Manager>::Error: std::fmt::Debug,
    DieselStoreError: From<
        deadpool::managed::PoolError<
            <deadpool_diesel::Manager<C> as deadpool::managed::Manager>::Error,
        >,
    >,
{
    type Connection = C;

    async fn interact<R, E, F>(&self, c: F) -> Result<R, DieselStoreError>
    where
        F: FnOnce(&mut Self::Connection) -> Result<R, E> + Send + 'static,
        DieselStoreError: From<E>,
        R: Send + 'static,
        E: Send + 'static,
    {
        let conn = self.get().await?;
        let r = conn
            .as_ref()
            .interact(c)
            .await
            .map_err(|e| DieselStoreError::InteractError(e.to_string()))?;
        r.map_err(Into::into)
    }
}

impl<C> SessionTable<C> for self::sessions::table
where
    <C::Backend as Backend>::QueryBuilder: Default,
    C: diesel::Connection,
    InsertStatement<
        Self,
        <(
            Eq<sessions::id, String>,
            Eq<sessions::expiry_date, time::PrimitiveDateTime>,
            Eq<sessions::data, Vec<u8>>,
        ) as Insertable<Self>>::Values,
    >: ExecuteDsl<C>,
    UpdateStatement<
        Self,
        <Filter<sessions::table, Eq<sessions::id, String>> as IntoUpdateTarget>::WhereClause,
        <(
            Eq<sessions::expiry_date, time::PrimitiveDateTime>,
            Eq<sessions::data, Vec<u8>>,
        ) as AsChangeset>::Changeset,
    >: ExecuteDsl<C>,
{
    type ExpiryDate = self::sessions::expiry_date;

    fn insert(
        conn: &mut C,
        session_record: &crate::session::Session,
    ) -> Result<(), DieselStoreError> {
        let expiry_date = session_record.expiry_date();
        let expiry_date = time::PrimitiveDateTime::new(expiry_date.date(), expiry_date.time());
        let data = rmp_serde::to_vec(session_record)?;
        let session_id = session_record.id().to_string();
        // we want to use an upsert statement here, but that's potentially not supported
        // on all backends, therefore we do a seperate insert + check whether
        // we got a `UniqueViolation` error
        conn.transaction(|conn| {
            let res = diesel::insert_into(sessions::table)
                .values((
                    sessions::id.eq(session_id.clone()),
                    sessions::expiry_date.eq(expiry_date),
                    sessions::data.eq(data.clone()),
                ))
                .execute(conn);
            if matches!(
                res,
                Err(diesel::result::Error::DatabaseError(
                    diesel::result::DatabaseErrorKind::UniqueViolation,
                    _
                ))
            ) {
                diesel::update(sessions::table.find(session_id))
                    .set((
                        sessions::expiry_date.eq(expiry_date),
                        sessions::data.eq(data),
                    ))
                    .execute(conn)?;
            } else {
                res?;
            }
            Ok(())
        })
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
        qb.push_identifier(sessions::expiry_date::NAME)?;
        qb.push_sql(" TIMESTAMP NOT NULL, ");
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

impl<P> DieselStore<P>
where
    P: DieselPool,
{
    /// Create a new diesel store with a provided connection pool.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # #[cfg(feature = "diesel-r2d2")]
    /// # fn main() {
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
    /// # }
    ///
    /// # #[cfg(not(feature = "diesel-r2d2"))]
    /// # fn main() {}
    /// ```
    pub fn new(pool: P) -> Self {
        Self {
            pool,
            p: PhantomData,
        }
    }
}

impl<P, T> DieselStore<P, T>
where
    P: DieselPool,
    T: SessionTable<P::Connection>,
{
    /// Create a new diesel store with a provided connection pool.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # #[cfg(feature = "diesel-r2d2")]
    /// # fn main() {
    /// use diesel::{
    ///     prelude::*,
    ///     r2d2::{ConnectionManager, Pool},
    /// };
    /// use tower_sessions::diesel_store::DieselStore;
    ///
    /// let pool = Pool::builder()
    ///     .build(ConnectionManager::<SqliteConnection>::new(":memory:"))
    ///     .unwrap();
    /// let session_store =
    ///     DieselStore::with_table(tower_sessions::diesel_store::sessions::table, pool);
    /// # }
    ///
    /// # #[cfg(not(feature = "diesel-r2d2"))]
    /// # fn main() {}
    /// ```
    pub fn with_table(_table: T, pool: P) -> Self {
        Self {
            pool,
            p: PhantomData,
        }
    }

    /// Migrate the session schema.
    pub async fn migrate(&self) -> Result<(), DieselStoreError> {
        self.pool
            .interact(|conn| {
                T::migrate(conn)?;
                Ok::<_, DieselStoreError>(())
            })
            .await?;
        Ok(())
    }
}

impl<DB> Queryable<(Text, Timestamp, Binary), DB> for crate::session::Session
where
    DB: Backend,
    (String, time::PrimitiveDateTime, Vec<u8>): FromStaticSqlRow<(Text, Timestamp, Binary), DB>,
{
    type Row = (String, time::PrimitiveDateTime, Vec<u8>);

    fn build((_id, _expiration_time, data): Self::Row) -> diesel::deserialize::Result<Self> {
        let session = rmp_serde::from_slice(&data)?;
        Ok(session)
    }
}

#[async_trait::async_trait]
impl<P, T> SessionStore for DieselStore<P, T>
where
    P: DieselPool,
    T: SessionTable<P::Connection> + 'static,
    String: AsExpression<SqlTypeOf<T::PrimaryKey>>,
    T::PrimaryKey: Default,
    <T::PrimaryKey as Expression>::SqlType: SqlType + SingleValue,
    DeleteStatement<T, Eq<T::PrimaryKey, String>>: ExecuteDsl<P::Connection>,
    T: FilterDsl<Eq<T::PrimaryKey, String>>
        + BoxedDsl<'static, <P::Connection as Connection>::Backend>,
    IntoBoxed<'static, T, <P::Connection as Connection>::Backend>:
        LimitDsl<Output = IntoBoxed<'static, T, <P::Connection as Connection>::Backend>>,
    IntoBoxed<'static, T, <P::Connection as Connection>::Backend>: FilterDsl<
        And<Eq<T::PrimaryKey, String>, Gt<T::ExpiryDate, diesel::dsl::now>>,
        Output = IntoBoxed<'static, T, <P::Connection as Connection>::Backend>,
    >,
    Filter<T, Eq<T::PrimaryKey, String>>: IntoUpdateTarget,
    DeleteStatement<
        <Filter<T, Eq<T::PrimaryKey, String>> as HasTable>::Table,
        <Filter<T, Eq<T::PrimaryKey, String>> as IntoUpdateTarget>::WhereClause,
    >: ExecuteDsl<P::Connection>,
    Eq<T::PrimaryKey, String>: BoolExpressionMethods<SqlType = Bool>,
    for<'a> IntoBoxed<'static, T, <P::Connection as Connection>::Backend>:
        LoadQuery<'a, P::Connection, crate::Session>,
{
    type Error = DieselStoreError;

    async fn save(&self, session_record: &crate::Session) -> Result<(), Self::Error> {
        let record = session_record.clone();
        self.pool
            .interact(move |conn| T::insert(conn, &record))
            .await?;
        Ok(())
    }

    async fn load(
        &self,
        session_id: &crate::session::Id,
    ) -> Result<Option<crate::Session>, Self::Error> {
        let session_id = session_id.to_string();
        let res = self
            .pool
            .interact(move |conn| {
                let q = T::table()
                    .into_boxed()
                    .limit(1)
                    .filter(
                        T::PrimaryKey::default()
                            .eq(session_id.to_string())
                            .and(T::ExpiryDate::default().gt(diesel::dsl::now)),
                    )
                    .get_result(conn)
                    .optional()?;
                Ok::<_, DieselStoreError>(q)
            })
            .await?;
        Ok(res)
    }

    async fn delete(&self, session_id: &crate::session::Id) -> Result<(), Self::Error> {
        let session_id = session_id.to_string();
        self.pool
            .interact(move |conn| {
                diesel::delete(
                    T::table().filter(T::PrimaryKey::default().eq(session_id.to_string())),
                )
                .execute(conn)?;
                Ok::<_, DieselStoreError>(())
            })
            .await?;
        Ok(())
    }
}

#[async_trait]
impl<P, T> ExpiredDeletion for DieselStore<P, T>
where
    P: DieselPool,
    Self: SessionStore<Error = DieselStoreError>,
    T: SessionTable<P::Connection>,
    T: FilterDsl<
        Box<dyn BoxableExpression<T, <P::Connection as Connection>::Backend, SqlType = Bool>>,
    >,
    Filter<
        T,
        Box<dyn BoxableExpression<T, <P::Connection as Connection>::Backend, SqlType = Bool>>,
    >: IntoUpdateTarget,
    DeleteStatement<
        <Filter<
            T,
            Box<dyn BoxableExpression<T, <P::Connection as Connection>::Backend, SqlType = Bool>>,
        > as HasTable>::Table,
        <Filter<
            T,
            Box<dyn BoxableExpression<T, <P::Connection as Connection>::Backend, SqlType = Bool>>,
        > as IntoUpdateTarget>::WhereClause,
    >: ExecuteDsl<P::Connection>,
    Lt<T::ExpiryDate, diesel::dsl::now>: QueryFragment<<P::Connection as Connection>::Backend>
        + SelectableExpression<T>
        + Expression<SqlType = Bool>,
{
    async fn delete_expired(&self) -> Result<(), Self::Error> {
        self.pool
            .interact(|conn| {
                let filter = Box::new(T::ExpiryDate::default().lt(diesel::dsl::now)) as Box<_>;
                diesel::delete(T::table().filter(filter)).execute(conn)?;
                Ok::<_, DieselStoreError>(())
            })
            .await?;
        Ok(())
    }
}
