//! A middleware that provides [`Session`] as a request extension.
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use cookie::{Cookie, SameSite};
use http::{header::COOKIE, Request, Response};
use pin_project_lite::pin_project;
use time::OffsetDateTime;
use tower_layer::Layer;
use tower_service::Service;
use tower_sesh_core::{expires::Expiry, id::Id};
use tracing::{instrument::Instrumented, Instrument};

use crate::{
    session::{SessionUpdate, Updater},
    Session,
};

/// the configuration options for the [`SessionManagerLayer`].
///
/// ## Default
/// ```
/// # use tower_sesh::middleware::Config;
/// # use tower_sesh::Expiry;
/// # use cookie::SameSite;
/// let default = Config {
///    name: "id",
///    http_only: true,
///    same_site: SameSite::Strict,
///    secure: true,
///    path: "/",
///    domain: None,
///    always_set_expiry: None,
/// };
///
/// assert_eq!(default, Config::default());
/// ```
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Config<'a> {
    /// The name of the cookie.
    pub name: &'a str,
    /// Whether the cookie is [HTTP only](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie#httponly).
    pub http_only: bool,
    /// The
    /// [SameSite](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie#samesitesamesite-value)
    /// policy.
    pub same_site: SameSite,
    /// Whether the cookie should be
    /// [secure](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie#secure).
    pub secure: bool,
    /// The [path](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie#pathpath-value)
    /// attribute of the cookie.
    pub path: &'a str,
    /// The
    /// [domain](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie#domaindomain-value)
    /// attribute of the cookie.
    pub domain: Option<&'a str>,
    /// If this is set to `None`, the session will only be saved if it is modified. If it is set to
    /// `Some(expiry)`, the session will be saved as usual if it is modified, but it will also be
    /// saved with the provided `expiry` when it is not modified.
    ///
    /// This manages the
    /// [`Max-Age`](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie#max-agenumber)
    /// and the
    /// [`Expires`](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie#expiresdate)
    /// attributes.
    pub always_set_expiry: Option<Expiry>,
}

impl<'a> Config<'a> {
    fn build_cookie(self, session_id: Option<Id>, expiry: Expiry) -> Cookie<'a> {
        let mut cookie_builder = Cookie::build((
            self.name,
            session_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
        ))
        .http_only(self.http_only)
        .same_site(self.same_site)
        .secure(self.secure)
        .path(self.path);

        cookie_builder = match expiry {
            Expiry::OnInactivity(duration) => cookie_builder.max_age(duration),
            Expiry::AtDateTime(datetime) => {
                cookie_builder.max_age(datetime - OffsetDateTime::now_utc())
            }
            Expiry::OnSessionEnd => cookie_builder,
        };

        if let Some(domain) = self.domain {
            cookie_builder = cookie_builder.domain(domain);
        }

        cookie_builder.build()
    }
}

impl Default for Config<'static> {
    fn default() -> Self {
        Self {
            name: "id", /* See: https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html#session-id-name-fingerprinting */
            http_only: true,
            same_site: SameSite::Strict,
            secure: true,
            path: "/",
            domain: None,
            always_set_expiry: None,
        }
    }
}

/// A middleware that provides [`Session`] as a request extension.
#[derive(Debug, Clone)]
pub struct SessionManager<Store, S> {
    inner: S,
    store: Store,
    config: Config<'static>,
}

impl<Store, S> SessionManager<Store, S> {
    /// Create a new [`SessionManager`].
    ///
    /// # Examples
    /// ```
    /// use tower_sesh::{MemoryStore, SessionManager};
    ///
    /// struct MyService;
    ///
    /// let _ = SessionManager::new(MyService, MemoryStore::<()>::default(), Default::default());
    /// ```
    pub fn new(inner: S, store: Store, config: Config<'static>) -> Self {
        Self {
            inner,
            store,
            config,
        }
    }
}

