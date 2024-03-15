//! A middleware that provides [`Session`] as a request extension.
use std::{
    borrow::Cow,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use http::{Request, Response};
use time::Duration;
#[cfg(any(feature = "signed", feature = "private"))]
use tower_cookies::Key;
use tower_cookies::{cookie::SameSite, Cookie, CookieManager, Cookies};
use tower_layer::Layer;
use tower_service::Service;
use tracing::Instrument;

use crate::{
    session::{self, Expiry},
    Session, SessionStore,
};

#[doc(hidden)]
pub trait CookieController: Clone + Send + Sync + 'static {
    fn get(&self, cookies: &Cookies, name: &str) -> Option<Cookie<'static>>;
    fn add(&self, cookies: &Cookies, cookie: Cookie<'static>);
    fn remove(&self, cookies: &Cookies, cookie: Cookie<'static>);
}

#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct PlaintextCookie;

impl CookieController for PlaintextCookie {
    fn get(&self, cookies: &Cookies, name: &str) -> Option<Cookie<'static>> {
        cookies.get(name).map(Cookie::into_owned)
    }

    fn add(&self, cookies: &Cookies, cookie: Cookie<'static>) {
        cookies.add(cookie)
    }

    fn remove(&self, cookies: &Cookies, cookie: Cookie<'static>) {
        cookies.remove(cookie)
    }
}

#[doc(hidden)]
#[cfg(feature = "signed")]
#[derive(Debug, Clone)]
pub struct SignedCookie {
    key: Key,
}

#[cfg(feature = "signed")]
impl CookieController for SignedCookie {
    fn get(&self, cookies: &Cookies, name: &str) -> Option<Cookie<'static>> {
        cookies.signed(&self.key).get(name).map(Cookie::into_owned)
    }

    fn add(&self, cookies: &Cookies, cookie: Cookie<'static>) {
        cookies.signed(&self.key).add(cookie)
    }

    fn remove(&self, cookies: &Cookies, cookie: Cookie<'static>) {
        cookies.signed(&self.key).remove(cookie)
    }
}

#[doc(hidden)]
#[cfg(feature = "private")]
#[derive(Debug, Clone)]
pub struct PrivateCookie {
    key: Key,
}

#[cfg(feature = "private")]
impl CookieController for PrivateCookie {
    fn get(&self, cookies: &Cookies, name: &str) -> Option<Cookie<'static>> {
        cookies.private(&self.key).get(name).map(Cookie::into_owned)
    }

    fn add(&self, cookies: &Cookies, cookie: Cookie<'static>) {
        cookies.private(&self.key).add(cookie)
    }

    fn remove(&self, cookies: &Cookies, cookie: Cookie<'static>) {
        cookies.private(&self.key).remove(cookie)
    }
}

#[derive(Debug, Clone)]
struct SessionConfig<'a> {
    name: Cow<'a, str>,
    http_only: bool,
    same_site: SameSite,
    expiry: Option<Expiry>,
    secure: bool,
    path: Cow<'a, str>,
    domain: Option<Cow<'a, str>>,
}

impl<'a> SessionConfig<'a> {
    fn build_cookie(self, session_id: session::Id, expiry_age: Duration) -> Cookie<'a> {
        let mut cookie_builder = Cookie::build((self.name, session_id.to_string()))
            .http_only(self.http_only)
            .same_site(self.same_site)
            .secure(self.secure)
            .path(self.path);

        if !matches!(self.expiry, Some(Expiry::OnSessionEnd) | None) {
            cookie_builder = cookie_builder.max_age(expiry_age);
        }

        if let Some(domain) = self.domain {
            cookie_builder = cookie_builder.domain(domain);
        }

        cookie_builder.build()
    }
}

impl<'a> Default for SessionConfig<'a> {
    fn default() -> Self {
        Self {
            name: "id".into(), /* See: https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html#session-id-name-fingerprinting */
            http_only: true,
            same_site: SameSite::Strict,
            expiry: None, // TODO: Is `Max-Age: "Session"` the right default?
            secure: true,
            path: "/".into(),
            domain: None,
        }
    }
}

