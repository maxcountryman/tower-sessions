//! A middleware that provides [`Session`] as a request extension.
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use http::{Request, Response};
use time::Duration;
use tower_cookies::{CookieManager, Cookies};
use tower_layer::Layer;
use tower_service::Service;

use crate::{
    cookie::SameSite,
    session::{SessionDeletion, SessionId},
    CookieConfig, Session, SessionStore,
};

/// A manager for creating and configuring [`Session`]s.
#[derive(Debug, Clone)]
pub struct SessionManager<Store: SessionStore> {
    session_store: Store,
    cookie_config: CookieConfig,
}

impl<Store: SessionStore> SessionManager<Store> {
    /// Create a new [`SessionManager`].
    pub fn new(session_store: Store, cookie_config: CookieConfig) -> Self {
        Self {
            session_store,
            cookie_config,
        }
    }

    /// Create a new [`Session`] with default configuration.
    pub fn create_session(&self) -> Option<Session> {
        Some((&self.cookie_config).into())
    }
}

/// A middleware that provides [`Session`] as a request extension.
#[derive(Debug, Clone)]
pub struct SessionHandler<S, Store: SessionStore> {
    inner: S,
    session_manager: SessionManager<Store>,
}

impl<ReqBody, ResBody, S, Store: SessionStore> Service<Request<ReqBody>>
    for SessionHandler<S, Store>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>> + 'static,
    S::Future: Send,
    ReqBody: Send + 'static,
    ResBody: Send,
{
    type Response = S::Response;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let manager = self.session_manager.clone();

        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        Box::pin(async move {
            let cookies = req
                .extensions()
                .get::<Cookies>()
                .cloned()
                .expect("Something has gone wrong with tower-cookies.");

            let mut session =
                if let Some(session_cookie) = cookies.get(&manager.cookie_config.name) {
                    // We do have a session cookie, so let's see if our store has the associated
                    // session.
                    //
                    // N.B.: Our store will *not* have the session if the session is empty.
                    let session_id = session_cookie.value().try_into()?;
                    manager.session_store.load(&session_id).await?
                } else {
                    // We don't have a session cookie, so let's create a new session.
                    manager.create_session()
                }
                .filter(Session::active)
                .unwrap_or_else(|| {
                    // We either:
                    //
                    // 1. Didn't find the session in the store (but had a cookie) or,
                    // 2. We found a session but it was filtered out by `Session::active`.
                    //
                    // In both cases we want to create a new session.
                    manager.create_session().unwrap()
                });

            req.extensions_mut().insert(session.clone());

            let res = Ok(inner.call(req).await.map_err(Into::into)?);

            if let Some(session_deletion) = session.deleted() {
                match session_deletion {
                    SessionDeletion::Deleted => {
                        manager.session_store.delete(&session.id()).await?;
                        cookies.remove(manager.cookie_config.build_cookie(&session));

                        // Since the session has been deleted, there's no need for further
                        // processing.
                        return res;
                    }

                    SessionDeletion::Cycled(deleted_id) => {
                        manager.session_store.delete(&deleted_id).await?;
                        session.id = SessionId::default();
                    }
                }
            };

            // For further consideration:
            //
            // We only persist the session in the store when the `modified` flag is set.
            //
            // However, we could offer additional configuration of this behavior via an
            // extended interface in the future. For instance, we might consider providing
            // the `Set-Cookie` header whenever modified or if some "always save" marker is
            // set.
            if session.modified() {
                let session_record = (&session).into();
                manager.session_store.save(&session_record).await?;
                cookies.add(manager.cookie_config.build_cookie(&session))
            }

            res
        })
    }
}

/// A layer for providing [`Session`] as a request extension.
#[derive(Debug, Clone)]
pub struct SessionManagerLayer<Store: SessionStore> {
    manager: SessionManager<Store>,
}

impl<Store: SessionStore> SessionManagerLayer<Store> {
    /// Create a new [`SessionManagerLayer`] with the provided session store
    /// and default cookie configuration.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager = SessionManager::new(session_store, CookieConfig::default());
    /// let session_service = SessionManagerLayer::new(session_manager);
    /// ```
    pub fn new(session_manager: SessionManager<Store>) -> Self {
        Self {
            manager: session_manager,
        }
    }

    /// Configures the name of the cookie used for the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager = SessionManager::new(session_store, CookieConfig::default());
    /// let session_service = SessionManagerLayer::new(session_manager).with_name("my.sid");
    /// ```
    #[deprecated(note = "Use `CookieConfig::with_name` instead.")]
    pub fn with_name(mut self, name: &str) -> Self {
        self.manager.cookie_config.name = name.to_string();
        self
    }

    /// Configures the `"SameSite"` attribute of the cookie used for the
    /// session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_cookies::cookie::SameSite;
    /// use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager = SessionManager::new(session_store, CookieConfig::default());
    /// let session_service = SessionManagerLayer::new(session_manager).with_same_site(SameSite::Lax);
    /// ```
    #[deprecated(note = "Use `CookieConfig::with_same_site` instead.")]
    pub fn with_same_site(mut self, same_site: SameSite) -> Self {
        self.manager.cookie_config.same_site = same_site;
        self
    }

    /// Configures the `"Max-Age"` attribute of the cookie used for the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use time::Duration;
    /// use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager = SessionManager::new(session_store, CookieConfig::default());
    /// let session_service =
    ///     SessionManagerLayer::new(session_manager).with_max_age(Duration::hours(1));
    /// ```
    #[deprecated(note = "Use `CookieConfig::with_max_age` instead.")]
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.manager.cookie_config.max_age = Some(max_age);
        self
    }

    /// Configures the `"Secure"` attribute of the cookie used for the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager = SessionManager::new(session_store, CookieConfig::default());
    /// let session_service = SessionManagerLayer::new(session_manager).with_secure(true);
    /// ```
    #[deprecated(note = "Use `CookieConfig::with_secure` instead.")]
    pub fn with_secure(mut self, secure: bool) -> Self {
        self.manager.cookie_config.secure = secure;
        self
    }

    /// Configures the `"Path"` attribute of the cookie used for the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager = SessionManager::new(session_store, CookieConfig::default());
    /// let session_service =
    ///     SessionManagerLayer::new(session_manager).with_path("/some/path".to_string());
    /// ```
    #[deprecated(note = "Use `CookieConfig::with_path` instead.")]
    pub fn with_path(mut self, path: String) -> Self {
        self.manager.cookie_config.path = path;
        self
    }

    /// Configures the `"Domain"` attribute of the cookie used for the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager = SessionManager::new(session_store, CookieConfig::default());
    /// let session_service =
    ///     SessionManagerLayer::new(session_manager).with_domain("localhost".to_string());
    /// ```
    #[deprecated(note = "Use `CookieConfig::with_domain` instead.")]
    pub fn with_domain(mut self, domain: String) -> Self {
        self.manager.cookie_config.domain = Some(domain);
        self
    }
}

impl<S, Store: SessionStore> Layer<S> for SessionManagerLayer<Store> {
    type Service = CookieManager<SessionHandler<S, Store>>;

    fn layer(&self, inner: S) -> Self::Service {
        let session_handler = SessionHandler {
            inner,
            session_manager: self.manager.clone(),
        };

        CookieManager::new(session_handler)
    }
}
