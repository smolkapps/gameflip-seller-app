//! Gameflip seller API client.
//!
//! The client is generic over a [`Transport`] so the listing-publishing logic
//! can be unit-tested against an in-memory recorder (and exercised in
//! `--dry-run` mode) without ever contacting the live Gameflip service. A real
//! [`ReqwestTransport`] is provided for production use.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::auth::Credentials;
use crate::error::{Error, Result};
use crate::model::{self, ApiEnvelope, ListingRequest, ListingResponse, PatchOp, PhotoUpload};

/// Production API base URLs, selected from the API-key prefix exactly as the
/// reference bindings do (`test-…` / `dev-…` / otherwise production).
pub const BASE_URL_PRODUCTION: &str = "https://production-gameflip.fingershock.com/api/v1";
pub const BASE_URL_TEST: &str = "https://test-gameflip.fingershock.com/api/v1";
pub const BASE_URL_DEV: &str = "http://localhost:3000/api/v1";

/// Pick the base URL implied by an API key's environment prefix.
pub fn base_url_for_key(api_key: &str) -> &'static str {
    match api_key.split('-').next() {
        Some("test") => BASE_URL_TEST,
        Some("dev") | Some("development") => BASE_URL_DEV,
        _ => BASE_URL_PRODUCTION,
    }
}

/// HTTP verb for a [`HttpRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
}

impl Method {
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
        }
    }
}

/// A fully-formed HTTP request the transport must execute.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    /// Header name/value pairs (already includes Authorization & Content-Type).
    pub headers: Vec<(String, String)>,
    /// Raw request body (JSON text, or image bytes for photo uploads).
    pub body: Vec<u8>,
}

/// The transport's response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Pluggable HTTP backend. Implemented by [`ReqwestTransport`] for production
/// and by a recorder in tests / dry-run mode.
pub trait Transport {
    fn execute(&self, req: HttpRequest) -> Result<HttpResponse>;
}

/// The high-level seller client.
pub struct GameflipClient<T: Transport> {
    creds: Credentials,
    base_url: String,
    transport: T,
}

impl<T: Transport> GameflipClient<T> {
    /// Build a client, deriving the base URL from the API key prefix.
    pub fn new(creds: Credentials, transport: T) -> Self {
        let base_url = base_url_for_key(creds.api_key()).to_string();
        Self {
            creds,
            base_url,
            transport,
        }
    }

    /// Build a client against an explicit base URL (used by tests/mocks).
    pub fn with_base_url(creds: Credentials, base_url: impl Into<String>, transport: T) -> Self {
        Self {
            creds,
            base_url: base_url.into(),
            transport,
        }
    }

    /// Borrow the underlying transport. Primarily useful for inspecting a
    /// recording transport (the dry-run transport's log) in tests and tooling.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn auth_headers(&self, content_type: &str) -> Vec<(String, String)> {
        vec![
            (
                "Authorization".to_string(),
                self.creds.authorization_header(),
            ),
            ("Content-Type".to_string(), content_type.to_string()),
        ]
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    /// Run a request whose JSON body envelope wraps a `T` payload, and return
    /// the unwrapped payload. Enforces both HTTP status and the API's
    /// `status == "SUCCESS"` contract.
    fn send_json<R: DeserializeOwned>(&self, req: HttpRequest) -> Result<R> {
        let resp = self.transport.execute(req)?;
        parse_envelope::<R>(resp.status, &resp.body)
    }

    /// `POST /listing` — create a listing. Returns the new listing (with its id).
    pub fn create_listing(&self, listing: &ListingRequest) -> Result<ListingResponse> {
        let body = serde_json::to_vec(listing)
            .map_err(|e| Error::Decode(format!("serialize listing: {e}")))?;
        let req = HttpRequest {
            method: Method::Post,
            url: self.url("listing"),
            headers: self.auth_headers("application/json"),
            body,
        };
        self.send_json(req)
    }

