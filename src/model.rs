//! Data model for the Gameflip listing API.
//!
//! Field names and string-enum values mirror the Gameflip REST API exactly so
//! that the serialized JSON is wire-compatible with the reference Node bindings.

use serde::{Deserialize, Serialize};

/// Gameflip product category. Credit packs are sold as digital gift-card-style
/// items, so the catalog uses [`Category::Giftcard`].
pub mod category {
    pub const GAMES: &str = "CONSOLE_VIDEO_GAMES";
    pub const INGAME: &str = "DIGITAL_INGAME";
    pub const GIFTCARD: &str = "GIFTCARD";
}

/// Listing kind.
pub mod kind {
    pub const ITEM: &str = "item";
    pub const GIG: &str = "gig";
}

/// How a digital item is delivered.
pub mod digital_deliverable {
    /// A code/key entered into Gameflip (enables auto-delivery).
    pub const CODE: &str = "code";
    /// Transfer handled manually between buyer and seller.
    pub const TRANSFER: &str = "transfer";
}

/// Currency the seller will accept.
pub mod accept_currency {
    pub const USD: &str = "USD";
    pub const FLP: &str = "FLP";
    pub const BOTH: &str = "BOTH";
}

/// Listing lifecycle status.
pub mod status {
    pub const DRAFT: &str = "draft";
    pub const READY: &str = "ready";
    pub const ONSALE: &str = "onsale";
    pub const SALE_PENDING: &str = "sale_pending";
    pub const SOLD: &str = "sold";
}

/// Photo lifecycle status (used inside JSON-patch paths).
pub mod photo_status {
    pub const PENDING: &str = "pending";
    pub const ACTIVE: &str = "active";
    pub const DELETED: &str = "deleted";
}

/// `shipping_within_days = 0` means Gameflip auto-delivers the stored code.
pub const SHIPPING_WITHIN_DAYS_AUTO: u32 = 0;

/// Allowed listing-expiry windows.
pub mod expire_in_days {
    pub const SEVEN: u32 = 7;
    pub const FOURTEEN: u32 = 14;
    pub const THIRTY: u32 = 30;
}

/// The body sent to `POST /listing` to create a listing.
///
/// Only the fields the catalog needs are modeled; everything is `String`/number
/// to keep the JSON identical to the reference sample. Fields that are `None`
/// are omitted from the serialized body (Gameflip treats absent fields as
/// "use default").
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ListingRequest {
    /// Display title. Convention: include the dollar value (e.g. "$20.00 …").
    pub name: String,
    /// Long description shown on the listing page.
    pub description: String,
    /// Price in **cents** USD.
    pub price: u64,
    /// Search/filter tags, e.g. `["balance: 2000", "currency: USD", "type: giftcard"]`.
    pub tags: Vec<String>,

    pub platform: String,
    pub category: String,
    pub kind: String,

    pub digital: bool,
    /// Region restriction; `"none"` for unrestricted.
    pub digital_region: String,
    pub digital_deliverable: String,

    pub expire_in_days: u32,
    pub shipping_within_days: u32,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept_currency: Option<String>,
}

/// Standard Gameflip response envelope: `{ "status": "...", "data": ..., "error": ... }`.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiEnvelope<T> {
    pub status: Option<String>,
    pub data: Option<T>,
    pub error: Option<ApiError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiError {
    pub code: Option<serde_json::Value>,
    pub message: Option<String>,
}

/// Subset of the listing object returned by `POST /listing`.
#[derive(Debug, Clone, Deserialize)]
pub struct ListingResponse {
    pub id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub price: Option<u64>,
}

/// Response from `POST /listing/{id}/photo` (request-upload-permission step).
#[derive(Debug, Clone, Deserialize)]
pub struct PhotoUpload {
    pub id: String,
    pub upload_url: String,
    #[serde(default)]
    pub view_url: Option<String>,
}

/// A single RFC 6902 JSON-Patch operation. Gameflip mutations (status change,
/// photo activation) are expressed as arrays of these.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PatchOp {
    pub op: String,
    pub path: String,
    pub value: serde_json::Value,
}

impl PatchOp {
    pub fn replace(path: impl Into<String>, value: serde_json::Value) -> Self {
        PatchOp {
            op: "replace".into(),
            path: path.into(),
            value,
        }
    }
}

