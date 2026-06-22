//! `gameflip_seller` — a seller tool for the Gameflip marketplace.
//!
//! The library ships with a preset catalog of credit-pack listings
//! (10 / 20 / 50 / 100 credits) and posts them for sale through the Gameflip
//! REST API. The wire format (auth header, listing fields, JSON-patch status
//! transitions, photo upload) mirrors Gameflip's reference Node bindings.
//!
//! ## Layout
//! - [`auth`] — `GFAPI <key>:<totp>` authorization + RFC 6238 TOTP.
//! - [`model`] — listing request/response types, enum constants, JSON-patch ops.
//! - [`catalog`] — the preset credit-pack catalog (the fixed starting point).
//! - [`client`] — the HTTP client, generic over a [`client::Transport`].
//! - [`dryrun`] — a non-networking transport for safe previews.
//!
//! ## Quick start
//! ```no_run
//! use gameflip_seller::{auth::Credentials, catalog::CatalogConfig,
//!     client::{GameflipClient, ReqwestTransport}, model};
//!
//! let creds = Credentials::new(
//!     std::env::var("GFAPI_KEY").unwrap(),
//!     &std::env::var("GFAPI_SECRET").unwrap(),
//! ).unwrap();
//! let client = GameflipClient::new(creds, ReqwestTransport::new().unwrap());
//!
//! let cfg = CatalogConfig::with_brand("Acme");
//! for (pack, listing) in gameflip_seller::catalog::PRESET_PACKS
//!     .iter()
//!     .zip(gameflip_seller::catalog::build_preset_listings(&cfg))
//! {
//!     let code = format!("ACME-{}-XXXX", pack.credits); // your real codes
//!     let id = client.publish_listing(&listing, &code, model::status::ONSALE).unwrap();
//!     println!("listed {} credits as {id}", pack.credits);
//! }
//! ```

pub mod auth;
pub mod catalog;
pub mod client;
pub mod dryrun;
pub mod error;
pub mod model;

pub use error::{Error, Result};