    /// `PUT /listing/{id}/digital_goods` — store the digital code for a listing
    /// so Gameflip can auto-deliver it. The code must be unique per listing.
    pub fn put_digital_goods(&self, listing_id: &str, code: &str) -> Result<()> {
        let body = serde_json::to_vec(&serde_json::json!({ "code": code }))
            .map_err(|e| Error::Decode(format!("serialize digital_goods: {e}")))?;
        let req = HttpRequest {
            method: Method::Put,
            url: self.url(&format!("listing/{listing_id}/digital_goods")),
            headers: self.auth_headers("application/json"),
            body,
        };
        // The endpoint returns a SUCCESS envelope; we don't need its payload.
        let resp = self.transport.execute(req)?;
        ensure_success(resp.status, &resp.body)
    }

    /// `PATCH /listing/{id}` with `Content-Type: application/json-patch+json`.
    pub fn patch_listing(&self, listing_id: &str, ops: &[PatchOp]) -> Result<()> {
        let body =
            serde_json::to_vec(ops).map_err(|e| Error::Decode(format!("serialize patch: {e}")))?;
        let req = HttpRequest {
            method: Method::Patch,
            url: self.url(&format!("listing/{listing_id}")),
            headers: self.auth_headers("application/json-patch+json"),
            body,
        };
        let resp = self.transport.execute(req)?;
        ensure_success(resp.status, &resp.body)
    }

    /// Transition a listing to `new_status` (e.g. `ready`, `onsale`).
    pub fn set_status(&self, listing_id: &str, new_status: &str) -> Result<()> {
        self.patch_listing(listing_id, &model::status_patch(new_status))
    }

    /// `POST /listing/{id}/photo` — request an upload URL for a new photo.
    pub fn request_photo_upload(&self, listing_id: &str) -> Result<PhotoUpload> {
        let req = HttpRequest {
            method: Method::Post,
            url: self.url(&format!("listing/{listing_id}/photo")),
            headers: self.auth_headers("application/json"),
            body: Vec::new(),
        };
        self.send_json(req)
    }

    /// PUT raw image bytes to a pre-signed upload URL (no auth header — the URL
    /// is already signed). `mime` must be `image/jpeg` or `image/png`.
    pub fn put_photo_bytes(&self, upload_url: &str, bytes: Vec<u8>, mime: &str) -> Result<()> {
        const MAX_IMAGE_BYTES: usize = 500_000;
        if mime != "image/jpeg" && mime != "image/png" {
            return Err(Error::Photo(format!("unsupported image MIME type: {mime}")));
        }
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(Error::Photo(format!(
                "image too large: {} > {} bytes",
                bytes.len(),
                MAX_IMAGE_BYTES
            )));
        }
        let req = HttpRequest {
            method: Method::Put,
            url: upload_url.to_string(),
            headers: vec![("Content-Type".to_string(), mime.to_string())],
            body: bytes,
        };
        let resp = self.transport.execute(req)?;
        // The pre-signed storage endpoint returns a bare 200, not a Gameflip
        // envelope, so only the HTTP status is meaningful here.
        if (200..300).contains(&resp.status) {
            Ok(())
        } else {
            Err(Error::api(resp.status as i64, "photo upload (PUT) failed"))
        }
    }

    /// Full publish of one listing: create it, store its digital code, then set
    /// it to `final_status` (`ready` or `onsale`). Returns the listing id.
    ///
    /// Photos are optional and handled separately (via [`Self::request_photo_upload`],
    /// [`Self::put_photo_bytes`], then [`Self::patch_listing`]); keeping them out
    /// of this method means a missing image never blocks publishing.
    pub fn publish_listing(
        &self,
        listing: &ListingRequest,
        digital_code: &str,
        final_status: &str,
    ) -> Result<String> {
        let created = self.create_listing(listing)?;
        self.put_digital_goods(&created.id, digital_code)?;
        self.set_status(&created.id, final_status)?;
        Ok(created.id)
    }
}

