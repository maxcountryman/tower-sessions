//! A middleware that provides [`Session`] as a request extension.
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use http::{Request, Response};
use tower_cookies::{cookie::SameSite, Cookie, CookieManager, Cookies};
use tower_layer::Layer;
use tower_service::Service;
use tracing::Instrument;

use crate::{
    session::{Deletion, Expiry, Id},
    Session, SessionStore,
};

#[derive(Debug, Clone)]
struct SessionConfig {
    name: String,
    http_only: bool,
    same_site: SameSite,
    expiry: Option<Expiry>,
    secure: bool,
    path: String,
    domain: Option<String>,
}

impl SessionConfig {
    fn build_cookie<'c>(&self, session: &Session) -> Cookie<'c> {
        let mut cookie_builder = Cookie::build((self.name.clone(), session.id().to_string()))
            .http_only(self.http_only)
            .same_site(self.same_site)
            .secure(self.secure)
            .path(self.path.clone());

        cookie_builder = cookie_builder.max_age(session.expiry_age());

        if let Some(domain) = &self.domain {
            cookie_builder = cookie_builder.domain(domain.clone());
        }

        cookie_builder.build()
    }

    fn new_session(&self) -> Session {
        Session::new(self.expiry.clone())
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            name: String::from("id"), /* See: https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html#session-id-name-fingerprinting */
            http_only: true,
            same_site: SameSite::Strict,
            expiry: None, // TODO: Is `Max-Age: "Session"` the right default?
            secure: false,
            path: String::from("/"),
            domain: None,
        }
    }
}

/// A middleware that provides [`Session`] as a request extension.
#[derive(Debug, Clone)]
pub struct SessionManager<S, Store: SessionStore> {
    inner: S,
    session_store: Store,
    session_config: SessionConfig,
}

impl<S, Store: SessionStore> SessionManager<S, Store> {
    /// Create a new [`SessionManager`].
    pub fn new(inner: S, session_store: Store) -> Self {
        Self {
            inner,
            session_store,
            session_config: Default::default(),
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
        let span = tracing::debug_span!("session_middleware", session.id = tracing::field::Empty);

        let session_store = self.session_store.clone();
        let session_config = self.session_config.clone();

        // This is necessary to prevent potential panics.
        //
        // See: https://docs.rs/tower/latest/tower/trait.Service.html#be-careful-when-cloning-inner-services
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(
            async move {
                let cookies = req
                    .extensions()
                    .get::<Cookies>()
                    .cloned()
                    .expect("Something has gone wrong with tower-cookies.");

                let mut has_session_cookie = false;
                let mut session = if let Some(session_cookie) =
                    cookies.get(&session_config.name).map(Cookie::into_owned)
                {
                    // We do have a session cookie, so we retrieve it either from memory or the
                    // backing session store.
                    tracing::debug!("loading session from cookie");
                    has_session_cookie = true;
                    let session_id = session_cookie.value().try_into()?;

                    let session = session_store.load(&session_id).await?;
                    tracing::trace!("loaded from store");

                    // N.B.: Our store will *not* have the session if the session is empty.
                    if session.is_none() {
                        cookies.remove(session_cookie);
                    }

                    session.unwrap_or_else(|| session_config.new_session())
                } else {
                    // We don't have a session cookie, so let's create a new session.
                    let session = session_config.new_session();
                    tracing::debug!("created new session");
                    session
                };

                tracing::Span::current().record("session.id", session.id().to_string());

                req.extensions_mut().insert(session.clone());

                let res = Ok(inner.call(req).await.map_err(Into::into)?);

                // N.B. When a session is empty, it will be deleted. Here the deleted method
                // accounts for this check.
                if let Some(session_deletion) = session.deleted() {
                    match session_deletion {
                        Deletion::Deleted => {
                            tracing::debug!("deleted state");

                            if has_session_cookie {
                                session_store.delete(session.id()).await?;
                                cookies.remove(session_config.build_cookie(&session));

                                tracing::trace!("deleted from store");
                            }

                            // Since the session has been deleted, there's no need for further
                            // processing.
                            return res;
                        }

                        Deletion::Cycled(deleted_id) => {
                            tracing::debug!("cycled state");

                            session_store.delete(&deleted_id).await?;
                            cookies.remove(session_config.build_cookie(&session));
                            session.reset_deleted();

                            session.id = Id::default();
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
                if session.is_modified() {
                    tracing::debug!("modified state");
                    session.reset_modified();

                    session_store.save(&session).await?;
                    cookies.add(session_config.build_cookie(&session));
                }

                res
            }
            .instrument(span),
        )
    }
}

/// A layer for providing [`Session`] as a request extension.
#[derive(Debug, Clone)]
pub struct SessionManagerLayer<Store: SessionStore> {
    session_store: Store,
    session_config: SessionConfig,
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
        self.session_config.name = name.to_string();
        self
    }

    /// Configures the `"HttpOnly"` attribute of the cookie used for the
    /// session.
    ///
    /// # ⚠️ **Warning: Cross-site scripting risk**
    ///
    /// Applications should generally **not** override the default value of
    /// `true`. If you do, you are exposing your application to increased risk
    /// of cookie theft via techniques like cross-site scripting.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service = SessionManagerLayer::new(session_store).with_http_only(true);
    /// ```
    pub fn with_http_only(mut self, http_only: bool) -> Self {
        self.session_config.http_only = http_only;
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
        self.session_config.same_site = same_site;
        self
    }

    /// Configures the `"Max-Age"` attribute of the cookie used for the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use time::Duration;
    /// use tower_sessions::{Expiry, MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_expiry = Expiry::OnInactivity(Duration::hours(1));
    /// let session_service = SessionManagerLayer::new(session_store).with_expiry(session_expiry);
    /// ```
    pub fn with_expiry(mut self, expiry: Expiry) -> Self {
        self.session_config.expiry = Some(expiry);
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
        self.session_config.secure = secure;
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
        self.session_config.path = path;
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
        self.session_config.domain = Some(domain);
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
        let session_config = SessionConfig::default();

        Self {
            session_store,
            session_config,
        }
    }
}

impl<S, Store: SessionStore> Layer<S> for SessionManagerLayer<Store> {
    type Service = CookieManager<SessionManager<S, Store>>;

    fn layer(&self, inner: S) -> Self::Service {
        let session_manager = SessionManager {
            inner,
            session_store: self.session_store.clone(),
            session_config: self.session_config.clone(),
        };

        CookieManager::new(session_manager)
    }
}
