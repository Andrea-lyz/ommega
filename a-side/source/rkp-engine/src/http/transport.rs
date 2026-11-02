use std::time::Duration;

use anyhow::{Result, ensure};
use ciborium::value::Value;
use reqwest::{Client, Response, Url, redirect::Policy};

use crate::{
    cbor::{encode, int, text},
    error::RkpError,
};

const MAX_RESPONSE: usize = 1024 * 1024;

pub fn validate_endpoint(value: &str) -> Result<()> {
    endpoint_url(value).map(|_| ())
}

fn endpoint_url(value: &str) -> Result<Url> {
    let mut url = Url::parse(value).map_err(|_| RkpError::InvalidEndpoint)?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
    ensure!(
        (url.scheme() == "https" || (url.scheme() == "http" && loopback))
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        RkpError::InvalidEndpoint
    );
    let path = match url.path().trim_end_matches('/') {
        "" => "/v1",
        path => path,
    }
    .to_owned();
    url.set_path(&path);
    Ok(url)
}

pub(super) fn rpc_url(endpoint: &str, method: &str) -> Result<Url> {
    let mut url = endpoint_url(endpoint)?;
    url.set_path(&format!("{}:{method}", url.path()));
    Ok(url)
}

pub fn rkp_client() -> Result<Client> {
    // Port note: the original streamed this through duck-platform's embedded
    // rustls config plus reqwest's retry hook. Both are reqwest 0.13 APIs; this
    // port uses reqwest 0.12 with its bundled rustls (ring) backend, the same
    // stack the rest of the module already builds for Android, and relies on the
    // single explicit attempt the RKP protocol expects.
    let builder = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(Policy::none())
        .use_rustls_tls();
    Ok(builder.build()?)
}

pub async fn fetch_challenge(
    client: &Client,
    fingerprint: &str,
    server_url: &str,
) -> Result<Vec<u8>> {
    let url = rpc_url(server_url, "fetchEekChain")?;
    let payload = encode(&Value::Map(vec![
        (text("fingerprint"), text(fingerprint)),
        (text("id"), int(42)),
    ]))?;
    let response = client
        .post(url)
        .header("Content-Type", "application/cbor")
        .header("Accept", "application/cbor")
        .body(payload)
        .send()
        .await
        .map_err(|_| RkpError::ChallengeUnavailable)?;
    let status = response.status();
    let body = read_body(response, false).await?;
    if !status.is_success() {
        return Err(super::map_http_error(status, &body).into());
    }
    super::parse_challenge_response(&body)
}

pub(super) async fn read_body(mut response: Response, submitted: bool) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        if submitted {
            RkpError::SubmissionUnknown
        } else {
            RkpError::ChallengeUnavailable
        }
    })? {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_RESPONSE,
            RkpError::InvalidRkpResponse("response exceeds size limit".into())
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
