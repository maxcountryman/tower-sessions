//! A session which allows HTTP applications to associate data with visitors.
//!
//! The structs provided here have a strict API, but they are designed to be nearly impossible to
//! misuse. Luckily, they only have a handful of methods, and all of them document how they work.
use std::{
    fmt::Debug,
    mem::ManuallyDrop,
    sync::{Arc, Mutex},
};
// TODO: Remove send + sync bounds on `R` once return type notation is stable.

use tower_sesh_core::{expires::Expires, id::Id, Expiry, SessionStore};

#[derive(Debug, Clone, Copy)]
pub(crate) enum SessionUpdate {
    Delete,
    Set(Id, Expiry),
}

pub(crate) type Updater = Arc<Mutex<Option<SessionUpdate>>>;

/// A session that is lazily loaded.
///
/// This is struct provided throught a Request's Extensions by the [`SessionManager`] middleware.
/// If you happen to use `axum`, you can use this struct as an extractor since it implements
/// [`FromRequestParts`].
///
/// When this struct refers to the "underlying store error", it is referring to the fact that the
/// store used returned a "hard" error. For example, it could be a connection error, a protocol error,
/// a timeout, etc. A counterexample would be the [`SessionState`] not being found in the store, which is
/// not considered an error by the [`SessionStore`] trait.
///
/// # Examples
/// - If you are using `axum`, and you have enabled the `extractor` feature, you can use this
///     struct as an extractor:
/// ```rust
/// use tower_sesh::{Session, MemoryStore};
///
/// async fn handler(session: Session<MemoryStore<()>>) -> String {
///     unimplemented!()
/// }
/// ```
/// The extractor will error if the handler was called without a `SessionManager` middleware.
///
/// - Otherwise, you can extract it from a request's extensions:
/// ```
/// use tower_sesh::{Session, MemoryStore};
/// use axum_core::{extract::Request, body::Body};
///
/// async fn handler(mut req: Request<Body>) -> String {
///    let Some(session) = req.extensions_mut().remove::<Session<MemoryStore<()>>>() else {
///         return "No session found".to_string();
///    };
///    unimplemented!()
///    // ...
/// }
/// ```
/// Again, the session will not be found if the handler was called without a `SessionManager`
/// middleware.
#[derive(Debug, Clone)]
pub struct Session<Store> {
    /// This will be `None` if the handler has not received a session cookie or if the it could
    /// not be parsed.
    pub(crate) id: Option<Id>,
    pub(crate) store: Store,
    pub(crate) updater: Updater,
}

impl<Store> Session<Store> {
    /// Try to load the session from the store.
    ///
    /// The return type of this method looks convoluted, so let's break it down:
    /// - The outer `Result` will return `Err(...)` if the underlying session store errors.
    /// - Otherwise, it will return `Ok(...)`, where `...` is an `Option`.
    /// - The inner `Option` will be `None` if the session was not found in the store.
    /// - Otherwise, it will be `Some(...)`, where `...` is the loaded session.
    ///
    /// # Error
    ///
    /// Errors if the underlying store errors.
    ///
    /// # Example
    /// ```rust
    /// use tower_sesh::{Session, MemoryStore, Expires};
    ///
    /// #[derive(Clone)]
    /// struct User {
    ///     id: u64,
    ///     admin: bool,
    /// }
    ///
    /// impl Expires for User {}
    ///
    /// async fn handler(session: Session<MemoryStore<User>>) -> String {
    ///     match session.load().await {
    ///         Ok(Some(session)) => {
    ///             "User has a valid session"
    ///         }
    ///         Ok(None) => {
    ///             "User does not have a session, redirect to login?"
    ///         }
    ///         Err(_error) => {
    ///             "An error occurred while loading the session"
    ///         }
    ///     }.to_string()
    /// }
    /// ```
    pub async fn load<R>(mut self) -> Result<Option<SessionState<R, Store>>, Store::Error>
    where
        R: Send + Sync,
        Store: SessionStore<R>,
    {
        Ok(if let Some(id) = self.id {
            if let Some(record) = self.store.load(&id).await? {
                Some(SessionState {
                    store: self.store,
                    id,
                    data: record,
                    updater: self.updater,
                })
            } else {
                self.updater
                    .lock()
                    .expect("lock should not be poisoned")
                    .replace(SessionUpdate::Delete);
                None
            }
        } else {
            None
        })
    }