/// Parse a Gameflip envelope, returning the inner `data` on success or a
/// structured [`Error::Api`] otherwise.
pub fn parse_envelope<R: DeserializeOwned>(http_status: u16, body: &[u8]) -> Result<R> {
    let env: ApiEnvelope<R> = serde_json::from_slice(body).map_err(|e| {
        Error::Decode(format!(
            "invalid JSON envelope (HTTP {http_status}): {e}; body={}",
            String::from_utf8_lossy(body)
        ))
    })?;
    if env.status.as_deref() == Some("SUCCESS") {
        env.data
            .ok_or_else(|| Error::Decode("SUCCESS envelope had no data".into()))
    } else {
        Err(envelope_error(http_status, env.error))
    }
}

/// Like [`parse_envelope`] but for endpoints whose payload we ignore.
pub fn ensure_success(http_status: u16, body: &[u8]) -> Result<()> {
    // Some success bodies have no `data`; accept any SUCCESS status.
    let env: ApiEnvelope<Value> = serde_json::from_slice(body).map_err(|e| {
        Error::Decode(format!(
            "invalid JSON envelope (HTTP {http_status}): {e}; body={}",
            String::from_utf8_lossy(body)
        ))
    })?;
    if env.status.as_deref() == Some("SUCCESS") {
        Ok(())
    } else {
        Err(envelope_error(http_status, env.error))
    }
}

fn envelope_error(http_status: u16, err: Option<model::ApiError>) -> Error {
    let (code, message) = match err {
        Some(e) => {
            let code = match e.code {
                Some(Value::Number(n)) => n.as_i64().unwrap_or(http_status as i64),
                Some(Value::String(s)) => s.parse::<i64>().unwrap_or(http_status as i64),
                _ => http_status as i64,
            };
            (code, e.message.unwrap_or_else(|| "unknown error".into()))
        }
        None => (http_status as i64, "request failed".into()),
    };
    Error::api(code, message)
}

/// Production transport backed by a blocking `reqwest::Client`.
pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("gameflip-seller-app/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Transport(format!("building HTTP client: {e}")))?;
        Ok(Self { client })
    }
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::new().expect("failed to build reqwest client")
    }
}