/// A middleware that provides [`Session`] as a request extension.
#[derive(Debug, Clone)]
pub struct SessionManager<S, Store: SessionStore, C: CookieController = PlaintextCookie> {
    inner: S,
    session_store: Arc<Store>,
    session_config: SessionConfig<'static>,
    cookie_controller: C,
}

impl<S, Store: SessionStore> SessionManager<S, Store> {
    /// Create a new [`SessionManager`].
    pub fn new(inner: S, session_store: Store) -> Self {
        Self {
            inner,
            session_store: Arc::new(session_store),
            session_config: Default::default(),
            cookie_controller: PlaintextCookie,
        }
    }
}

impl<ReqBody, ResBody, S, Store: SessionStore, C: CookieController> Service<Request<ReqBody>>
    for SessionManager<S, Store, C>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send,
    ReqBody: Send + 'static,
    ResBody: Default + Send,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let span = tracing::info_span!("call");

        let session_store = self.session_store.clone();
        let session_config = self.session_config.clone();
        let cookie_controller = self.cookie_controller.clone();

        // Because the inner service can panic until ready, we need to ensure we only
        // use the ready service.
        //
        // See: https://docs.rs/tower/latest/tower/trait.Service.html#be-careful-when-cloning-inner-services
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(
            async move {
                let Some(cookies) = req.extensions().get::<Cookies>().cloned() else {
                    // In practice this should never happen because we wrap `CookieManager`
                    // directly.
                    tracing::error!("missing cookies request extension");
                    return Ok(Response::default());
                };

                let session_cookie = cookie_controller.get(&cookies, &session_config.name);
                let session_id = session_cookie.as_ref().and_then(|cookie| {
                    cookie
                        .value()
                        .parse::<session::Id>()
                        .map_err(|err| {
                            tracing::warn!(
                                err = %err,
                                "possibly suspicious activity: malformed session id"
                            )
                        })
                        .ok()
                });

                let session = Session::new(session_id, session_store, session_config.expiry);

                req.extensions_mut().insert(session.clone());

                let res = inner.call(req).await?;

                let modified = session.is_modified();
                let empty = session.is_empty().await;

                tracing::trace!(modified = modified, empty = empty, "session response state");

                match session_cookie {
                    Some(mut cookie) if empty => {
                        tracing::debug!("removing session cookie");

                        // Path and domain must be manually set to ensure a proper removal cookie is
                        // constructed.
                        //
                        // See: https://docs.rs/cookie/latest/cookie/struct.CookieJar.html#method.remove
                        cookie.set_path(session_config.path);
                        if let Some(domain) = session_config.domain {
                            cookie.set_domain(domain);
                        }

                        cookie_controller.remove(&cookies, cookie);
                    }

                    // TODO: We can consider an "always save" configuration option:
                    _ if modified && !empty && !res.status().is_server_error() => {
                        tracing::debug!("saving session");
                        if let Err(err) = session.save().await {
                            tracing::error!(err = %err, "failed to save session");

                            let mut res = Response::default();
                            *res.status_mut() = http::StatusCode::INTERNAL_SERVER_ERROR;
                            return Ok(res);
                        }

                        let Some(session_id) = session.id() else {
                            tracing::error!("missing session id");

                            let mut res = Response::default();
                            *res.status_mut() = http::StatusCode::INTERNAL_SERVER_ERROR;
                            return Ok(res);
                        };

                        let expiry_age = session.expiry_age();
                        let session_cookie = session_config.build_cookie(session_id, expiry_age);

                        tracing::debug!("adding session cookie");
                        cookie_controller.add(&cookies, session_cookie);
                    }

                    _ => (),
                };

                Ok(res)
            }
            .instrument(span),
        )
    }
}