/// Build the JSON-patch body that transitions a listing to a new status.
pub fn status_patch(new_status: &str) -> Vec<PatchOp> {
    vec![PatchOp::replace(
        "/status",
        serde_json::Value::String(new_status.to_string()),
    )]
}

/// Build the JSON-patch body that activates an uploaded photo and either sets
/// its display order or makes it the cover photo.
///
/// Mirrors the reference `upload_photo`: when `display_order` is `Some`, set the
/// order; otherwise mark it the cover photo.
pub fn photo_activation_patch(photo_id: &str, display_order: Option<u32>) -> Vec<PatchOp> {
    let mut ops = vec![PatchOp::replace(
        format!("/photo/{photo_id}/status"),
        serde_json::Value::String(photo_status::ACTIVE.to_string()),
    )];
    match display_order {
        Some(order) => ops.push(PatchOp::replace(
            format!("/photo/{photo_id}/display_order"),
            serde_json::Value::Number(order.into()),
        )),
        None => ops.push(PatchOp::replace(
            "/cover_photo",
            serde_json::Value::String(photo_id.to_string()),
        )),
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn listing_request_serializes_with_exact_field_names() {
        let req = ListingRequest {
            name: "$20.00 Acme Credits".into(),
            description: "20 USD of Acme platform credits".into(),
            price: 1950,
            tags: vec![
                "balance: 2000".into(),
                "currency: USD".into(),
                "type: giftcard".into(),
            ],
            platform: "unknown".into(),
            category: category::GIFTCARD.into(),
            kind: kind::ITEM.into(),
            digital: true,
            digital_region: "none".into(),
            digital_deliverable: digital_deliverable::CODE.into(),
            expire_in_days: expire_in_days::SEVEN,
            shipping_within_days: SHIPPING_WITHIN_DAYS_AUTO,
            accept_currency: None,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["name"], "$20.00 Acme Credits");
        assert_eq!(v["price"], 1950);
        assert_eq!(v["category"], "GIFTCARD");
        assert_eq!(v["kind"], "item");
        assert_eq!(v["digital"], true);
        assert_eq!(v["digital_deliverable"], "code");
        assert_eq!(v["shipping_within_days"], 0);
        // accept_currency omitted when None (so Gameflip applies its USD default).
        assert!(v.get("accept_currency").is_none());
        // tags preserved in order.
        assert_eq!(v["tags"][0], "balance: 2000");
    }

    #[test]
    fn accept_currency_is_included_when_set() {
        let mut req = sample_request();
        req.accept_currency = Some(accept_currency::BOTH.into());
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["accept_currency"], "BOTH");
    }

    #[test]
    fn status_patch_matches_reference_shape() {
        let ops = status_patch(status::ONSALE);
        let v = serde_json::to_value(&ops).unwrap();
        assert_eq!(
            v,
            json!([{"op":"replace","path":"/status","value":"onsale"}])
        );
    }

    #[test]
    fn photo_patch_with_display_order() {
        let ops = photo_activation_patch("PHOTOID", Some(0));
        let v = serde_json::to_value(&ops).unwrap();
        assert_eq!(
            v,
            json!([
                {"op":"replace","path":"/photo/PHOTOID/status","value":"active"},
                {"op":"replace","path":"/photo/PHOTOID/display_order","value":0}
            ])
        );
    }

    #[test]
    fn photo_patch_without_order_sets_cover() {
        let ops = photo_activation_patch("PHOTOID", None);
        let v = serde_json::to_value(&ops).unwrap();
        assert_eq!(
            v,
            json!([
                {"op":"replace","path":"/photo/PHOTOID/status","value":"active"},
                {"op":"replace","path":"/cover_photo","value":"PHOTOID"}
            ])
        );
    }

    fn sample_request() -> ListingRequest {
        ListingRequest {
            name: "x".into(),
            description: "y".into(),
            price: 100,
            tags: vec![],
            platform: "unknown".into(),
            category: category::GIFTCARD.into(),
            kind: kind::ITEM.into(),
            digital: true,
            digital_region: "none".into(),
            digital_deliverable: digital_deliverable::CODE.into(),
            expire_in_days: expire_in_days::SEVEN,
            shipping_within_days: SHIPPING_WITHIN_DAYS_AUTO,
            accept_currency: None,
        }
    }
}