    /// Create a new session with the given data, using the expiry from the data's `Expires` impl.
    ///
    /// # Error
    ///
    /// Errors if the underlying store errors.
    ///
    /// # Example
    /// ```rust
    /// use tower_sesh::{Session, MemoryStore, Expires};
    ///
    /// #[derive(Clone)]
    /// struct User {
    ///     id: u64,
    ///     admin: bool,
    /// }
    ///
    /// impl Expires for User {}
    ///
    /// async fn handler(session: Session<MemoryStore<User>>) -> String {
    ///     let user = User { id: 1, admin: false };
    ///     match session.create(user).await {
    ///         Ok(session) => {
    ///             "We have successfully created a new session with the user's id"
    ///         }
    ///         Err(_error) => {
    ///             "An error occurred while loading the session"
    ///         }
    ///     }.to_string()
    /// }
    /// ```
    pub async fn create<R>(self, data: R) -> Result<SessionState<R, Store>, Store::Error>
    where
        R: Expires + Send + Sync,
        Store: SessionStore<R>,
    {
        let exp = data.expires();
        self.create_with_expiry(data, exp).await
    }

    /// Create a new session with the given data and expiry. See [`Session::create`] for an example.
    ///
    /// # Error
    ///
    /// Errors if the underlying store errors.
    pub async fn create_with_expiry<R>(
        mut self,
        data: R,
        exp: Expiry,
    ) -> Result<SessionState<R, Store>, Store::Error>
    where
        R: Send + Sync,
        Store: SessionStore<R>,
    {
        let id = self.store.create(&data).await?;
        self.updater
            .lock()
            .expect("lock should not be poisoned")
            .replace(SessionUpdate::Set(id, exp));
        Ok(SessionState {
            store: self.store,
            id,
            data,
            updater: self.updater,
        })
    }
}

#[cfg(feature = "extractor")]
pub use self::extractor::*;

#[cfg(feature = "extractor")]
mod extractor {
    use super::*;
    use axum_core::{
        body::Body,
        extract::FromRequestParts,
        response::{IntoResponse, Response},
    };
    use http::request::Parts;

    /// A rejection that is returned from the [`Session`] extractor when the [`SessionManagerLayer`]
    /// middleware is not set.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[cfg_attr(docsrs, doc(cfg(feature = "extractor")))]
    pub struct NoMiddleware;

    impl std::fmt::Display for NoMiddleware {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Missing session middleware. Is it added to the app?")
        }
    }

    impl std::error::Error for NoMiddleware {}

    impl IntoResponse for NoMiddleware {
        fn into_response(self) -> Response {
            let mut resp = Response::new(Body::from(self.to_string()));
            *resp.status_mut() = http::StatusCode::INTERNAL_SERVER_ERROR;
            resp
        }
    }

    #[async_trait::async_trait]
    #[cfg_attr(docsrs, doc(cfg(feature = "extractor")))]
    impl<State, Store> FromRequestParts<State> for Session<Store>
    where
        Store: Send + Sync + 'static,
    {
        type Rejection = NoMiddleware;

        async fn from_request_parts(
            parts: &mut Parts,
            _state: &State,
        ) -> Result<Self, Self::Rejection> {
            let session = parts
                .extensions
                .remove::<Session<Store>>()
                .ok_or(NoMiddleware)?;

            Ok(session)
        }
    }
}

/// A loaded session.
///
/// When this struct refers to the "underlying store error", it is referring to the fact that the
/// store used returned a "hard" error. For example, it could be a connection error, a protocol error,
/// a timeout, etc. A counterexample would be the session not being found in the store, which is
/// not considered an error by the `SessionStore` trait.
#[derive(Debug, Clone)]
pub struct SessionState<R, Store> {
    store: Store,
    id: Id,
    data: R,
    updater: Updater,
}

impl<R, Store> SessionState<R, Store> {
    /// Read the data associated with the session.
    pub fn data(&self) -> &R {
        &self.data
    }
}

