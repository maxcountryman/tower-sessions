//! A middleware that provides [`Session`] as a request extension.
use std::{
    borrow::Cow,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use http::{Request, Response};
use time::OffsetDateTime;
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
pub trait CookieController: Clone + Send + 'static {
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
    always_save: bool,
}

impl<'a> SessionConfig<'a> {
    fn build_cookie(self, session_id: session::Id, expiry: Option<Expiry>) -> Cookie<'a> {
        let mut cookie_builder = Cookie::build((self.name, session_id.to_string()))
            .http_only(self.http_only)
            .same_site(self.same_site)
            .secure(self.secure)
            .path(self.path);

        cookie_builder = match expiry {
            Some(Expiry::OnInactivity(duration)) => cookie_builder.max_age(duration),
            Some(Expiry::AtDateTime(datetime)) => {
                cookie_builder.max_age(datetime - OffsetDateTime::now_utc())
            }
            Some(Expiry::OnSessionEnd) | None => cookie_builder,
        };

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
            always_save: false,
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
                let Some(cookies) = req.extensions().get::<_>().cloned() else {
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

                tracing::trace!(
                    modified = modified,
                    empty = empty,
                    always_save = session_config.always_save,
                    "session response state",
                );

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

                    _ if (modified || session_config.always_save)
                        && !empty
                        && !res.status().is_client_error()
                        && !res.status().is_server_error() =>
                    {
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

                        let expiry = session.expiry();
                        let session_cookie = session_config.build_cookie(session_id, expiry);

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

    /// Configures whether unmodified session should be saved on read or not.
    /// When the value is `true`, the session will be saved even if it was not
    /// changed.
    ///
    /// This is useful when you want to reset [`Session`] expiration time
    /// on any valid request at the cost of higher [`SessionStore`] write
    /// activity and transmitting `set-cookie` header with each response.
    ///
    /// It makes sense to use this setting with relative session expiration
    /// values, such as `Expiry::OnInactivity(Duration)`. This setting will
    /// _not_ cause session id to be cycled on save.
    ///
    /// The default value is `false`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use time::Duration;
    /// use tower_sessions::{Expiry, MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_expiry = Expiry::OnInactivity(Duration::hours(1));
    /// let session_service = SessionManagerLayer::new(session_store)
    ///     .with_expiry(session_expiry)
    ///     .with_always_save(true);
    /// ```
    pub fn with_always_save(mut self, always_save: bool) -> Self {
        self.session_config.always_save = always_save;
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
    /// # let key = Key::try_from(key).unwrap();
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
    /// # let key = Key::try_from(key).unwrap();
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use anyhow::anyhow;
    use axum::body::Body;
    use tower::{ServiceBuilder, ServiceExt};
    use tower_sessions_memory_store::MemoryStore;

    use crate::session::{Id, Record};

    use super::*;

    async fn handler(req: Request<Body>) -> anyhow::Result<Response<Body>> {
        let session = req
            .extensions()
            .get::<Session>()
            .ok_or(anyhow!("Missing session"))?;

        session.insert("foo", 42).await?;

        Ok(Response::new(Body::empty()))
    }

    async fn noop_handler(_: Request<Body>) -> anyhow::Result<Response<Body>> {
        Ok(Response::new(Body::empty()))
    }

    #[tokio::test]
    async fn basic_service_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.clone().oneshot(req).await?;

        let session = res.headers().get(http::header::SET_COOKIE);
        assert!(session.is_some());

        let req = Request::builder()
            .header(http::header::COOKIE, session.unwrap())
            .body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(res.headers().get(http::header::SET_COOKIE).is_none());

        Ok(())
    }

    #[tokio::test]
    async fn bogus_cookie_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.clone().oneshot(req).await?;

        assert!(res.headers().get(http::header::SET_COOKIE).is_some());

        let req = Request::builder()
            .header(http::header::COOKIE, "id=bogus")
            .body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(res.headers().get(http::header::SET_COOKIE).is_some());

        Ok(())
    }

    #[tokio::test]
    async fn no_set_cookie_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(noop_handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(res.headers().get(http::header::SET_COOKIE).is_none());

        Ok(())
    }

    #[tokio::test]
    async fn name_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store).with_name("my.sid");
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(cookie_value_matches(&res, |s| s.starts_with("my.sid=")));

