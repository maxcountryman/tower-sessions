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
use tower_sessions_core::{
    expires::Expiry,
    id::Id,
};

use crate::{
    session::{SessionUpdate, Updater}, Session
};

#[derive(Debug, Copy, Clone)]
/// the configuration options for the [`SessionManagerLayer`].
///
/// ## Default
/// ```
/// # use tower_sessions::SessionConfig;
/// # use tokwer_sessions::expires::Expiry;
/// # use cookie::SameSite;
/// let default = SessionConfig {
///    name: "id",
///    http_only: true,
///    same_site: SameSite::Strict,
///    expiry: Expiry::OnSessionEnd,
///    secure: true,
///    path: "/",
///    domain: None,
///    always_save: false,
/// };
///
/// assert_eq!(default, SessionConfig::default());
/// ```
pub struct SessionConfig<'a> {
    /// The name of the cookie.
    pub name: &'a str,
    /// Whether the cookie is [HTTP only](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie#httponly).
    pub http_only: bool,
    /// The
    /// [SameSite](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie#samesitesamesite-value)
    /// policy.
    pub same_site: SameSite,
    /// When the cookie should expire.
    ///
    /// This manages the
    /// [`Max-Age`](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie#max-agenumber)
    /// and the
    /// [`Expires`](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie#expiresdate)
    /// attributes.
    pub expiry: Expiry,
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
    /// Whether the session should always be saved once extracted, even if its value did not
    /// change.
    pub always_save: bool,
}

impl<'a> SessionConfig<'a> {
    fn build_cookie(self, session_id: Id, expiry: Expiry) -> Cookie<'a> {
        let mut cookie_builder = Cookie::build((self.name, session_id.to_string()))
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

impl Default for SessionConfig<'static> {
    fn default() -> Self {
        Self {
            name: "id", /* See: https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html#session-id-name-fingerprinting */
            http_only: true,
            same_site: SameSite::Strict,
            expiry: Expiry::OnSessionEnd, // TODO: Is `Max-Age: "Session"` the right default?
            secure: true,
            path: "/",
            domain: None,
            always_save: false,
        }
    }
}

/// A middleware that provides [`Session`] as a request extension.
#[derive(Debug, Clone)]
pub struct SessionManager<Store, S> {
    inner: S,
    store: Store,
    config: SessionConfig<'static>,
}

impl<Store, S> SessionManager<Store, S> {
    /// Create a new [`SessionManager`].
    pub fn new(inner: S, store: Store, config: SessionConfig<'static>) -> Self {
        Self {
            inner,
            store,
            config,
        }
    }
}

impl<ReqBody, ResBody, S, Store> Service<Request<ReqBody>>
    for SessionManager<Store, S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send,
    ReqBody: Send + 'static,
    ResBody: Default + Send,
    Store: Clone + Send + Sync + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
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
        req.extensions_mut().insert(session);

        ResponseFuture {
            inner: self.inner.call(req),
            updater,
            config: self.config,
            old_id: id,
        }
    }
}