impl<ReqBody, ResBody, S, Store> Service<Request<ReqBody>> for SessionManager<Store, S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    Store: Clone + Send + Sync + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Instrumented<ResponseFuture<S::Future>>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let span = tracing::debug_span!("session_manager");
        let _enter = span.enter();

        let session_cookie = req
            .headers()
            .get_all(COOKIE)
            .into_iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|cookie| Cookie::parse(cookie).ok())
            .find(|cookie| cookie.name() == self.config.name);

        let id = session_cookie.and_then(|cookie| {
            cookie
                .value()
                .parse::<Id>()
                .map_err(|err| {
                    tracing::warn!(
                        err = %err,
                        "possibly suspicious activity: malformed session id"
                    )
                })
                .ok()
        });

        let updater = Arc::new(Mutex::new(None));
        let session = Session {
            id,
            store: self.store.clone(),
            updater: Arc::clone(&updater),
        };
        tracing::debug!("adding session to request extensions");
        req.extensions_mut().insert(session);

        drop(_enter);
        ResponseFuture {
            inner: self.inner.call(req),
            updater,
            config: self.config,
            old_id: id,
        }
        .instrument(span)
    }
}

pin_project! {
    #[derive(Debug, Clone)]
    /// The future returned by [`SessionManager`].
    pub struct ResponseFuture<F> {
        #[pin]
        inner: F,
        updater: Updater,
        config: Config<'static>,
        old_id: Option<Id>,
    }
}

impl<F, ResBody, Error> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, Error>>,
{
    type Output = Result<Response<ResBody>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let self_ = self.project();
        let mut resp = match self_.inner.poll(cx) {
            Poll::Ready(r) => r,
            Poll::Pending => return Poll::Pending,
        }?;

        let update = self_
            .updater
            .lock()
            .expect("updater should not be poisoned")
            .or_else(|| {
                self_
                    .config
                    .always_set_expiry
                    .and_then(|expiry| self_.old_id.map(|id| SessionUpdate::Set(id, expiry)))
            });
        match update {
            Some(SessionUpdate::Delete) => {
                tracing::debug!("deleting session");
                let cookie = self_.config.build_cookie(
                    *self_.old_id,
                    Expiry::AtDateTime(
                        // The Year 2000 in UNIX time.
                        time::OffsetDateTime::from_unix_timestamp(946684800)
                            .expect("year 2000 should be in range"),
                    ),
                );
                resp.headers_mut().insert(
                    http::header::SET_COOKIE,
                    cookie
                        .to_string()
                        .try_into()
                        .expect("cookie should be valid"),
                );
            }
            Some(SessionUpdate::Set(id, expiry)) => {
                tracing::debug!("setting session {id}, expiring: {:?}", expiry);
                let cookie = self_.config.build_cookie(Some(id), expiry);
                resp.headers_mut().insert(
                    http::header::SET_COOKIE,
                    cookie
                        .to_string()
                        .try_into()
                        .expect("cookie should be valid"),
                );
            }
            None => {}
        };

        Poll::Ready(Ok(resp))
    }
}

/// A layer for providing [`Session`] as a request extension.
///
/// # Examples
///
/// ```rust
/// use tower_sesh::{MemoryStore, SessionManagerLayer};
///
/// let session_store: MemoryStore<()> = MemoryStore::default();
/// let session_service = SessionManagerLayer {
///     store: session_store,
///     config: Default::default()
/// };
/// ```
#[derive(Debug, Clone)]
pub struct SessionManagerLayer<Store> {
    /// The store to use for session data.
    ///
    /// This should implement [`tower_sesh_core::SessionStore`], and be cloneable.
    pub store: Store,
    /// The configuration options for the session cookie.
    pub config: Config<'static>,
}

