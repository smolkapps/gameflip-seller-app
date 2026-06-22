//! A [`Transport`](crate::client::Transport) that never makes a network call.
//!
//! In dry-run mode the CLI uses this transport so a seller can preview exactly
//! which requests *would* be sent (method, URL, redacted headers, body) and
//! confirm the catalog before any real listing is created. Every request gets a
//! synthetic `SUCCESS` response so the multi-step publish flow runs end to end.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::client::{HttpRequest, HttpResponse, Method, Transport};
use crate::error::Result;

/// Records requests and returns synthetic successful responses.
pub struct DryRunTransport {
    /// Accumulated human-readable log lines.
    pub log: RefCell<Vec<String>>,
    /// Monotonic counter used to mint fake listing/photo ids.
    counter: AtomicU64,
    /// When true, also print each request to stdout as it happens.
    print: bool,
}

impl DryRunTransport {
    pub fn new(print: bool) -> Self {
        Self {
            log: RefCell::new(Vec::new()),
            counter: AtomicU64::new(0),
            print,
        }
    }

    fn next_id(&self, prefix: &str) -> String {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        format!("{prefix}-DRYRUN-{n}")
    }

    fn redact_headers(headers: &[(String, String)]) -> Vec<String> {
        headers
            .iter()
            .map(|(k, v)| {
                if k.eq_ignore_ascii_case("authorization") {
                    // Show the scheme and key but hide the live OTP code.
                    let shown = match v.rsplit_once(':') {
                        Some((left, _otp)) => format!("{left}:<otp>"),
                        None => "<redacted>".to_string(),
                    };
                    format!("{k}: {shown}")
                } else {
                    format!("{k}: {v}")
                }
            })
            .collect()
    }
}

impl Transport for DryRunTransport {
    fn execute(&self, req: HttpRequest) -> Result<HttpResponse> {
        let body_preview = if req.body.is_empty() {
            "(no body)".to_string()
        } else {
            // Try to pretty-print JSON; fall back to a byte count for binary.
            match serde_json::from_slice::<serde_json::Value>(&req.body) {
                Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| "(json)".into()),
                Err(_) => format!("({} bytes binary)", req.body.len()),
            }
        };
        let mut lines = vec![format!("[dry-run] {} {}", req.method.as_str(), req.url)];
        for h in Self::redact_headers(&req.headers) {
            lines.push(format!("[dry-run]   {h}"));
        }
        lines.push(format!("[dry-run]   body: {body_preview}"));
        let block = lines.join("\n");
        if self.print {
            println!("{block}");
        }
        self.log.borrow_mut().push(block);

        // Synthesize a plausible SUCCESS body so the publish flow proceeds.
        let body = if req.method == Method::Post && req.url.ends_with("/photo") {
            let id = self.next_id("PH");
            format!(
                r#"{{"status":"SUCCESS","data":{{"id":"{id}","upload_url":"https://dry-run.invalid/upload/{id}"}}}}"#
            )
        } else if req.method == Method::Post && req.url.ends_with("/listing") {
            let id = self.next_id("L");
            format!(r#"{{"status":"SUCCESS","data":{{"id":"{id}","status":"draft"}}}}"#)
        } else {
            r#"{"status":"SUCCESS","data":{}}"#.to_string()
        };

        Ok(HttpResponse {
            status: 200,
            body: body.into_bytes(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Credentials;
    use crate::catalog::{CatalogConfig, PRESET_PACKS};
    use crate::client::GameflipClient;
    use crate::model;

    #[test]
    fn dry_run_publish_records_requests_and_never_panics() {
        let creds = Credentials::new("test-key", "JBSWY3DPEHPK3PXP").unwrap();
        let transport = DryRunTransport::new(false);
        let client = GameflipClient::new(creds, transport);
        let listing = PRESET_PACKS[0].to_listing_request(&CatalogConfig::with_brand("Acme"));
        let id = client
            .publish_listing(&listing, "CODE-1", model::status::ONSALE)
            .unwrap();
        assert!(id.starts_with("L-DRYRUN-"));
        // create + digital_goods + status == 3 recorded requests.
        assert_eq!(client.transport().log.borrow().len(), 3);
    }

    #[test]
    fn dry_run_redacts_otp_in_logged_authorization_header() {
        let creds = Credentials::new("test-key", "JBSWY3DPEHPK3PXP").unwrap();
        let transport = DryRunTransport::new(false);
        let client = GameflipClient::new(creds, transport);
        let listing = PRESET_PACKS[0].to_listing_request(&CatalogConfig::default());
        client.create_listing(&listing).unwrap();
        let log = client.transport().log.borrow().join("\n");
        assert!(log.contains("Authorization: GFAPI test-key:<otp>"));
        // The real 6-digit code must not leak into the log.
        assert!(!log.contains("GFAPI test-key:0"));
    }
}