/// A layer for providing [`Session`] as a request extension.
#[derive(Debug, Clone)]
pub struct SessionManagerLayer<Store: SessionStore, C: CookieController = PlaintextCookie> {
    session_store: Arc<Store>,
    session_config: SessionConfig<'static>,
    cookie_controller: C,
}

impl<Store: SessionStore, C: CookieController> SessionManagerLayer<Store, C> {
    /// Configures the name of the cookie used for the session.
    /// The default value is `"id"`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service = SessionManagerLayer::new(session_store).with_name("my.sid");
    /// ```
    pub fn with_name<N: Into<Cow<'static, str>>>(mut self, name: N) -> Self {
        self.session_config.name = name.into();
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
    /// The default value is [`SameSite::Strict`].
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
    /// The default value is `None`.
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
    /// The default value is `true`.
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
    /// The default value is `"/"`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service = SessionManagerLayer::new(session_store).with_path("/some/path");
    /// ```
    pub fn with_path<P: Into<Cow<'static, str>>>(mut self, path: P) -> Self {
        self.session_config.path = path.into();
        self
    }

    /// Configures the `"Domain"` attribute of the cookie used for the session.
    /// The default value is `None`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service = SessionManagerLayer::new(session_store).with_domain("localhost");
    /// ```
    pub fn with_domain<D: Into<Cow<'static, str>>>(mut self, domain: D) -> Self {
        self.session_config.domain = Some(domain.into());
        self
    }

    /// Manages the session cookie via a signed interface.
    ///
    /// See [`SignedCookies`](tower_cookies::SignedCookies).
    ///
    /// ```rust
    /// use tower_sessions::{cookie::Key, MemoryStore, SessionManagerLayer};
    ///
    /// # /*
    /// let key = { /* a cryptographically random key >= 64 bytes */ };
    /// # */
    /// # let key: &Vec<u8> = &(0..64).collect();
    /// # let key: &[u8] = &key[..];
    /// let key = Key::try_from(key).unwrap();
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service = SessionManagerLayer::new(session_store).with_signed(key);
    /// ```
    #[cfg(feature = "signed")]
    pub fn with_signed(self, key: Key) -> SessionManagerLayer<Store, SignedCookie> {
        SessionManagerLayer::<Store, SignedCookie> {
            session_store: self.session_store,
            session_config: self.session_config,
            cookie_controller: SignedCookie { key },
        }
    }

    /// Manages the session cookie via an encrypted interface.
    ///
    /// See [`PrivateCookies`](tower_cookies::PrivateCookies).
    ///
    /// ```rust
    /// use tower_sessions::{cookie::Key, MemoryStore, SessionManagerLayer};
    ///
    /// # /*
    /// let key = { /* a cryptographically random key >= 64 bytes */ };
    /// # */
    /// # let key: &Vec<u8> = &(0..64).collect();
    /// # let key: &[u8] = &key[..];
    /// let key = Key::try_from(key).unwrap();
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service = SessionManagerLayer::new(session_store).with_private(key);
    /// ```
    #[cfg(feature = "private")]
    pub fn with_private(self, key: Key) -> SessionManagerLayer<Store, PrivateCookie> {
        SessionManagerLayer::<Store, PrivateCookie> {
            session_store: self.session_store,
            session_config: self.session_config,
            cookie_controller: PrivateCookie { key },
        }
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
            session_store: Arc::new(session_store),
            session_config,
            cookie_controller: PlaintextCookie,
        }
    }
}

impl<S, Store: SessionStore, C: CookieController> Layer<S> for SessionManagerLayer<Store, C> {
    type Service = CookieManager<SessionManager<S, Store, C>>;

    fn layer(&self, inner: S) -> Self::Service {
        let session_manager = SessionManager {
            inner,
            session_store: self.session_store.clone(),
            session_config: self.session_config.clone(),
            cookie_controller: self.cookie_controller.clone(),
        };

        CookieManager::new(session_manager)
    }
}
