//! Defines the configuration for the cookie belonging to the session.
use tower_cookies::{cookie::SameSite, Cookie};

use crate::{session::Expiry, Session};

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

    /// Specifies the maximum age of the session.
    pub expiry: Option<Expiry>,

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

        cookie_builder = cookie_builder.max_age(session.expiry_age());

        if let Some(domain) = &self.domain {
            cookie_builder = cookie_builder.domain(domain.clone());
        }

        cookie_builder.finish()
    }
}

impl Default for CookieConfig {
    fn default() -> Self {
        Self {
            name: String::from("tower.sid"),
            same_site: SameSite::Strict,
            expiry: None, // TODO: Is `Max-Age: "Session"` the right default?
            secure: false,
            path: String::from("/"),
            domain: None,
        }
    }
}
