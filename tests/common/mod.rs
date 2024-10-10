use axum::{routing::get, Router};
use axum_core::body::Body;
use http::{header, HeaderMap};
use http_body_util::BodyExt;
use time::{Duration, OffsetDateTime};
use tower_sessions::{Expires, Expiry, Session, SessionManagerLayer, SessionStore};


#[derive(Debug, Clone, Copy)]
pub struct Foo(pub u32);

impl Expires for Foo {
    fn expires(&self) -> Expiry {
        Expiry::OnInactivity(Duration::hours(1))
    }
}

fn routes<Store>() -> Router where
Store: SessionStore<Foo> + Clone + 'static,
Store::Error: std::fmt::Debug,
{
    
    Router::new()
        .route("/", get(|_: Session<Store>| async move { "Hello, world!" }))
        .route(
            "/create",
            get(|session: Session<Store>| async move {
                session.create(Foo(42)).await.unwrap();
            }),
        )
        .route(
            "/get",
            get(|session: Session<Store>| async move {
                format!("{:?}", session.load().await.unwrap().unwrap().data())
            }),
        )
        .route(
            "/remove",
            get(|session: Session<Store>| async move {
                let state = session.load().await.unwrap().unwrap();
                println!("{}", state.delete().await.unwrap());
            }),
        )
        .route(
            "/cycle_id",
            get(|session: Session<Store>| async move {
                let state = session.load().await.unwrap().unwrap();
                state.cycle().await.unwrap();
            }),
        )
        .route(
            "/set_expiry",
            get(|session: Session<Store>| async move {
                let expiry = Expiry::AtDateTime(OffsetDateTime::now_utc() + Duration::days(1));
                session.load().await.unwrap().unwrap().update_with_expiry(|_| {}, expiry).await.unwrap();
            }),
        )
}

