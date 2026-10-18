//! The only code that talks to the network, and only when built with `--features online`.
//!
//! Default builds contain no HTTP client at all: the app stays strictly offline.

use release_lookup::Request;

/// Whether this build can look things up online.
pub const ENABLED: bool = cfg!(feature = "online");

/// Performs a GET for the lookup crate and returns the body.
#[cfg(feature = "online")]
pub fn get(request: &Request) -> Result<String, String> {
    use std::sync::LazyLock;

    /// The services require a User-Agent that says who is asking.
    const USER_AGENT: &str = concat!(
        "DSDWorkbench/",
        env!("CARGO_PKG_VERSION"),
        " ( https://github.com/malkhuzanie/dsd-workbench )"
    );
    static CLIENT: LazyLock<release_lookup::http::Client> =
        LazyLock::new(|| release_lookup::http::Client::new(USER_AGENT));
    CLIENT.fetch(request)
}

/// Performs a GET for the lookup crate and returns the body.
#[cfg(not(feature = "online"))]
pub fn get(_request: &Request) -> Result<String, String> {
    Err("this build has no online lookup; rebuild with `--features online` to enable it".to_owned())
}
