//! Defines the configuration for the cookie belonging to the session.
use time::{Duration, OffsetDateTime};
use tower_cookies::{cookie::SameSite, Cookie};

use crate::Session;

/// Defines the configuration for the cookie belonging to the session.
#[derive(Debug, Clone)]
pub struct CookieConfig {
    /// The name of the cookie.
    pub name: String,

    /// Specifies the SameSite attribute of the cookie.
    ///
    /// The SameSite attribute restricts when cookies are sent to the server,
    /// helping to protect against certain types of cross-site request
    /// forgery (CSRF) attacks.
    ///
    /// - `SameSite::Strict`: The cookie is only sent when making a "same-site"
    ///   request, which includes requests originating from the same site as the
    ///   cookie.
    /// - `SameSite::Lax`: The cookie is sent on "same-site" requests as well as
    ///   during safe "cross-site" top-level navigations (e.g., clicking a
    ///   link).
    /// - `SameSite::None`: The cookie is sent on all requests, regardless of
    ///   origin.
    pub same_site: SameSite,

    /// Specifies the maximum age of the cookie.
    ///
    /// This field represents the duration for which the cookie is considered
    /// valid before it expires. If set to `None`, the cookie will be
    /// treated as a session cookie and will expire when the browser is
    /// closed.
    pub max_age: Option<Duration>,

    /// Indicates whether the cookie should only be transmitted over secure
    /// (HTTPS) connections.
    ///
    /// If `true`, the cookie will only be sent to the server over secure
    /// connections. If `false`, the cookie may be sent over both secure and
    /// non-secure connections.
    pub secure: bool,

    /// Specifies the path for which the cookie is valid.
    ///
    /// The cookie will only be sent to the server when the request path matches
    /// or is a subpath of the `path` specified here.
    pub path: String,

    /// Specifies the domain for which the cookie is valid.
    ///
    /// If `Some`, the cookie is only sent to the server when the request domain
    /// matches the specified domain. If `None`, the cookie is valid for the
    /// current domain.
    pub domain: Option<String>,
}

impl CookieConfig {
    /// Create a `Cookie` from the provided `Session`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{CookieConfig, Session};
    /// let session = Session::default();
    /// let cookie_config = CookieConfig::default();
    /// let cookie = cookie_config.build_cookie(&session);
    /// assert_eq!(cookie.value(), session.id().to_string());
    /// ```
    pub fn build_cookie<'c>(&self, session: &Session) -> Cookie<'c> {
        let mut cookie_builder = Cookie::build(self.name.clone(), session.id().to_string())
            .http_only(true)
            .same_site(self.same_site)
            .secure(self.secure)
            .path(self.path.clone());

        if let Some(max_age) = session
            .expiration_time()
            .map(|et| et - OffsetDateTime::now_utc())
        {
            cookie_builder = cookie_builder.max_age(max_age);
        }

        if let Some(domain) = &self.domain {
            cookie_builder = cookie_builder.domain(domain.clone());
        }

        cookie_builder.finish()
    }

    /// Configures the name of the cookie.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager =
    ///     SessionManager::new(session_store, CookieConfig::default().with_name("my.sid"));
    /// let session_service = SessionManagerLayer::new(session_manager);
    /// ```
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// Configures the `"SameSite"` attribute of the cookie.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{
    ///     cookie::SameSite, CookieConfig, MemoryStore, SessionManager, SessionManagerLayer,
    /// };
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager = SessionManager::new(
    ///     session_store,
    ///     CookieConfig::default().with_same_site(SameSite::Lax),
    /// );
    /// let session_service = SessionManagerLayer::new(session_manager);
    /// ```
    pub fn with_same_site(mut self, same_site: SameSite) -> Self {
        self.same_site = same_site;
        self
    }

    /// Configures the `"Max-Age"` attribute of the cookie.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use time::Duration;
    /// use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager = SessionManager::new(
    ///     session_store,
    ///     CookieConfig::default().with_max_age(Duration::hours(1)),
    /// );
    /// let session_service = SessionManagerLayer::new(session_manager);
    /// ```
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = Some(max_age);
        self
    }

    /// Configures the `"Secure"` attribute of the cookie.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager =
    ///     SessionManager::new(session_store, CookieConfig::default().with_secure(true));
    /// let session_service = SessionManagerLayer::new(session_manager);
    /// ```
    pub fn with_secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    /// Configures the `"Path"` attribute of the cookie.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager = SessionManager::new(
    ///     session_store,
    ///     CookieConfig::default().with_path("/some/path".to_string()),
    /// );
    /// let session_service = SessionManagerLayer::new(session_manager);
    /// ```
    pub fn with_path(mut self, path: String) -> Self {
        self.path = path;
        self
    }

    /// Configures the `"Domain"` attribute of the cookie.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};
    ///
    /// let session_store = MemoryStore::default();
    /// let session_manager = SessionManager::new(
    ///     session_store,
    ///     CookieConfig::default().with_domain("localhost".to_string()),
    /// );
    /// let session_service = SessionManagerLayer::new(session_manager);
    /// ```
    pub fn with_domain(mut self, domain: String) -> Self {
        self.domain = Some(domain);
        self
    }
}

impl Default for CookieConfig {
    fn default() -> Self {
        Self {
            name: String::from("tower.sid"),
            same_site: SameSite::Strict,
            max_age: None, // TODO: Is `Max-Age: "Session"` the right default?
            secure: false,
            path: String::from("/"),
            domain: None,
        }
    }
}