impl<R, Store> SessionState<R, Store>
where
    R: Send + Sync,
    Store: SessionStore<R>,
{
    /// Update the session data, returning the session if successful.
    ///
    /// It updates the sessions' expiry through the [`Expires`] impl. If your data does not implement
    /// [`Expires`], or you want to set a different expiry, use [`DataMut::save_with_expiry`].
    ///
    /// This method returns the `Session` if the data was saved successfully. It returns
    /// `Ok(None)` when the session was deleted or expired between the time it was loaded and the
    /// time this method is called.
    ///
    /// # Error
    ///
    /// Errors if the underlying store errors.
    ///
    /// # Example
    /// ```
    /// use tower_sesh::{SessionState, Expires, MemoryStore};
    ///
    /// #[derive(Clone)]
    /// struct User {
    ///    id: u64,
    ///    admin: bool,
    /// }
    ///
    /// impl Expires for User {}
    ///
    /// async fn upgrade_priviledges(state: SessionState<User, MemoryStore<User>>) -> Option<String> {
    ///     let new_state = state.update(|user| {
    ///         user.admin = true;
    ///     }).await.ok()??;
    ///     assert!(new_state.data().admin);
    ///     Some("User has been upgraded to admin".to_string())
    /// }
    /// ```
    pub async fn update<F>(self, update: F) -> Result<Option<SessionState<R, Store>>, Store::Error>
    where
        F: FnOnce(&mut R),
        R: Expires,
    {
        let exp = self.data.expires();
        self.update_with_expiry(update, exp).await
    }

    /// Update the session data with a provided expiry, returning the session if successful.
    ///
    /// Similar to [`SessionState::update`], but allows you to set an expiry for types that don't
    /// implement [`Expires`]. See [that method's documentation][SessionState::update] for more
    /// information.
    pub async fn update_with_expiry<F>(
        mut self,
        update: F,
        exp: Expiry,
    ) -> Result<Option<SessionState<R, Store>>, Store::Error>
    where
        F: FnOnce(&mut R),
    {
        update(&mut self.data);
        Ok(if self.store.save(&self.id, &self.data).await? {
            self.updater
                .lock()
                .expect("lock should not be poisoned")
                .replace(SessionUpdate::Set(self.id, exp));
            Some(self)
        } else {
            self.updater
                .lock()
                .expect("lock should not be poisoned")
                .replace(SessionUpdate::Delete);
            None
        })
    }

    /// Delete the session from the store.
    ///
    /// This method returns a boolean indicating whether the session was deleted from the store.
    /// If the `Store` returns `Ok(false)` if the session simply did not exist. This can happen if
    /// it was deleted by another request or if the session expired between the time it was
    /// loaded and the time this method was called.
    ///
    /// # Error
    ///
    /// Errors if the underlying store errors.
    ///
    /// # Example
    /// ```
    /// use tower_sesh::{SessionState, MemoryStore, Expires};
    ///
    /// #[derive(Clone)]
    /// struct User;
    ///
    /// impl Expires for User {}
    /// 
    /// async fn logout(state: SessionState<User, MemoryStore<User>>) -> Option<String> {
    ///     Some(if state.delete().await.ok()? {
    ///         "User has been logged out".to_string()
    ///     } else {
    ///         "User was not logged in".to_string()
    ///     })
    /// }
    pub async fn delete(mut self) -> Result<bool, Store::Error> {
        let deleted = self.store.delete(&self.id).await?;
        self.updater
            .lock()
            .expect("lock should not be poisoned")
            .replace(SessionUpdate::Delete);
        let _ = ManuallyDrop::new(self);
        Ok(deleted)
    }

    /// Cycle the session ID.
    ///
    /// This consumes the current session and returns a new session with the new ID. This method
    /// should be used to mitigate [session fixation attacks](https://www.acrossecurity.com/papers/session_fixation.pdf).
    ///
    /// This method returns `Ok(None)` if the session was deleted or expired between the time it
    /// was loaded and the time this method was called. Otherwise, it returns the new
    /// `Some(Session)`.
    ///
    /// # Error
    ///
    /// Errors if the underlying store errors.
    ///
    /// # Example
    /// ```
    /// use tower_sesh::{SessionState, MemoryStore, Expires};
    /// 
    /// #[derive(Clone)]
    /// struct User;
    ///
    /// impl Expires for User {}
    /// 
    /// async fn cycle(state: SessionState<User, MemoryStore<User>>) -> Option<String> {
    ///     Some(if let Some(new_state) = state.cycle().await.ok()? {
    ///         "Session has been cycled".to_string()
    ///     } else {
    ///         "Session was not found".to_string()
    ///     })
    /// }
    /// ```
    pub async fn cycle(self) -> Result<Option<SessionState<R, Store>>, Store::Error>
    where
        R: Expires,
    {
        let exp = self.data.expires();
        self.cycle_with_expiry(exp).await
    }

    /// Cycle the session ID with a provided expiry, instead of the one from the [`Expires`] trait.
    ///
    /// Similar to [`SessionState::cycle`], but allows you to set an expiry for types that don't
    /// implement [`Expires`]. See [that method's documentation][SessionState::cycle] for more information.
    pub async fn cycle_with_expiry(
        mut self,
        exp: Expiry,
    ) -> Result<Option<SessionState<R, Store>>, Store::Error> {
        if let Some(new_id) = self.store.cycle_id(&self.id).await? {
            self.updater
                .lock()
                .expect("lock should not be poisoned")
                .replace(SessionUpdate::Set(new_id, exp));
            self.id = new_id;
            return Ok(Some(self));
        }
        self.updater
            .lock()
            .expect("lock should not be poisoned")
            .replace(SessionUpdate::Delete);
        Ok(None)
    }

    /// Get the session store.
    pub fn into_store(self) -> Store {
        self.store
    }
}
