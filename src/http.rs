use std::time::Duration;

use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue, USER_AGENT},
    Client,
};

pub fn client() -> Client {
    let mut default_header = HeaderMap::new();
    default_header.append(
        USER_AGENT,
        HeaderValue::from_static("Notifi-printer (https://github.com/angeloanan/notifi-printer)"),
    );

    Client::builder()
        .default_headers(default_header)
        .brotli(true)
        .gzip(true)
        .zstd(true)
        // .https_only(true)
        // .http2_prior_knowledge()
        .timeout(Duration::from_secs(30))
        .tcp_keepalive(Some(Duration::from_secs(120)))
        .http2_keep_alive_interval(Some(Duration::from_secs(30)))
        .http2_keep_alive_while_idle(true)
        .build()
        .expect("Unable to build HTTP client")
}
