//! A small blocking HTTP client for [`crate::find`], with the headers and pacing the
//! services require. Enabled by the `http` feature.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::Request;

const TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// MusicBrainz asks for at most one request per second; Discogs allows 60 a minute with a
/// token and 25 without.
const MUSICBRAINZ_GAP: Duration = Duration::from_millis(1100);
const DISCOGS_TOKEN_GAP: Duration = Duration::from_millis(1100);
const DISCOGS_ANONYMOUS_GAP: Duration = Duration::from_millis(2500);

/// Which service a request goes to, for pacing and error messages.
fn service(url: &str) -> &'static str {
    if url.contains("discogs.com") {
        "Discogs"
    } else {
        "MusicBrainz"
    }
}

fn gap_for(request: &Request) -> Duration {
    match service(&request.url) {
        "Discogs"
            if request
                .headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("authorization")) =>
        {
            DISCOGS_TOKEN_GAP
        }
        "Discogs" => DISCOGS_ANONYMOUS_GAP,
        _ => MUSICBRAINZ_GAP,
    }
}

/// Makes requests one at a time, waiting as long as each service asks.
///
/// MusicBrainz and Discogs both require a `User-Agent` that says who you are, so
/// [`Client::new`] insists on one (for example `MyApp/1.0 ( https://example.org/myapp )`).
pub struct Client {
    agent: ureq::Agent,
    last: Mutex<Option<(&'static str, Instant)>>,
}

impl Client {
    /// A client that identifies itself as `user_agent`.
    pub fn new(user_agent: &str) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(TIMEOUT)
                .timeout_connect(CONNECT_TIMEOUT)
                .user_agent(user_agent)
                .build(),
            last: Mutex::new(None),
        }
    }

    /// Waits so that two requests to the same service are far enough apart.
    fn pace(&self, request: &Request) {
        let name = service(&request.url);
        let wait = self
            .last
            .lock()
            .ok()
            .and_then(|last| *last)
            .filter(|(last_name, _)| *last_name == name)
            .map(|(_, at)| gap_for(request).saturating_sub(at.elapsed()));
        if let Some(wait) = wait {
            std::thread::sleep(wait);
        }
        if let Ok(mut last) = self.last.lock() {
            *last = Some((name, Instant::now()));
        }
    }

    /// Performs the GET and returns the body. The error is a sentence for the user.
    pub fn fetch(&self, request: &Request) -> Result<String, String> {
        if !request.url.starts_with("https://") {
            return Err("only https requests are made".to_owned());
        }
        self.pace(request);
        let name = service(&request.url);
        let mut call = self.agent.get(&request.url);
        for (key, value) in &request.headers {
            call = call.set(key, value);
        }
        match call.call() {
            Ok(response) => response
                .into_string()
                .map_err(|e| format!("could not read the reply from {name}: {e}")),
            Err(ureq::Error::Status(code, response)) => {
                // Both services explain errors in a JSON "message" or "error" field.
                let detail = response
                    .into_string()
                    .ok()
                    .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
                    .and_then(|json| {
                        ["message", "error"]
                            .iter()
                            .find_map(|key| json.get(*key)?.as_str().map(str::to_owned))
                    });
                Err(match (code, detail) {
                    (429 | 503, _) => {
                        format!("{name} is busy (rate limited); wait a moment and try again")
                    }
                    (401, _) if name == "Discogs" => {
                        "Discogs refused the request: check your personal access token".to_owned()
                    }
                    (_, Some(message)) => format!("{name} answered with error {code}: {message}"),
                    (_, None) => format!("{name} answered with error {code}"),
                })
            }
            Err(err) => Err(format!("could not reach {name}: {err}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(url: &str, headers: &[(&str, &str)]) -> Request {
        Request {
            url: url.to_owned(),
            headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn pacing_depends_on_the_service_and_the_token() {
        let mb = request("https://musicbrainz.org/ws/2/release/x", &[]);
        let anonymous = request("https://api.discogs.com/releases/1", &[]);
        let token = request(
            "https://api.discogs.com/releases/1",
            &[("Authorization", "Discogs token=x")],
        );
        assert_eq!(gap_for(&mb), MUSICBRAINZ_GAP);
        assert_eq!(gap_for(&anonymous), DISCOGS_ANONYMOUS_GAP);
        assert_eq!(gap_for(&token), DISCOGS_TOKEN_GAP);
        assert_eq!(service(&mb.url), "MusicBrainz");
        assert_eq!(service(&anonymous.url), "Discogs");
    }

    #[test]
    fn plain_http_is_refused_without_a_connection() {
        let client = Client::new("Test/0 ( test )");
        let err = client
            .fetch(&request("http://musicbrainz.org/ws/2/release/x", &[]))
            .expect_err("http is refused");
        assert!(err.contains("https"), "{err}");
    }
}