        Ok(())
    }

    #[tokio::test]
    async fn http_only_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(cookie_value_matches(&res, |s| s.contains("HttpOnly")));

        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store).with_http_only(false);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(cookie_value_matches(&res, |s| !s.contains("HttpOnly")));

        Ok(())
    }

    #[tokio::test]
    async fn same_site_strict_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer =
            SessionManagerLayer::new(session_store).with_same_site(SameSite::Strict);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(cookie_value_matches(&res, |s| s.contains("SameSite=Strict")));

        Ok(())
    }

    #[tokio::test]
    async fn same_site_lax_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store).with_same_site(SameSite::Lax);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(cookie_value_matches(&res, |s| s.contains("SameSite=Lax")));

        Ok(())
    }

    #[tokio::test]
    async fn same_site_none_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store).with_same_site(SameSite::None);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(cookie_value_matches(&res, |s| s.contains("SameSite=None")));

        Ok(())
    }

    #[tokio::test]
    async fn expiry_on_session_end_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer =
            SessionManagerLayer::new(session_store).with_expiry(Expiry::OnSessionEnd);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(cookie_value_matches(&res, |s| !s.contains("Max-Age")));

        Ok(())
    }

    #[tokio::test]
    async fn expiry_on_inactivity_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let inactivity_duration = time::Duration::hours(2);
        let session_layer = SessionManagerLayer::new(session_store)
            .with_expiry(Expiry::OnInactivity(inactivity_duration));
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        let expected_max_age = inactivity_duration.whole_seconds();
        assert!(cookie_has_expected_max_age(&res, expected_max_age));

        Ok(())
    }

    #[tokio::test]
    async fn expiry_at_date_time_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let expiry_time = time::OffsetDateTime::now_utc() + time::Duration::weeks(1);
        let session_layer =
            SessionManagerLayer::new(session_store).with_expiry(Expiry::AtDateTime(expiry_time));
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        let expected_max_age = (expiry_time - time::OffsetDateTime::now_utc()).whole_seconds();
        assert!(cookie_has_expected_max_age(&res, expected_max_age));

        Ok(())
    }

    #[tokio::test]
    async fn expiry_on_session_end_always_save_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store.clone())
            .with_expiry(Expiry::OnSessionEnd)
            .with_always_save(true);
        let mut svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req1 = Request::builder().body(Body::empty())?;
        let res1 = svc.call(req1).await?;
        let sid1 = get_session_id(&res1);
        let rec1 = get_record(&session_store, &sid1).await;
        let req2 = Request::builder()
            .header(http::header::COOKIE, &format!("id={}", sid1))
            .body(Body::empty())?;
        let res2 = svc.call(req2).await?;
        let sid2 = get_session_id(&res2);
        let rec2 = get_record(&session_store, &sid2).await;

        assert!(cookie_value_matches(&res2, |s| !s.contains("Max-Age")));
        assert!(sid1 == sid2);
        assert!(rec1.expiry_date < rec2.expiry_date);

        Ok(())
    }

    #[tokio::test]
    async fn expiry_on_inactivity_always_save_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let inactivity_duration = time::Duration::hours(2);
        let session_layer = SessionManagerLayer::new(session_store.clone())
            .with_expiry(Expiry::OnInactivity(inactivity_duration))
            .with_always_save(true);
        let mut svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req1 = Request::builder().body(Body::empty())?;
        let res1 = svc.call(req1).await?;
        let sid1 = get_session_id(&res1);
        let rec1 = get_record(&session_store, &sid1).await;
        let req2 = Request::builder()
            .header(http::header::COOKIE, &format!("id={}", sid1))
            .body(Body::empty())?;
        let res2 = svc.call(req2).await?;
        let sid2 = get_session_id(&res2);
        let rec2 = get_record(&session_store, &sid2).await;

        let expected_max_age = inactivity_duration.whole_seconds();
        assert!(cookie_has_expected_max_age(&res2, expected_max_age));
        assert!(sid1 == sid2);
        assert!(rec1.expiry_date < rec2.expiry_date);

        Ok(())
    }

    #[tokio::test]
    async fn expiry_at_date_time_always_save_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let expiry_time = time::OffsetDateTime::now_utc() + time::Duration::weeks(1);
        let session_layer = SessionManagerLayer::new(session_store.clone())
            .with_expiry(Expiry::AtDateTime(expiry_time))
            .with_always_save(true);
        let mut svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req1 = Request::builder().body(Body::empty())?;
        let res1 = svc.call(req1).await?;
        let sid1 = get_session_id(&res1);
        let rec1 = get_record(&session_store, &sid1).await;
        let req2 = Request::builder()
            .header(http::header::COOKIE, &format!("id={}", sid1))
            .body(Body::empty())?;
        let res2 = svc.call(req2).await?;
        let sid2 = get_session_id(&res2);
        let rec2 = get_record(&session_store, &sid2).await;

        let expected_max_age = (expiry_time - time::OffsetDateTime::now_utc()).whole_seconds();
        assert!(cookie_has_expected_max_age(&res2, expected_max_age));
        assert!(sid1 == sid2);
        assert!(rec1.expiry_date == rec2.expiry_date);

        Ok(())
    }

    #[tokio::test]
    async fn secure_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store).with_secure(true);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(cookie_value_matches(&res, |s| s.contains("Secure")));

        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store).with_secure(false);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(cookie_value_matches(&res, |s| !s.contains("Secure")));

        Ok(())
    }

    #[tokio::test]
    async fn path_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store).with_path("/foo/bar");
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(cookie_value_matches(&res, |s| s.contains("Path=/foo/bar")));

        Ok(())
    }

    #[tokio::test]
    async fn domain_test() -> anyhow::Result<()> {
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store).with_domain("example.com");
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(cookie_value_matches(&res, |s| s.contains("Domain=example.com")));

        Ok(())
    }

    #[cfg(feature = "signed")]
    #[tokio::test]
    async fn signed_test() -> anyhow::Result<()> {
        let key = Key::generate();
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store).with_signed(key);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(res.headers().get(http::header::SET_COOKIE).is_some());

        Ok(())
    }

    #[cfg(feature = "private")]
    #[tokio::test]
    async fn private_test() -> anyhow::Result<()> {
        let key = Key::generate();
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store).with_private(key);
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(res.headers().get(http::header::SET_COOKIE).is_some());

        Ok(())
    }

    fn cookie_value_matches<F>(res: &Response<Body>, matcher: F) -> bool
    where
        F: FnOnce(&str) -> bool,
    {
        res.headers()
            .get(http::header::SET_COOKIE)
            .is_some_and(|set_cookie| set_cookie.to_str().is_ok_and(matcher))
    }

    fn cookie_has_expected_max_age(res: &Response<Body>, expected_value: i64) -> bool {
        res.headers()
            .get(http::header::SET_COOKIE)
            .is_some_and(|set_cookie| {
                set_cookie.to_str().is_ok_and(|s| {
                    let max_age_value = s
                        .split("Max-Age=")
                        .nth(1)
                        .unwrap_or_default()
                        .split(';')
                        .next()
                        .unwrap_or_default()
                        .parse::<i64>()
                        .unwrap_or_default();
                    (max_age_value - expected_value).abs() <= 1
                })
            })
    }

    fn get_session_id(res: &Response<Body>) -> String {
        res.headers()
            .get(http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split("id=")
            .nth(1)
            .unwrap()
            .split(";")
            .next()
            .unwrap()
            .to_string()
    }

    async fn get_record(store: &impl SessionStore, id: &str) -> Record {
        store
            .load(&Id::from_str(id).unwrap())
            .await
            .unwrap()
            .unwrap()
    }
}
