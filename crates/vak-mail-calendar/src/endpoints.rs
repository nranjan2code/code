//! Where each provider call goes: the one place the Google, Microsoft and
//! OAuth hosts are named.
//!
//! A build with the `test-support` feature, and only such a build, sends
//! every call to a stand-in when `VAK_TEST_PROVIDER_BASE` names a plain
//! HTTP loopback origin (`http://127.0.0.1:<port>`). Integration tests and
//! the mail and calendar soak use it; a normal build has no override, and a
//! test build refuses any other host.

/// The base of every provider endpoint Vakyartha calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub gmail: String,
    pub calendar: String,
    pub graph: String,
    pub google_authorize: String,
    pub google_token: String,
    pub google_signing_keys: String,
    pub google_revoke: String,
    pub microsoft_authorize: String,
    pub microsoft_token: String,
    pub microsoft_signing_keys: String,
}

impl Endpoints {
    fn providers() -> Self {
        Self {
            gmail: "https://gmail.googleapis.com/gmail/v1".into(),
            calendar: "https://www.googleapis.com/calendar/v3".into(),
            graph: "https://graph.microsoft.com/v1.0".into(),
            google_authorize: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            google_token: "https://oauth2.googleapis.com/token".into(),
            google_signing_keys: "https://www.googleapis.com/oauth2/v3/certs".into(),
            google_revoke: "https://oauth2.googleapis.com/revoke".into(),
            microsoft_authorize: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"
                .into(),
            microsoft_token: "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
            microsoft_signing_keys: "https://login.microsoftonline.com/common/discovery/v2.0/keys"
                .into(),
        }
    }

    /// Every endpoint under one stand-in origin.
    pub fn stand_in(origin: &str) -> Self {
        let at = |path: &str| format!("{}/{path}", origin.trim_end_matches('/'));
        Self {
            gmail: at("gmail/v1"),
            calendar: at("calendar/v3"),
            graph: at("graph/v1.0"),
            google_authorize: at("google/authorize"),
            google_token: at("google/token"),
            google_signing_keys: at("google/certs"),
            google_revoke: at("google/revoke"),
            microsoft_authorize: at("microsoft/authorize"),
            microsoft_token: at("microsoft/token"),
            microsoft_signing_keys: at("microsoft/keys"),
        }
    }
}

/// The endpoints this process calls.
pub fn current() -> Endpoints {
    #[cfg(feature = "test-support")]
    if let Some(origin) =
        vak_config::get_var("VAK_TEST_PROVIDER_BASE").filter(|origin| is_loopback_origin(origin))
    {
        return Endpoints::stand_in(&origin);
    }
    Endpoints::providers()
}

#[cfg(any(test, feature = "test-support"))]
fn is_loopback_origin(origin: &str) -> bool {
    url::Url::parse(origin).is_ok_and(|url| {
        url.scheme() == "http"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && matches!(url.path(), "" | "/")
            && url.port().is_some_and(|port| port != 0)
            && url
                .host_str()
                .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "[::1]"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_loopback_http_origin_is_a_stand_in() {
        assert!(is_loopback_origin("http://127.0.0.1:9090"));
        assert!(is_loopback_origin("http://localhost:9090/"));
        assert!(!is_loopback_origin("https://127.0.0.1:9090"));
        assert!(!is_loopback_origin("http://example.com:9090"));
        assert!(!is_loopback_origin("http://127.0.0.1"));
        assert!(!is_loopback_origin("http://127.0.0.1:9090/gmail"));
        assert!(!is_loopback_origin("http://user@127.0.0.1:9090"));
    }

    #[test]
    fn a_stand_in_names_every_endpoint_under_its_origin() {
        let stand_in = Endpoints::stand_in("http://127.0.0.1:9090/");
        assert_eq!(stand_in.gmail, "http://127.0.0.1:9090/gmail/v1");
        assert_eq!(
            stand_in.microsoft_token,
            "http://127.0.0.1:9090/microsoft/token"
        );
    }
}