pin_project! {
    #[derive(Debug, Clone)]
    /// The future returned by [`SessionManager`].
    pub struct ResponseFuture<F> {
        #[pin]
        inner: F,
        updater: Updater,
        config: SessionConfig<'static>,
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
                if self_.config.always_save {
                    self_.old_id
                        .map(|id| SessionUpdate::Set(id, self_.config.expiry))
                } else {
                    None
                }
            });
        match update {
            Some(SessionUpdate::Delete) => {
                if let Some(old_id) = self_.old_id {
                    let cookie = self_.config.build_cookie(
                        *old_id,
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
                };
            }
            Some(SessionUpdate::Set(id, expiry)) => {
                let cookie = self_.config.build_cookie(id, expiry);
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
#[derive(Debug, Clone)]
pub struct SessionManagerLayer<Store> {
    store: Store,
    config: SessionConfig<'static>,
}

impl<Store> SessionManagerLayer<Store> {
    /// Create a new [`SessionManagerLayer`] with the provided session store
    /// and configuration.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_service = SessionManagerLayer::new(session_store);
    /// ```
    pub fn new(store: Store, config: SessionConfig<'static>) -> Self {
        Self {
            store,
            config,
        }
    }
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

// #[cfg(test)]
// mod tests {
//     use std::str::FromStr;
//
//     use anyhow::anyhow;
//     use axum::body::Body;
//     use tower::{ServiceBuilder, ServiceExt};
//     use tower_sessions_memory_store::MemoryStore;
//
//     use crate::session::{Id, Record};
//
//     use super::*;
//
//     async fn handler(req: Request<Body>) -> anyhow::Result<Response<Body>> {
//         let session = req
//             .extensions()
//             .get::<LazySession>()
//             .ok_or(anyhow!("Missing session"))?;
//
//         session.insert("foo", 42).await?;
//
//         Ok(Response::new(Body::empty()))
//     }
//
//     async fn noop_handler(_: Request<Body>) -> anyhow::Result<Response<Body>> {
//         Ok(Response::new(Body::empty()))
//     }
//
//     #[tokio::test]
//     async fn basic_service_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.clone().oneshot(req).await?;
//
//         let session = res.headers().get(http::header::SET_COOKIE);
//         assert!(session.is_some());
//
//         let req = Request::builder()
//             .header(http::header::COOKIE, session.unwrap())
//             .body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(res.headers().get(http::header::SET_COOKIE).is_none());
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn bogus_cookie_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.clone().oneshot(req).await?;
//
//         assert!(res.headers().get(http::header::SET_COOKIE).is_some());
//
//         let req = Request::builder()
//             .header(http::header::COOKIE, "id=bogus")
//             .body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(res.headers().get(http::header::SET_COOKIE).is_some());
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn no_set_cookie_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(noop_handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(res.headers().get(http::header::SET_COOKIE).is_none());
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn name_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store).with_name("my.sid");
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(cookie_value_matches(&res, |s| s.starts_with("my.sid=")));
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn http_only_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(cookie_value_matches(&res, |s| s.contains("HttpOnly")));
//
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store).with_http_only(false);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(cookie_value_matches(&res, |s| !s.contains("HttpOnly")));
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn same_site_strict_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer =
//             SessionManagerLayer::new(session_store).with_same_site(SameSite::Strict);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(cookie_value_matches(&res, |s| s.contains("SameSite=Strict")));
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn same_site_lax_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store).with_same_site(SameSite::Lax);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(cookie_value_matches(&res, |s| s.contains("SameSite=Lax")));
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn same_site_none_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store).with_same_site(SameSite::None);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(cookie_value_matches(&res, |s| s.contains("SameSite=None")));
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn expiry_on_session_end_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer =
//             SessionManagerLayer::new(session_store).with_expiry(Expiry::OnSessionEnd);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(cookie_value_matches(&res, |s| !s.contains("Max-Age")));
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn expiry_on_inactivity_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let inactivity_duration = time::Duration::hours(2);
//         let session_layer = SessionManagerLayer::new(session_store)
//             .with_expiry(Expiry::OnInactivity(inactivity_duration));
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         let expected_max_age = inactivity_duration.whole_seconds();
//         assert!(cookie_has_expected_max_age(&res, expected_max_age));
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn expiry_at_date_time_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let expiry_time = time::OffsetDateTime::now_utc() + time::Duration::weeks(1);
//         let session_layer =
//             SessionManagerLayer::new(session_store).with_expiry(Expiry::AtDateTime(expiry_time));
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         let expected_max_age = (expiry_time - time::OffsetDateTime::now_utc()).whole_seconds();
//         assert!(cookie_has_expected_max_age(&res, expected_max_age));
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn expiry_on_session_end_always_save_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store.clone())
//             .with_expiry(Expiry::OnSessionEnd)
//             .with_always_save(true);
//         let mut svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req1 = Request::builder().body(Body::empty())?;
//         let res1 = svc.call(req1).await?;
//         let sid1 = get_session_id(&res1);
//         let rec1 = get_record(&session_store, &sid1).await;
//         let req2 = Request::builder()
//             .header(http::header::COOKIE, &format!("id={}", sid1))
//             .body(Body::empty())?;
//         let res2 = svc.call(req2).await?;
//         let sid2 = get_session_id(&res2);
//         let rec2 = get_record(&session_store, &sid2).await;
//
//         assert!(cookie_value_matches(&res2, |s| !s.contains("Max-Age")));
//         assert!(sid1 == sid2);
//         assert!(rec1.expiry_date < rec2.expiry_date);
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn expiry_on_inactivity_always_save_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let inactivity_duration = time::Duration::hours(2);
//         let session_layer = SessionManagerLayer::new(session_store.clone())
//             .with_expiry(Expiry::OnInactivity(inactivity_duration))
//             .with_always_save(true);
//         let mut svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req1 = Request::builder().body(Body::empty())?;
//         let res1 = svc.call(req1).await?;
//         let sid1 = get_session_id(&res1);
//         let rec1 = get_record(&session_store, &sid1).await;
//         let req2 = Request::builder()
//             .header(http::header::COOKIE, &format!("id={}", sid1))
//             .body(Body::empty())?;
//         let res2 = svc.call(req2).await?;
//         let sid2 = get_session_id(&res2);
//         let rec2 = get_record(&session_store, &sid2).await;
//
//         let expected_max_age = inactivity_duration.whole_seconds();
//         assert!(cookie_has_expected_max_age(&res2, expected_max_age));
//         assert!(sid1 == sid2);
//         assert!(rec1.expiry_date < rec2.expiry_date);
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn expiry_at_date_time_always_save_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let expiry_time = time::OffsetDateTime::now_utc() + time::Duration::weeks(1);
//         let session_layer = SessionManagerLayer::new(session_store.clone())
//             .with_expiry(Expiry::AtDateTime(expiry_time))
//             .with_always_save(true);
//         let mut svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req1 = Request::builder().body(Body::empty())?;
//         let res1 = svc.call(req1).await?;
//         let sid1 = get_session_id(&res1);
//         let rec1 = get_record(&session_store, &sid1).await;
//         let req2 = Request::builder()
//             .header(http::header::COOKIE, &format!("id={}", sid1))
//             .body(Body::empty())?;
//         let res2 = svc.call(req2).await?;
//         let sid2 = get_session_id(&res2);
//         let rec2 = get_record(&session_store, &sid2).await;
//
//         let expected_max_age = (expiry_time - time::OffsetDateTime::now_utc()).whole_seconds();
//         assert!(cookie_has_expected_max_age(&res2, expected_max_age));
//         assert!(sid1 == sid2);
//         assert!(rec1.expiry_date == rec2.expiry_date);
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn secure_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store).with_secure(true);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(cookie_value_matches(&res, |s| s.contains("Secure")));
//
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store).with_secure(false);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(cookie_value_matches(&res, |s| !s.contains("Secure")));
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn path_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store).with_path("/foo/bar");
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(cookie_value_matches(&res, |s| s.contains("Path=/foo/bar")));
//
//         Ok(())
//     }
//
//     #[tokio::test]
//     async fn domain_test() -> anyhow::Result<()> {
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store).with_domain("example.com");
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(cookie_value_matches(&res, |s| s.contains("Domain=example.com")));
//
//         Ok(())
//     }
//
//     #[cfg(feature = "signed")]
//     #[tokio::test]
//     async fn signed_test() -> anyhow::Result<()> {
//         let key = Key::generate();
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store).with_signed(key);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(res.headers().get(http::header::SET_COOKIE).is_some());
//
//         Ok(())
//     }
//
//     #[cfg(feature = "private")]
//     #[tokio::test]
//     async fn private_test() -> anyhow::Result<()> {
//         let key = Key::generate();
//         let session_store = MemoryStore::default();
//         let session_layer = SessionManagerLayer::new(session_store).with_private(key);
//         let svc = ServiceBuilder::new()
//             .layer(session_layer)
//             .service_fn(handler);
//
//         let req = Request::builder().body(Body::empty())?;
//         let res = svc.oneshot(req).await?;
//
//         assert!(res.headers().get(http::header::SET_COOKIE).is_some());
//
//         Ok(())
//     }
//
//     fn cookie_value_matches<F>(res: &Response<Body>, matcher: F) -> bool
//     where
//         F: FnOnce(&str) -> bool,
//     {
//         res.headers()
//             .get(http::header::SET_COOKIE)
//             .is_some_and(|set_cookie| set_cookie.to_str().is_ok_and(matcher))
//     }
//
//     fn cookie_has_expected_max_age(res: &Response<Body>, expected_value: i64) -> bool {
//         res.headers()
//             .get(http::header::SET_COOKIE)
//             .is_some_and(|set_cookie| {
//                 set_cookie.to_str().is_ok_and(|s| {
//                     let max_age_value = s
//                         .split("Max-Age=")
//                         .nth(1)
//                         .unwrap_or_default()
//                         .split(';')
//                         .next()
//                         .unwrap_or_default()
//                         .parse::<i64>()
//                         .unwrap_or_default();
//                     (max_age_value - expected_value).abs() <= 1
//                 })
//             })
//     }
//
//     fn get_session_id(res: &Response<Body>) -> String {
//         res.headers()
//             .get(http::header::SET_COOKIE)
//             .unwrap()
//             .to_str()
//             .unwrap()
//             .split("id=")
//             .nth(1)
//             .unwrap()
//             .split(";")
//             .next()
//             .unwrap()
//             .to_string()
//     }
//
//     async fn get_record(store: &impl SessionStore, id: &str) -> Record {
//         store
//             .load(&Id::from_str(id).unwrap())
//             .await
//             .unwrap()
//             .unwrap()
//     }
// }
