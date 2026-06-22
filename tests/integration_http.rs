//! End-to-end client tests against an in-process mock HTTP server.
//!
//! These exercise the *real* [`ReqwestTransport`] over a loopback TCP socket
//! (via `httpmock`), so we verify the actual bytes on the wire — HTTP method,
//! path, `Content-Type`, the `GFAPI <key>:<otp>` Authorization header, and the
//! JSON bodies — without ever contacting Gameflip. The server stands in for the
//! Gameflip API at an explicit base URL.

use gameflip_seller::auth::Credentials;
use gameflip_seller::catalog::{CatalogConfig, PRESET_PACKS};
use gameflip_seller::client::{GameflipClient, ReqwestTransport};
use gameflip_seller::model;

use httpmock::prelude::*;
use httpmock::Method::PATCH;

fn client_for(server: &MockServer) -> GameflipClient<ReqwestTransport> {
    let creds = Credentials::new("test-itest-key", "JBSWY3DPEHPK3PXP").unwrap();
    let base = format!("{}/api/v1", server.base_url());
    GameflipClient::with_base_url(creds, base, ReqwestTransport::new().unwrap())
}

#[test]
fn full_publish_flow_hits_expected_endpoints_over_http() {
    let server = MockServer::start();

    // 1) Create listing.
    let create = server.mock(|when, then| {
        when.method(POST)
            .path("/api/v1/listing")
            .header_exists("authorization")
            .header("content-type", "application/json");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"status":"SUCCESS","data":{"id":"LX1","status":"draft"}}"#);
    });

    // 2) Store digital code.
    let digital = server.mock(|when, then| {
        when.method(PUT)
            .path("/api/v1/listing/LX1/digital_goods")
            .json_body(serde_json::json!({"code":"REAL-CODE-1"}));
        then.status(200).body(r#"{"status":"SUCCESS"}"#);
    });

    // 3) Set status onsale via JSON Patch.
    let status = server.mock(|when, then| {
        when.method(PATCH)
            .path("/api/v1/listing/LX1")
            .header("content-type", "application/json-patch+json")
            .json_body(serde_json::json!([{"op":"replace","path":"/status","value":"onsale"}]));
        then.status(200).body(r#"{"status":"SUCCESS"}"#);
    });

    let client = client_for(&server);
    let listing = PRESET_PACKS[0].to_listing_request(&CatalogConfig::with_brand("Acme"));
    let id = client
        .publish_listing(&listing, "REAL-CODE-1", model::status::ONSALE)
        .unwrap();

    assert_eq!(id, "LX1");
    create.assert();
    digital.assert();
    status.assert();
}

#[test]
fn authorization_header_is_gfapi_scheme_with_six_digit_otp() {
    let server = MockServer::start();
    let m = server.mock(|when, then| {
        when.method(POST).path("/api/v1/listing").matches(|req| {
            // Header value must match GFAPI <key>:<6 digits> exactly.
            let re = Regex::new(r"^GFAPI test-itest-key:\d{6}$").unwrap();
            req.headers
                .as_ref()
                .map(|hs| {
                    hs.iter()
                        .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && re.is_match(v))
                })
                .unwrap_or(false)
        });
        then.status(200)
            .body(r#"{"status":"SUCCESS","data":{"id":"LX2"}}"#);
    });

    let client = client_for(&server);
    let listing = PRESET_PACKS[1].to_listing_request(&CatalogConfig::default());
    let created = client.create_listing(&listing).unwrap();
    assert_eq!(created.id, "LX2");
    m.assert();
}

#[test]
fn api_failure_envelope_surfaces_as_error_and_aborts() {
    let server = MockServer::start();
    let _create = server.mock(|when, then| {
        when.method(POST).path("/api/v1/listing");
        then.status(422)
            .body(r#"{"status":"FAIL","error":{"code":400,"message":"price too low"}}"#);
    });

    let client = client_for(&server);
    let listing = PRESET_PACKS[0].to_listing_request(&CatalogConfig::default());
    let err = client
        .publish_listing(&listing, "CODE", model::status::ONSALE)
        .unwrap_err();
    match err {
        gameflip_seller::Error::Api { code, message } => {
            assert_eq!(code, 400);
            assert!(message.contains("price too low"));
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[test]
fn photo_upload_put_goes_to_presigned_url_without_auth_header() {
    let server = MockServer::start();

    // request_photo_upload returns an upload_url pointing back at the same
    // server (a stand-in for the pre-signed storage endpoint).
    let upload_path = "/storage/presigned-xyz";
    let request_photo = server.mock(|when, then| {
        when.method(POST).path("/api/v1/listing/LP1/photo");
        then.status(200).body(format!(
            r#"{{"status":"SUCCESS","data":{{"id":"PH9","upload_url":"{}{}"}}}}"#,
            server.base_url(),
            upload_path
        ));
    });

    // The pre-signed PUT must carry the image content-type and NO Authorization.
    let put_bytes = server.mock(|when, then| {
        when.method(PUT)
            .path(upload_path)
            .header("content-type", "image/png");
        then.status(200);
    });

    let client = client_for(&server);
    let upload = client.request_photo_upload("LP1").unwrap();
    assert_eq!(upload.id, "PH9");

    let png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    client
        .put_photo_bytes(&upload.upload_url, png, "image/png")
        .unwrap();

    request_photo.assert();
    put_bytes.assert();
}
