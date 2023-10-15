//! A middleware that provides [`Session`] as a request extension.
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use dashmap::{mapref::entry::Entry, DashMap};
use http::{Request, Response};
use time::Duration;
use tower_cookies::{cookie::SameSite, Cookie, CookieManager, Cookies};
use tower_layer::Layer;
use tower_service::Service;

use crate::{
    session::{SessionDeletion, SessionId},
    CookieConfig, Session, SessionStore,
};

#[derive(Debug, Default)]
struct LoadedSession {
    session: Session,
    refs: usize,
}

/// A middleware that provides [`Session`] as a request extension.
#[derive(Debug, Clone)]
pub struct SessionManager<S, Store: SessionStore> {
    inner: S,
    session_store: Store,
    cookie_config: CookieConfig,
    loaded_sessions: Arc<DashMap<SessionId, LoadedSession>>,
}

impl<S, Store: SessionStore> SessionManager<S, Store> {
    /// Create a new [`SessionManager`].
    pub fn new(inner: S, session_store: Store, cookie_config: CookieConfig) -> Self {
        Self {
            inner,
            session_store,
            cookie_config,
            loaded_sessions: Default::default(),
        }
    }
}

impl<ReqBody, ResBody, S, Store: SessionStore> Service<Request<ReqBody>>
    for SessionManager<S, Store>
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
        let session_store = self.session_store.clone();
        let cookie_config = self.cookie_config.clone();
        let loaded_sessions = self.loaded_sessions.clone();

        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        Box::pin(async move {
            let cookies = req
                .extensions()
                .get::<Cookies>()
                .cloned()
                .expect("Something has gone wrong with tower-cookies.");

            let mut session = if let Some(session_cookie) =
                cookies.get(&cookie_config.name).map(Cookie::into_owned)
            {
                // We do have a session cookie, so we retrieve it either from memory or the
                // backing session store.
                let session_id: SessionId = session_cookie.value().try_into()?;
                match loaded_sessions.entry(session_id) {
                    Entry::Vacant(entry) => {
                        let session = session_store.load(&session_id).await?;

                        // N.B.: Our store will *not* have the session if the session is empty.
                        if let Some(session) = &session {
                            entry.insert(LoadedSession {
                                session: session.clone(),
                                refs: 1,
                            });
                        } else {
                            cookies.remove(session_cookie);
                        }

                        session
                    }

                    Entry::Occupied(mut entry) => {
                        let loaded_session = entry.get_mut();
                        loaded_session.refs += 1;
                        Some(loaded_session.session.clone())
                    }
                }
            } else {
                // We don't have a session cookie, so let's create a new session.
                Some((&cookie_config).into())
            }
            .filter(Session::active)
            .unwrap_or_else(|| {
                // We either:
                //
                // 1. Didn't find the session in the store (but had a cookie) or,
                // 2. We found a session but it was filtered out by `Session::active`.
                //
                // In both cases we want to create a new session.
                (&cookie_config).into()
            });

            req.extensions_mut().insert(session.clone());

            let res = Ok(inner.call(req).await.map_err(Into::into)?);

            let loaded_session = loaded_sessions.entry(*session.id());

            // N.B. When a session is empty, it will be deleted. Here the deleted method
            // accounts for this check.
            if let Some(session_deletion) = session.deleted() {
                match session_deletion {
                    SessionDeletion::Deleted => {
                        if let Entry::Occupied(entry) = loaded_session {
                            entry.remove();
                        };

                        session_store.delete(session.id()).await?;
                        cookies.remove(cookie_config.build_cookie(&session));

                        // Since the session has been deleted, there's no need for further
                        // processing.
                        return res;
                    }

                    SessionDeletion::Cycled(deleted_id) => {
                        if let Entry::Occupied(entry) = loaded_session {
                            entry.remove();
                        }

                        session_store.delete(&deleted_id).await?;
                        cookies.remove(cookie_config.build_cookie(&session));

                        if session.modified() {
                            session.id = SessionId::default();
                            let session_record = (&session).into();
                            session_store.save(&session_record).await?;
                            cookies.add(cookie_config.build_cookie(&session));
                        }

                        return res;
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
                match loaded_session {
                    Entry::Occupied(mut entry) => {
                        let loaded = entry.get_mut();
                        if loaded.refs == 1 {
                            let session_record = (&session).into();
                            session_store.save(&session_record).await?;
                            cookies.add(cookie_config.build_cookie(&session));
                            entry.remove();

                            return res;
                        }

                        loaded.refs -= 1;
                    }

                    Entry::Vacant(entry) => {
                        let mut entry = entry.insert_entry(LoadedSession {
                            session: session.clone(),
                            refs: 1,
                        });
                        let session_record = (&session).into();
                        session_store.save(&session_record).await?;
                        cookies.add(cookie_config.build_cookie(&session));
                        entry.get_mut().refs -= 1;
                    }
                };
            }

            res
        })
    }
}

