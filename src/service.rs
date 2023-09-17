//! A middleware that provides [`Session`] as a request extension.
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use http::{Request, Response};
use tower_cookies::{
    cookie::{time::Duration, SameSite},
    CookieManager, Cookies,
};
use tower_layer::Layer;
use tower_service::Service;

use crate::{
    session::{SessionDeletion, SessionId},
    CookieConfig, Session, SessionStore,
};

/// A middleware that provides [`Session`] as a request extension.
#[derive(Debug, Clone)]
pub struct SessionManager<S, Store: SessionStore> {
    inner: S,
    session_store: Store,
    cookie_config: CookieConfig,
}

impl<S, Store: SessionStore> SessionManager<S, Store> {
    /// Create a new [`SessionManager`].
    pub fn new(inner: S, session_store: Store, cookie_config: CookieConfig) -> Self {
        Self {
            inner,
            session_store,
            cookie_config,
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

        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        Box::pin(async move {
            let cookies = req
                .extensions()
                .get::<Cookies>()
                .cloned()
                .expect("Something has gone wrong with tower-cookies.");

            let mut session = if let Some(session_cookie) = cookies.get(&cookie_config.name) {
                // We do have a session cookie, so let's see if our store has the associated
                // session.
                //
                // N.B.: Our store will *not* have the session if we've not put data in it yet.
                let session_id = session_cookie.value().try_into()?;
                session_store.load(&session_id).await?
            } else {
                // We don't have a session cookie, so let's create a new session.
                Some((&cookie_config).into())
            }
            .filter(Session::active)
            .unwrap_or_else(|| {
                // The session was expired so we create a new one here.
                (&cookie_config).into()
            });

            req.extensions_mut().insert(session.clone());


            let res = Ok(inner.call(req).await.map_err(Into::into)?);

            if let Some(session_deletion) = session.deleted() {
                match session_deletion {
                    SessionDeletion::Deleted => {
                        session_store.delete(&session.id()).await?;
                        cookies.remove(session.build_cookie(&cookie_config));

                        // Since the session has been deleted, there's no need for further
                        // processing.
                        return res;
                    }

                    SessionDeletion::Cycled(deleted_id) => {
                        session_store.delete(&deleted_id).await?;
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
                session_store.save(&session_record).await?;
                cookies.add(session.build_cookie(&cookie_config))
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
    pub fn with_name(mut self, name: &str) -> Self {
        self.cookie_config.name = name.to_string();
        self
    }

    /// Configures the `"SameSite"` attribute of the cookie used for the
    /// session.
    pub fn with_same_site(mut self, same_site: SameSite) -> Self {
        self.cookie_config.same_site = same_site;
        self
    }

    /// Configures the `"Max-Age"` attribute of the cookie used for the session.
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.cookie_config.max_age = Some(max_age);
        self
    }

    /// Configures the `"Secure"` attribute of the cookie used for the session.
    pub fn with_secure(mut self, secure: bool) -> Self {
        self.cookie_config.secure = secure;
        self
    }

    /// Configures the `"Path"` attribute of the cookie used for the session.
    pub fn with_path(mut self, path: String) -> Self {
        self.cookie_config.path = path;
        self
    }

    /// Configures the `"Domain"` attribute of the cookie used for the session.
    pub fn with_domain(mut self, domain: String) -> Self {
        self.cookie_config.domain = Some(domain);
        self
    }
}

impl<Store: SessionStore> SessionManagerLayer<Store> {
    /// Create a new [`SessionManagerLayer`] with the provided session store
    /// and default cookie configuration.
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
        };

        CookieManager::new(session_manager)
    }
}

#[cfg(all(test, feature = "axum-core", feature = "memory-store"))]
mod tests {
    use axum::{body::Body, error_handling::HandleErrorLayer, routing::get, Router};
    use axum_core::{body::BoxBody, BoxError};
    use http::{header, HeaderMap, Request, StatusCode};
    use time::Duration;
    use tower::{ServiceBuilder, ServiceExt};
    use tower_cookies::{
        cookie::{self, SameSite},
        Cookie,
    };

    use crate::{MemoryStore, Session, SessionManagerLayer};

    fn app(max_age: Option<Duration>) -> Router {
        let session_store = MemoryStore::default();
        let mut session_manager = SessionManagerLayer::new(session_store).with_secure(true);
        if let Some(max_age) = max_age {
            session_manager = session_manager.with_max_age(max_age);
        }
        let session_service = ServiceBuilder::new()
            .layer(HandleErrorLayer::new(|_: BoxError| async {
                StatusCode::BAD_REQUEST
            }))
            .layer(session_manager);
        Router::new()
            .route("/", get(|_: Session| async move { "Hello, world!" }))
            .route(
                "/insert",
                get(|session: Session| async move {
                    session.insert("foo", 42).unwrap();
                }),
            )
            .route(
                "/get",
                get(|session: Session| async move {
                    format!("{}", session.get::<usize>("foo").unwrap().unwrap())
                }),
            )
            .route(
                "/remove",
                get(|session: Session| async move {
                    session.remove::<usize>("foo").unwrap();
                }),
            )
            .route(
                "/cycle_id",
                get(|session: Session| async move {
                    session.cycle_id();
                }),
            )
            .route(
                "/delete",
                get(|session: Session| async move {
                    session.delete();
                }),
            )
            .layer(session_service)
    }

    async fn body_string(body: BoxBody) -> String {
        let bytes = hyper::body::to_bytes(body).await.unwrap();
        String::from_utf8_lossy(&bytes).into()
    }

    fn get_session_cookie(headers: &HeaderMap) -> Result<Cookie<'_>, cookie::ParseError> {
        headers
            .get_all(header::SET_COOKIE)
            .iter()
            .flat_map(|header| header.to_str())
            .next()
            .ok_or(cookie::ParseError::MissingPair)
            .and_then(Cookie::parse_encoded)
    }

    #[tokio::test]
    async fn no_session_set() {
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let res = app(Some(Duration::hours(1))).oneshot(req).await.unwrap();

        assert!(res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .next()
            .is_none());
    }

    #[tokio::test]
    async fn bogus_session_cookie() {
        let session_cookie = Cookie::new("tower.sid", "00000000-0000-0000-0000-000000000000");
        let req = Request::builder()
            .uri("/insert")
            .header(header::COOKIE, session_cookie.encoded().to_string())
            .body(Body::empty())
            .unwrap();
        let res = app(Some(Duration::hours(1))).oneshot(req).await.unwrap();
        let session_cookie = get_session_cookie(dbg!(res.headers())).unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_ne!(
            session_cookie.value(),
            "00000000-0000-0000-0000-000000000000"
        );
    }

    #[tokio::test]
    async fn malformed_session_cookie() {
        let session_cookie = Cookie::new("tower.sid", "malformed");
        let req = Request::builder()
            .uri("/")
            .header(header::COOKIE, session_cookie.encoded().to_string())
            .body(Body::empty())
            .unwrap();
        let res = app(Some(Duration::hours(1))).oneshot(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn insert_session() {
        let req = Request::builder()
            .uri("/insert")
            .body(Body::empty())
            .unwrap();
        let res = app(Some(Duration::hours(1))).oneshot(req).await.unwrap();
        let session_cookie = get_session_cookie(res.headers()).unwrap();

        assert_eq!(session_cookie.name(), "tower.sid");
        assert_eq!(session_cookie.http_only(), Some(true));
        assert_eq!(session_cookie.same_site(), Some(SameSite::Strict));
        assert!(session_cookie
            .max_age()
            .is_some_and(|dt| dt <= Duration::hours(1)));
        assert_eq!(session_cookie.secure(), Some(true));
        assert_eq!(session_cookie.path(), Some("/"));
    }

    #[tokio::test]
    async fn session_expiration() {
        let req = Request::builder()
            .uri("/insert")
            .body(Body::empty())
            .unwrap();
        let res = app(None).oneshot(req).await.unwrap();
        let session_cookie = get_session_cookie(res.headers()).unwrap();

        assert_eq!(session_cookie.name(), "tower.sid");
        assert_eq!(session_cookie.http_only(), Some(true));
        assert_eq!(session_cookie.same_site(), Some(SameSite::Strict));
        assert!(session_cookie.max_age().is_none());
        assert_eq!(session_cookie.secure(), Some(true));
        assert_eq!(session_cookie.path(), Some("/"));
    }

    #[tokio::test]
    async fn get_session() {
        let app = app(Some(Duration::hours(1)));

        let req = Request::builder()
            .uri("/insert")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let session_cookie = get_session_cookie(res.headers()).unwrap();

        let req = Request::builder()
            .uri("/get")
            .header(header::COOKIE, session_cookie.encoded().to_string())
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();

        assert_eq!(body_string(res.into_body()).await, "42");
    }

    #[tokio::test]
    async fn cycle_session_id() {
        let app = app(Some(Duration::hours(1)));

        let req = Request::builder()
            .uri("/insert")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let first_session_cookie = get_session_cookie(res.headers()).unwrap();

        let req = Request::builder()
            .uri("/cycle_id")
            .header(header::COOKIE, first_session_cookie.encoded().to_string())
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let second_session_cookie = get_session_cookie(res.headers()).unwrap();

        let req = Request::builder()
            .uri("/get")
            .header(header::COOKIE, second_session_cookie.encoded().to_string())
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();

        assert_ne!(first_session_cookie.value(), second_session_cookie.value());
        assert_eq!(body_string(res.into_body()).await, "42");
    }

    #[tokio::test]
    async fn delete_session() {
        let app = app(Some(Duration::hours(1)));

        let req = Request::builder()
            .uri("/insert")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let session_cookie = get_session_cookie(res.headers()).unwrap();

        let req = Request::builder()
            .uri("/delete")
            .header(header::COOKIE, session_cookie.encoded().to_string())
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();

        let session_cookie = get_session_cookie(res.headers()).unwrap();

        assert_eq!(session_cookie.value(), "");
        assert_eq!(session_cookie.max_age(), Some(Duration::ZERO));
    }
}
