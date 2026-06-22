//! The preset credit-pack catalog.
//!
//! Per the project brief, the seller tool ships with a fixed catalog of
//! credit-pack listings (10 / 20 / 50 / 100 credits) and posts them for sale.
//! Each pack is rendered into a [`ListingRequest`] that is wire-compatible with
//! the Gameflip API. Titles/items are intentionally simple here; generalizing
//! to arbitrary items is deferred (see [`CatalogConfig`] for the seam).

use crate::model::{
    category, digital_deliverable, expire_in_days, kind, ListingRequest, SHIPPING_WITHIN_DAYS_AUTO,
};

/// One entry of the preset catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditPack {
    /// Number of credits in the pack (10, 20, 50, 100).
    pub credits: u32,
    /// Sale price in **cents** USD.
    pub price_cents: u64,
}

/// The four preset packs. Prices are a sensible default the seller can override
/// via [`CatalogConfig`]; they deliberately undercut face value slightly, the
/// usual marketplace pattern (the listing title still shows full value so the
/// discount renders on Gameflip).
pub const PRESET_PACKS: [CreditPack; 4] = [
    CreditPack {
        credits: 10,
        price_cents: 950,
    },
    CreditPack {
        credits: 20,
        price_cents: 1900,
    },
    CreditPack {
        credits: 50,
        price_cents: 4750,
    },
    CreditPack {
        credits: 100,
        price_cents: 9500,
    },
];

/// Seller-tunable knobs applied to every pack in the catalog. This is the seam
/// for later generalization: today it carries branding + delivery settings;
/// later it can grow per-item overrides without changing call sites.
#[derive(Debug, Clone)]
pub struct CatalogConfig {
    /// Brand/product name woven into titles & descriptions, e.g. "Acme".
    pub brand: String,
    /// Gameflip `platform` value. Credit packs have no console platform, so the
    /// reference samples use a neutral value.
    pub platform: String,
    /// Region restriction; `"none"` = unrestricted.
    pub digital_region: String,
    /// Listing expiry window in days.
    pub expire_in_days: u32,
    /// Currency accepted; `None` lets Gameflip apply its USD default.
    pub accept_currency: Option<String>,
    /// Cents-per-credit used to derive each pack's price, overriding
    /// [`PRESET_PACKS`] prices when set. `None` keeps the preset prices.
    pub price_per_credit_cents: Option<u64>,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        CatalogConfig {
            brand: "Store".to_string(),
            // Gameflip uses lowercase platform slugs; "unknown" is the neutral
            // choice for non-console digital goods.
            platform: "unknown".to_string(),
            digital_region: "none".to_string(),
            expire_in_days: expire_in_days::SEVEN,
            accept_currency: None,
            price_per_credit_cents: None,
        }
    }
}

impl CatalogConfig {
    /// Convenience constructor setting only the brand.
    pub fn with_brand(brand: impl Into<String>) -> Self {
        CatalogConfig {
            brand: brand.into(),
            ..Default::default()
        }
    }
}

/// Format a cents value as a `$X.YY` string.
pub fn format_usd(cents: u64) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}

impl CreditPack {
    /// Effective price in cents given a config (applies `price_per_credit_cents`
    /// override if present).
    pub fn effective_price_cents(&self, cfg: &CatalogConfig) -> u64 {
        match cfg.price_per_credit_cents {
            Some(per) => per * self.credits as u64,
            None => self.price_cents,
        }
    }

    /// Listing title. Includes the full credit count and dollar value so the
    /// Gameflip UI can show the discount vs. face value.
    pub fn title(&self, cfg: &CatalogConfig) -> String {
        format!(
            "{} {} Credits ({})",
            cfg.brand,
            self.credits,
            format_usd(self.effective_price_cents(cfg))
        )
    }

    /// Listing description.
    pub fn description(&self, cfg: &CatalogConfig) -> String {
        format!(
            "{credits} {brand} credits delivered as a digital code. \
Instant auto-delivery after purchase. One code per order.",
            credits = self.credits,
            brand = cfg.brand
        )
    }

    /// Search/filter tags. Gameflip relies on `"key: value"` tags for filtering;
    /// we tag the balance (credit count), currency, and item type.
    pub fn tags(&self, _cfg: &CatalogConfig) -> Vec<String> {
        vec![
            format!("balance: {}", self.credits),
            "currency: USD".to_string(),
            "type: giftcard".to_string(),
            "type: credits".to_string(),
        ]
    }