/// A layer for providing [`Session`] as a request extension.
#[derive(Debug, Clone)]
pub struct SessionManagerLayer<Store: SessionStore> {
    session_store: Store,
    cookie_config: CookieConfig,
}

impl<Store: SessionStore> SessionManagerLayer<Store> {
    /// Configures the name of the cookie used for the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service = SessionManagerLayer::new(session_store).with_name("my.sid");
    /// ```
    pub fn with_name(mut self, name: &str) -> Self {
        self.cookie_config.name = name.to_string();
        self
    }

    /// Configures the `"SameSite"` attribute of the cookie used for the
    /// session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{cookie::SameSite, MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service = SessionManagerLayer::new(session_store).with_same_site(SameSite::Lax);
    /// ```
    pub fn with_same_site(mut self, same_site: SameSite) -> Self {
        self.cookie_config.same_site = same_site;
        self
    }

    /// Configures the `"Max-Age"` attribute of the cookie used for the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use time::Duration;
    /// use tower_sessions::{MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service = SessionManagerLayer::new(session_store).with_max_age(Duration::hours(1));
    /// ```
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.cookie_config.max_age = Some(max_age);
        self
    }

    /// Configures the `"Secure"` attribute of the cookie used for the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service = SessionManagerLayer::new(session_store).with_secure(true);
    /// ```
    pub fn with_secure(mut self, secure: bool) -> Self {
        self.cookie_config.secure = secure;
        self
    }

    /// Configures the `"Path"` attribute of the cookie used for the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service =
    ///     SessionManagerLayer::new(session_store).with_path("/some/path".to_string());
    /// ```
    pub fn with_path(mut self, path: String) -> Self {
        self.cookie_config.path = path;
        self
    }

    /// Configures the `"Domain"` attribute of the cookie used for the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service =
    ///     SessionManagerLayer::new(session_store).with_domain("localhost".to_string());
    /// ```
    pub fn with_domain(mut self, domain: String) -> Self {
        self.cookie_config.domain = Some(domain);
        self
    }
}

impl<Store: SessionStore> SessionManagerLayer<Store> {
    /// Create a new [`SessionManagerLayer`] with the provided session store
    /// and default cookie configuration.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service = SessionManagerLayer::new(session_store);
    /// ```
    pub fn new(session_store: Store) -> Self {
        let cookie_config = CookieConfig::default();

        Self {
            session_store,
            cookie_config,
        }
    }
}

impl<S, Store: SessionStore> Layer<S> for SessionManagerLayer<Store> {
    type Service = CookieManager<SessionManager<S, Store>>;

    fn layer(&self, inner: S) -> Self::Service {
        let session_manager = SessionManager {
            inner,
            session_store: self.session_store.clone(),
            cookie_config: self.cookie_config.clone(),
            loaded_sessions: Default::default(),
        };

        CookieManager::new(session_manager)
    }
}