impl<S, Store> Layer<S> for SessionManagerLayer<Store>
where
    Store: Clone,
{
    type Service = SessionManager<Store, S>;

    fn layer(&self, inner: S) -> Self::Service {
        SessionManager {
            inner,
            store: self.store.clone(),
            config: self.config,
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use axum::body::Body;
    use tower::{ServiceBuilder, ServiceExt};
    use tower_sesh_core::Expires;
    use tower_sesh_memory_store::MemoryStore;

    use super::*;

    #[derive(Debug, Clone)]
    struct Record {
        foo: i32,
    }
    impl Expires for Record {}

    async fn handler(mut req: Request<Body>) -> anyhow::Result<Response<Body>> {
        let session = req
            .extensions_mut()
            .remove::<Session<MemoryStore<Record>>>()
            .ok_or(anyhow!("Missing session"))?;

        let session_state = session.clone().load().await?;
        if let Some(session_state) = session_state {
            session_state
                .update(|data| {
                    data.foo += 1;
                })
                .await?;
        } else {
            session.create(Record { foo: 42 }).await?;
        }

        Ok(Response::new(Body::empty()))
    }

    async fn noop_handler(_: Request<Body>) -> anyhow::Result<Response<Body>> {
        Ok(Response::new(Body::empty()))
    }

    #[tokio::test]
    async fn basic_service_test() -> anyhow::Result<()> {
        let session_store: MemoryStore<Record> = MemoryStore::default();
        let session_layer = SessionManagerLayer {
            store: session_store,
            config: Default::default(),
        };
        let svc = ServiceBuilder::new()
            .layer(session_layer.clone())
            .service_fn(handler);

        let noop_svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(noop_handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.clone().oneshot(req).await?;

        let session = res.headers().get(http::header::SET_COOKIE);
        assert!(session.is_some());

        let req = Request::builder()
            .header(http::header::COOKIE, session.unwrap())
            .body(Body::empty())?;
        let res = noop_svc.oneshot(req).await?;

        assert!(res.headers().get(http::header::SET_COOKIE).is_none());

        Ok(())
    }

    #[tokio::test]
    async fn bogus_cookie_test() -> anyhow::Result<()> {
        let session_store: MemoryStore<Record> = MemoryStore::default();
        let session_layer = SessionManagerLayer {
            store: session_store,
            config: Default::default(),
        };
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
        let session_store: MemoryStore<Record> = MemoryStore::default();
        let session_layer = SessionManagerLayer {
            store: session_store,
            config: Default::default(),
        };
        let svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(noop_handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(res.headers().get(http::header::SET_COOKIE).is_none());

        Ok(())
    }

    #[tokio::test]
    async fn custom_config() -> anyhow::Result<()> {
        let session_store: MemoryStore<Record> = MemoryStore::default();

        let session_config = Config {
            name: "my.sid",
            http_only: false,
            same_site: SameSite::Lax,
            secure: false,
            path: "/foo/bar",
            domain: Some("example.com"),
            always_set_expiry: Some(Expiry::OnInactivity(time::Duration::hours(2))),
        };
        let session_layer = SessionManagerLayer {
            store: session_store,
            config: session_config,
        };
        let svc = ServiceBuilder::new()
            .layer(session_layer.clone())
            .service_fn(handler);
        let noop_svc = ServiceBuilder::new()
            .layer(session_layer)
            .service_fn(noop_handler);

        let req = Request::builder().body(Body::empty())?;
        let res = svc.oneshot(req).await?;

        assert!(cookie_value_matches(&res, |s| s.contains("my.sid=")));
        assert!(cookie_value_matches(&res, |s| s.contains("SameSite=Lax")));
        assert!(cookie_value_matches(&res, |s| !s.contains("Secure")));
        assert!(cookie_value_matches(&res, |s| s.contains("Path=/foo/bar")));
        assert!(cookie_value_matches(&res, |s| s.contains("Domain=example.com")));

        let req = Request::builder()
            .header(
                http::header::COOKIE,
                res.headers().get(http::header::SET_COOKIE).unwrap(),
            )
            .body(Body::empty())?;
        let res = noop_svc.oneshot(req).await?;
        assert!(cookie_has_expected_max_age(&res, 7200));

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
}