impl Transport for ReqwestTransport {
    fn execute(&self, req: HttpRequest) -> Result<HttpResponse> {
        let method = match req.method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Patch => reqwest::Method::PATCH,
        };
        let mut builder = self.client.request(method, &req.url);
        for (k, v) in &req.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        if !req.body.is_empty() {
            builder = builder.body(req.body);
        }
        let resp = builder
            .send()
            .map_err(|e| Error::Transport(format!("{} {}: {e}", req.method.as_str(), req.url)))?;
        let status = resp.status().as_u16();
        let body = resp
            .bytes()
            .map_err(|e| Error::Transport(format!("reading response body: {e}")))?
            .to_vec();
        Ok(HttpResponse { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// In-memory transport that records every request and replays a queue of
    /// canned responses (FIFO). Lets us assert exact URLs/headers/bodies and
    /// drive the multi-step publish flow with no network.
    struct MockTransport {
        responses: RefCell<Vec<HttpResponse>>,
        recorded: RefCell<Vec<HttpRequest>>,
    }

    impl MockTransport {
        fn new(responses: Vec<HttpResponse>) -> Self {
            Self {
                responses: RefCell::new(responses),
                recorded: RefCell::new(Vec::new()),
            }
        }
        fn ok(json: &str) -> HttpResponse {
            HttpResponse {
                status: 200,
                body: json.as_bytes().to_vec(),
            }
        }
    }

    impl Transport for MockTransport {
        fn execute(&self, req: HttpRequest) -> Result<HttpResponse> {
            self.recorded.borrow_mut().push(req);
            let mut r = self.responses.borrow_mut();
            if r.is_empty() {
                panic!("MockTransport: no more canned responses");
            }
            Ok(r.remove(0))
        }
    }

    fn creds() -> Credentials {
        Credentials::new("test-key123", "JBSWY3DPEHPK3PXP").unwrap()
    }

    fn sample_listing() -> ListingRequest {
        crate::catalog::PRESET_PACKS[0]
            .to_listing_request(&crate::catalog::CatalogConfig::with_brand("Acme"))
    }

    #[test]
    fn base_url_selected_from_key_prefix() {
        assert_eq!(base_url_for_key("test-abc"), BASE_URL_TEST);
        assert_eq!(base_url_for_key("dev-abc"), BASE_URL_DEV);
        assert_eq!(base_url_for_key("abc123"), BASE_URL_PRODUCTION);
    }

    #[test]
    fn create_listing_sends_correct_request_and_parses_envelope() {
        let mock = MockTransport::new(vec![MockTransport::ok(
            r#"{"status":"SUCCESS","data":{"id":"LIST123","status":"draft","price":950}}"#,
        )]);
        let client = GameflipClient::new(creds(), mock);
        let created = client.create_listing(&sample_listing()).unwrap();
        assert_eq!(created.id, "LIST123");
        assert_eq!(created.price, Some(950));

        // Inspect the recorded request.
        let req = &client.transport.recorded.borrow()[0];
        assert_eq!(req.method, Method::Post);
        assert!(req.url.ends_with("/listing"));
        assert!(req.url.starts_with(BASE_URL_TEST));
        // Auth header present and correctly shaped.
        let auth = req
            .headers
            .iter()
            .find(|(k, _)| k == "Authorization")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert!(auth.starts_with("GFAPI test-key123:"));
        // Content-Type is application/json for create.
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "application/json"));
        // Body is the serialized listing.
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["category"], "GIFTCARD");
        assert_eq!(body["digital"], true);
    }

    #[test]
    fn put_digital_goods_uses_put_and_code_body() {
        let mock = MockTransport::new(vec![MockTransport::ok(r#"{"status":"SUCCESS"}"#)]);
        let client = GameflipClient::new(creds(), mock);
        client.put_digital_goods("LIST123", "CODE-XYZ").unwrap();

        let req = &client.transport.recorded.borrow()[0];
        assert_eq!(req.method, Method::Put);
        assert!(req.url.ends_with("/listing/LIST123/digital_goods"));
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["code"], "CODE-XYZ");
    }

    #[test]
    fn set_status_sends_json_patch_with_correct_content_type() {
        let mock = MockTransport::new(vec![MockTransport::ok(r#"{"status":"SUCCESS"}"#)]);
        let client = GameflipClient::new(creds(), mock);
        client.set_status("LIST123", model::status::ONSALE).unwrap();

        let req = &client.transport.recorded.borrow()[0];
        assert_eq!(req.method, Method::Patch);
        assert!(req.url.ends_with("/listing/LIST123"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "application/json-patch+json"));
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(
            body,
            serde_json::json!([{"op":"replace","path":"/status","value":"onsale"}])
        );
    }

    #[test]
    fn publish_listing_runs_create_then_code_then_status_in_order() {
        let mock = MockTransport::new(vec![
            MockTransport::ok(r#"{"status":"SUCCESS","data":{"id":"L1"}}"#), // create
            MockTransport::ok(r#"{"status":"SUCCESS"}"#),                    // digital_goods
            MockTransport::ok(r#"{"status":"SUCCESS"}"#),                    // status
        ]);
        let client = GameflipClient::new(creds(), mock);
        let id = client
            .publish_listing(&sample_listing(), "CODE-1", model::status::ONSALE)
            .unwrap();
        assert_eq!(id, "L1");

        let recorded = client.transport.recorded.borrow();
        assert_eq!(recorded.len(), 3);
        assert_eq!(recorded[0].method, Method::Post); // create
        assert!(recorded[0].url.ends_with("/listing"));
        assert_eq!(recorded[1].method, Method::Put); // digital_goods
        assert!(recorded[1].url.ends_with("/digital_goods"));
        assert_eq!(recorded[2].method, Method::Patch); // status
    }

    #[test]
    fn photo_upload_flow_records_post_put_patch() {
        let mock = MockTransport::new(vec![
            MockTransport::ok(
                r#"{"status":"SUCCESS","data":{"id":"PH1","upload_url":"https://up.example/abc"}}"#,
            ),
            HttpResponse {
                status: 200,
                body: b"".to_vec(),
            }, // PUT to storage (bare 200)
            MockTransport::ok(r#"{"status":"SUCCESS"}"#), // activation patch
        ]);
        let client = GameflipClient::new(creds(), mock);

        let upload = client.request_photo_upload("L1").unwrap();
        assert_eq!(upload.id, "PH1");
        assert_eq!(upload.upload_url, "https://up.example/abc");

        // PNG magic bytes, well under the size limit.
        let png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        client
            .put_photo_bytes(&upload.upload_url, png, "image/png")
            .unwrap();
        client
            .patch_listing("L1", &model::photo_activation_patch(&upload.id, Some(0)))
            .unwrap();

        let recorded = client.transport.recorded.borrow();
        assert_eq!(recorded[0].method, Method::Post);
        assert!(recorded[0].url.ends_with("/listing/L1/photo"));
        assert_eq!(recorded[1].method, Method::Put);
        assert_eq!(recorded[1].url, "https://up.example/abc");
        // The pre-signed PUT must NOT carry the Gameflip Authorization header.
        assert!(!recorded[1]
            .headers
            .iter()
            .any(|(k, _)| k == "Authorization"));
        assert!(recorded[1]
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "image/png"));
        assert_eq!(recorded[2].method, Method::Patch);
    }

    #[test]
    fn put_photo_bytes_rejects_bad_mime() {
        let mock = MockTransport::new(vec![]);
        let client = GameflipClient::new(creds(), mock);
        let err = client
            .put_photo_bytes("https://up.example/x", vec![1, 2, 3], "image/gif")
            .unwrap_err();
        assert!(matches!(err, Error::Photo(_)));
    }

    #[test]
    fn put_photo_bytes_rejects_oversize() {
        let mock = MockTransport::new(vec![]);
        let client = GameflipClient::new(creds(), mock);
        let big = vec![0u8; 500_001];
        let err = client
            .put_photo_bytes("https://up.example/x", big, "image/png")
            .unwrap_err();
        assert!(matches!(err, Error::Photo(_)));
    }

    #[test]
    fn non_success_envelope_becomes_structured_api_error() {
        let mock = MockTransport::new(vec![HttpResponse {
            status: 422,
            body: br#"{"status":"FAIL","error":{"code":412,"message":"If-Match check failed"}}"#
                .to_vec(),
        }]);
        let client = GameflipClient::new(creds(), mock);
        let err = client.set_status("L1", "onsale").unwrap_err();
        match err {
            Error::Api { code, message } => {
                assert_eq!(code, 412);
                assert!(message.contains("If-Match"));
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[test]
    fn string_error_code_is_parsed() {
        let err = envelope_error(
            400,
            Some(model::ApiError {
                code: Some(serde_json::Value::String("404".into())),
                message: Some("not found".into()),
            }),
        );
        match err {
            Error::Api { code, .. } => assert_eq!(code, 404),
            _ => panic!("expected Api error"),
        }
    }

    #[test]
    fn malformed_json_is_a_decode_error() {
        let mock = MockTransport::new(vec![HttpResponse {
            status: 200,
            body: b"not json".to_vec(),
        }]);
        let client = GameflipClient::new(creds(), mock);
        let err = client.create_listing(&sample_listing()).unwrap_err();
        assert!(matches!(err, Error::Decode(_)));
    }

    #[test]
    fn transport_error_propagates() {
        struct Boom;
        impl Transport for Boom {
            fn execute(&self, _req: HttpRequest) -> Result<HttpResponse> {
                Err(Error::Transport("connection refused".into()))
            }
        }
        let client = GameflipClient::new(creds(), Boom);
        let err = client.create_listing(&sample_listing()).unwrap_err();
        assert!(matches!(err, Error::Transport(_)));
    }
}