    /// Render this pack into a Gameflip [`ListingRequest`].
    pub fn to_listing_request(&self, cfg: &CatalogConfig) -> ListingRequest {
        ListingRequest {
            name: self.title(cfg),
            description: self.description(cfg),
            price: self.effective_price_cents(cfg),
            tags: self.tags(cfg),
            platform: cfg.platform.clone(),
            category: category::GIFTCARD.to_string(),
            kind: kind::ITEM.to_string(),
            digital: true,
            digital_region: cfg.digital_region.clone(),
            digital_deliverable: digital_deliverable::CODE.to_string(),
            expire_in_days: cfg.expire_in_days,
            shipping_within_days: SHIPPING_WITHIN_DAYS_AUTO,
            accept_currency: cfg.accept_currency.clone(),
        }
    }
}

/// Render the full preset catalog into listing requests using `cfg`.
pub fn build_preset_listings(cfg: &CatalogConfig) -> Vec<ListingRequest> {
    PRESET_PACKS
        .iter()
        .map(|p| p.to_listing_request(cfg))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_has_the_four_expected_packs() {
        let credits: Vec<u32> = PRESET_PACKS.iter().map(|p| p.credits).collect();
        assert_eq!(credits, vec![10, 20, 50, 100]);
    }

    #[test]
    fn format_usd_handles_cents_and_dollars() {
        assert_eq!(format_usd(950), "$9.50");
        assert_eq!(format_usd(9500), "$95.00");
        assert_eq!(format_usd(5), "$0.05");
        assert_eq!(format_usd(0), "$0.00");
    }

    #[test]
    fn title_and_description_include_brand_and_credits() {
        let cfg = CatalogConfig::with_brand("Acme");
        let pack = PRESET_PACKS[1]; // 20 credits
        let title = pack.title(&cfg);
        assert!(title.contains("Acme"));
        assert!(title.contains("20"));
        assert!(title.contains("$19.00"));
        let desc = pack.description(&cfg);
        assert!(desc.contains("20 Acme credits"));
    }

    #[test]
    fn price_per_credit_override_recomputes_all_prices() {
        let cfg = CatalogConfig {
            price_per_credit_cents: Some(100), // $1.00 per credit
            ..CatalogConfig::with_brand("Acme")
        };
        assert_eq!(PRESET_PACKS[0].effective_price_cents(&cfg), 1000); // 10 * 100
        assert_eq!(PRESET_PACKS[3].effective_price_cents(&cfg), 10000); // 100 * 100
    }

    #[test]
    fn default_prices_are_used_without_override() {
        let cfg = CatalogConfig::default();
        assert_eq!(PRESET_PACKS[2].effective_price_cents(&cfg), 4750); // 50 credits preset
    }

    #[test]
    fn to_listing_request_is_a_digital_giftcard_with_auto_delivery() {
        let cfg = CatalogConfig::with_brand("Acme");
        let req = PRESET_PACKS[3].to_listing_request(&cfg); // 100 credits
        assert_eq!(req.category, category::GIFTCARD);
        assert_eq!(req.kind, kind::ITEM);
        assert!(req.digital);
        assert_eq!(req.digital_deliverable, digital_deliverable::CODE);
        assert_eq!(req.shipping_within_days, SHIPPING_WITHIN_DAYS_AUTO);
        assert_eq!(req.price, 9500);
        // tags include the balance.
        assert!(req.tags.iter().any(|t| t == "balance: 100"));
    }

    #[test]
    fn build_preset_listings_returns_four() {
        let listings = build_preset_listings(&CatalogConfig::with_brand("Acme"));
        assert_eq!(listings.len(), 4);
        let prices: Vec<u64> = listings.iter().map(|l| l.price).collect();
        assert_eq!(prices, vec![950, 1900, 4750, 9500]);
    }

    #[test]
    fn accept_currency_flows_through_when_set() {
        let cfg = CatalogConfig {
            accept_currency: Some(crate::model::accept_currency::BOTH.to_string()),
            ..CatalogConfig::with_brand("Acme")
        };
        let req = PRESET_PACKS[0].to_listing_request(&cfg);
        assert_eq!(req.accept_currency.as_deref(), Some("BOTH"));
    }
}