// pub fn build_app<Store: SessionStore + Clone>(
//     mut session_manager: SessionManagerLayer<Store>,
//     max_age: Option<Duration>,
//     domain: Option<String>,
// ) -> Router {
//     if let Some(max_age) = max_age {
//         session_manager = session_manager.with_expiry(Expiry::OnInactivity(max_age));
//     }
// 
//     if let Some(domain) = domain {
//         session_manager = session_manager.with_domain(domain);
//     }
// 
//     routes().layer(session_manager)
// }
// 
// pub async fn body_string(body: Body) -> String {
//     let bytes = body.collect().await.unwrap().to_bytes();
//     String::from_utf8_lossy(&bytes).into()
// }
// 
// pub fn get_session_cookie(headers: &HeaderMap) -> Result<Cookie<'_>, cookie::ParseError> {
//     headers
//         .get_all(header::SET_COOKIE)
//         .iter()
//         .flat_map(|header| header.to_str())
//         .next()
//         .ok_or(cookie::ParseError::MissingPair)
//         .and_then(Cookie::parse_encoded)
// }
// 
// #[macro_export]
// macro_rules! route_tests {
//     ($create_app:expr) => {
//         use axum::body::Body;
//         use http::{header, Request, StatusCode};
//         use time::Duration;
//         use tower::ServiceExt;
//         use tower_cookies::{cookie::SameSite, Cookie};
//         use $crate::common::{body_string, get_session_cookie};
// 
//         #[tokio::test]
//         async fn no_session_set() {
//             let req = Request::builder().uri("/").body(Body::empty()).unwrap();
//             let res = $create_app(Some(Duration::hours(1)), None)
//                 .await
//                 .oneshot(req)
//                 .await
//                 .unwrap();
// 
//             assert!(res
//                 .headers()
//                 .get_all(header::SET_COOKIE)
//                 .iter()
//                 .next()
//                 .is_none());
//         }
// 
//         #[tokio::test]
//         async fn bogus_session_cookie() {
//             let session_cookie = Cookie::new("id", "AAAAAAAAAAAAAAAAAAAAAA");
//             let req = Request::builder()
//                 .uri("/insert")
//                 .header(header::COOKIE, session_cookie.encoded().to_string())
//                 .body(Body::empty())
//                 .unwrap();
//             let res = $create_app(Some(Duration::hours(1)), None)
//                 .await
//                 .oneshot(req)
//                 .await
//                 .unwrap();
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             assert_eq!(res.status(), StatusCode::OK);
//             assert_ne!(session_cookie.value(), "AAAAAAAAAAAAAAAAAAAAAA");
//         }
// 
//         #[tokio::test]
//         async fn malformed_session_cookie() {
//             let session_cookie = Cookie::new("id", "malformed");
//             let req = Request::builder()
//                 .uri("/")
//                 .header(header::COOKIE, session_cookie.encoded().to_string())
//                 .body(Body::empty())
//                 .unwrap();
//             let res = $create_app(Some(Duration::hours(1)), None)
//                 .await
//                 .oneshot(req)
//                 .await
//                 .unwrap();
// 
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
//             assert_ne!(session_cookie.value(), "malformed");
//             assert_eq!(res.status(), StatusCode::OK);
//         }
// 
//         #[tokio::test]
//         async fn insert_session() {
//             let req = Request::builder()
//                 .uri("/insert")
//                 .body(Body::empty())
//                 .unwrap();
//             let res = $create_app(Some(Duration::hours(1)), None)
//                 .await
//                 .oneshot(req)
//                 .await
//                 .unwrap();
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             assert_eq!(session_cookie.name(), "id");
//             assert_eq!(session_cookie.http_only(), Some(true));
//             assert_eq!(session_cookie.same_site(), Some(SameSite::Strict));
//             assert!(session_cookie
//                 .max_age()
//                 .is_some_and(|dt| dt <= Duration::hours(1)));
//             assert_eq!(session_cookie.secure(), Some(true));
//             assert_eq!(session_cookie.path(), Some("/"));
//         }
// 
//         #[tokio::test]
//         async fn session_max_age() {
//             let req = Request::builder()
//                 .uri("/insert")
//                 .body(Body::empty())
//                 .unwrap();
//             let res = $create_app(None, None).await.oneshot(req).await.unwrap();
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             assert_eq!(session_cookie.name(), "id");
//             assert_eq!(session_cookie.http_only(), Some(true));
//             assert_eq!(session_cookie.same_site(), Some(SameSite::Strict));
//             assert!(session_cookie.max_age().is_none());
//             assert_eq!(session_cookie.secure(), Some(true));
//             assert_eq!(session_cookie.path(), Some("/"));
//         }
// 
//         #[tokio::test]
//         async fn get_session() {
//             let app = $create_app(Some(Duration::hours(1)), None).await;
// 
//             let req = Request::builder()
//                 .uri("/insert")
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.clone().oneshot(req).await.unwrap();
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             let req = Request::builder()
//                 .uri("/get")
//                 .header(header::COOKIE, session_cookie.encoded().to_string())
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.oneshot(req).await.unwrap();
//             assert_eq!(res.status(), StatusCode::OK);
// 
//             assert_eq!(body_string(res.into_body()).await, "42");
//         }
// 
//         #[tokio::test]
//         async fn get_no_value() {
//             let app = $create_app(Some(Duration::hours(1)), None).await;
// 
//             let req = Request::builder()
//                 .uri("/get_value")
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.oneshot(req).await.unwrap();
// 
//             assert_eq!(body_string(res.into_body()).await, "None");
//         }
// 
//         #[tokio::test]
//         async fn remove_last_value() {
//             let app = $create_app(Some(Duration::hours(1)), None).await;
// 
//             let req = Request::builder()
//                 .uri("/insert")
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.clone().oneshot(req).await.unwrap();
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             let req = Request::builder()
//                 .uri("/remove_value")
//                 .header(header::COOKIE, session_cookie.encoded().to_string())
//                 .body(Body::empty())
//                 .unwrap();
//             app.clone().oneshot(req).await.unwrap();
// 
//             let req = Request::builder()
//                 .uri("/get_value")
//                 .header(header::COOKIE, session_cookie.encoded().to_string())
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.oneshot(req).await.unwrap();
// 
//             assert_eq!(body_string(res.into_body()).await, "None");
//         }
// 
//         #[tokio::test]
//         async fn cycle_session_id() {
//             let app = $create_app(Some(Duration::hours(1)), None).await;
// 
//             let req = Request::builder()
//                 .uri("/insert")
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.clone().oneshot(req).await.unwrap();
//             let first_session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             let req = Request::builder()
//                 .uri("/cycle_id")
//                 .header(header::COOKIE, first_session_cookie.encoded().to_string())
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.clone().oneshot(req).await.unwrap();
//             let second_session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             let req = Request::builder()
//                 .uri("/get")
//                 .header(header::COOKIE, second_session_cookie.encoded().to_string())
//                 .body(Body::empty())
//                 .unwrap();
//             let res = dbg!(app.oneshot(req).await).unwrap();
// 
//             assert_ne!(first_session_cookie.value(), second_session_cookie.value());
//             assert_eq!(body_string(res.into_body()).await, "42");
//         }
// 
//         #[tokio::test]
//         async fn flush_session() {
//             let app = $create_app(Some(Duration::hours(1)), None).await;
// 
//             let req = Request::builder()
//                 .uri("/insert")
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.clone().oneshot(req).await.unwrap();
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             let req = Request::builder()
//                 .uri("/flush")
//                 .header(header::COOKIE, session_cookie.encoded().to_string())
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.oneshot(req).await.unwrap();
// 
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             assert_eq!(session_cookie.value(), "");
//             assert_eq!(session_cookie.max_age(), Some(Duration::ZERO));
//             assert_eq!(session_cookie.path(), Some("/"));
//         }
// 
//         #[tokio::test]
//         async fn flush_with_domain() {
//             let app = $create_app(Some(Duration::hours(1)), Some("localhost".to_string())).await;
// 
//             let req = Request::builder()
//                 .uri("/insert")
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.clone().oneshot(req).await.unwrap();
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             let req = Request::builder()
//                 .uri("/flush")
//                 .header(header::COOKIE, session_cookie.encoded().to_string())
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.oneshot(req).await.unwrap();
// 
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             assert_eq!(session_cookie.value(), "");
//             assert_eq!(session_cookie.max_age(), Some(Duration::ZERO));
//             assert_eq!(session_cookie.domain(), Some("localhost"));
//             assert_eq!(session_cookie.path(), Some("/"));
//         }
// 
//         #[tokio::test]
//         async fn set_expiry() {
//             let app = $create_app(Some(Duration::hours(1)), Some("localhost".to_string())).await;
// 
//             let req = Request::builder()
//                 .uri("/insert")
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.clone().oneshot(req).await.unwrap();
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             let expected_duration = Duration::hours(1);
//             let actual_duration = session_cookie.max_age().unwrap();
//             let tolerance = Duration::seconds(1);
// 
//             assert!(
//                 actual_duration >= expected_duration - tolerance
//                     && actual_duration <= expected_duration + tolerance,
//                 "Duration is not within the acceptable range: {:?}",
//                 actual_duration
//             );
// 
//             let req = Request::builder()
//                 .uri("/set_expiry")
//                 .header(header::COOKIE, session_cookie.encoded().to_string())
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.oneshot(req).await.unwrap();
// 
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             let expected_duration = Duration::days(1);
//             let actual_duration = session_cookie.max_age().unwrap();
//             let tolerance = Duration::seconds(1);
// 
//             assert!(
//                 actual_duration >= expected_duration - tolerance
//                     && actual_duration <= expected_duration + tolerance,
//                 "Duration is not within the acceptable range: {:?}",
//                 actual_duration
//             );
//         }
// 
//         #[tokio::test]
//         async fn change_expiry_type() {
//             let app = $create_app(None, Some("localhost".to_string())).await;
// 
//             let req = Request::builder()
//                 .uri("/insert")
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.clone().oneshot(req).await.unwrap();
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             let expected_duration = None;
//             let actual_duration = session_cookie.max_age();
// 
//             assert_eq!(actual_duration, expected_duration, "Duration is not None");
// 
//             let req = Request::builder()
//                 .uri("/set_expiry")
//                 .header(header::COOKIE, session_cookie.encoded().to_string())
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app.oneshot(req).await.unwrap();
// 
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             let expected_duration = Duration::days(1);
//             assert!(session_cookie.max_age().is_some(), "Duration is None");
//             let actual_duration = session_cookie.max_age().unwrap();
//             let tolerance = Duration::seconds(1);
// 
//             assert!(
//                 actual_duration >= expected_duration - tolerance
//                     && actual_duration <= expected_duration + tolerance,
//                 "Duration is not within the acceptable range: {:?}",
//                 actual_duration
//             );
// 
//             let app2 = $create_app(Some(Duration::hours(1)), Some("localhost".to_string())).await;
// 
//             let req = Request::builder()
//                 .uri("/insert")
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app2.clone().oneshot(req).await.unwrap();
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             let expected_duration = Duration::hours(1);
//             let actual_duration = session_cookie.max_age().unwrap();
//             let tolerance = Duration::seconds(1);
// 
//             assert!(
//                 actual_duration >= expected_duration - tolerance
//                     && actual_duration <= expected_duration + tolerance,
//                 "Duration is not within the acceptable range: {:?}",
//                 actual_duration
//             );
// 
//             let req = Request::builder()
//                 .uri("/remove_expiry")
//                 .header(header::COOKIE, session_cookie.encoded().to_string())
//                 .body(Body::empty())
//                 .unwrap();
//             let res = app2.oneshot(req).await.unwrap();
// 
//             let session_cookie = get_session_cookie(res.headers()).unwrap();
// 
//             let expected_duration = None;
//             let actual_duration = session_cookie.max_age();
// 
//             assert_eq!(actual_duration, expected_duration, "Duration is not None");
//         }
//     };
// }
