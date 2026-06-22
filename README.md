# gameflip-seller-app

A small Rust seller tool for the [Gameflip](https://gameflip.com) marketplace.
It ships with a **preset catalog of credit-pack listings** — 10, 20, 50, and
100 credits — and posts them for sale through the Gameflip REST API. The fixed
catalog is the intentional starting point; customizable titles/items are a
deliberate later step (the `CatalogConfig` type is the seam for that).

It provides both a library (`gameflip_seller`) and a CLI (`gfseller`).

## Why a credit pack is a "digital gift card" on Gameflip

Gameflip sells prepaid value as digital gift-card-style items. Each pack is
therefore created as a listing with `category = GIFTCARD`, `kind = item`,
`digital = true`, `digital_deliverable = code`, and `shipping_within_days = 0`
(auto-delivery of a stored code). This matches Gameflip's own giftcard sample.

## Status: what works without account access

Everything except the final live HTTP calls is implemented and tested offline:

- the `GFAPI <key>:<otp>` authorization scheme, including an RFC 6238 TOTP
  generator verified against the RFC 4226/6238 reference vectors;
- the preset catalog → Gameflip listing JSON mapping;
- the create → store-digital-code → set-status publish flow;
- the photo-upload flow (request URL → PUT bytes → activate via JSON Patch);
- response-envelope parsing (`status: "SUCCESS"` / structured API errors);
- a **dry-run** transport that previews every request without sending it.

The only thing this tool cannot do for you is **be a verified Gameflip seller**:
you must obtain an API key + OTP secret from Gameflip (see below). That is the
single human/account step.

## Install / build

```bash
cargo build --release      # binary at target/release/gfseller
cargo test                 # runs unit + HTTP integration tests (offline)
```

## Credentials

Gameflip issues an **API Key** and **OTP secret** on the
[Settings page](https://gameflip.com/settings) for verified seller accounts.
Export them (never commit them):

```bash
export GFAPI_KEY=your_api_key
export GFAPI_SECRET=your_base32_otp_secret
```

The API key's prefix selects the environment automatically:
`test-…` → test API, `dev-…` → localhost, otherwise production.

## CLI usage

### Preview the catalog (no credentials needed)

```bash
gfseller --brand "Acme" catalog
```

### Dry-run the post flow (default; sends nothing)

```bash
gfseller --brand "Acme" post
```

This prints every HTTP request that *would* be made (with the live OTP redacted)
so you can confirm the catalog before going live.

### Post for real

`--live` requires credentials **and** a real digital code for every pack,
supplied via a `--codes` file (one `CREDITS=CODE` line per pack):

```
# codes.txt  (gitignored)
10=ACME-AAAA-0001
20=ACME-BBBB-0002
50=ACME-CCCC-0003
100=ACME-DDDD-0004
```

```bash
gfseller --brand "Acme" post --live --status onsale --codes codes.txt
```

Use `--status ready` to create the listings in the private "ready" state instead
of publishing them immediately.

### Pricing & currency

```bash
# $1.00 per credit (overrides the preset prices) and accept FLP + USD
gfseller --brand "Acme" --price-per-credit-cents 100 --accept-currency BOTH post
```

## Library usage

```rust
use gameflip_seller::{auth::Credentials, catalog::{CatalogConfig, build_preset_listings},
    client::{GameflipClient, ReqwestTransport}, model};

let creds = Credentials::new(std::env::var("GFAPI_KEY")?, &std::env::var("GFAPI_SECRET")?)?;
let client = GameflipClient::new(creds, ReqwestTransport::new()?);

let cfg = CatalogConfig::with_brand("Acme");
for (pack, listing) in gameflip_seller::catalog::PRESET_PACKS.iter()
    .zip(build_preset_listings(&cfg))
{
    let code = format!("ACME-{}-XXXX", pack.credits); // your real, unique code
    let id = client.publish_listing(&listing, &code, model::status::ONSALE)?;
    println!("listed {} credits as {id}", pack.credits);
}
```

The client is generic over a `Transport`, so logic is fully unit-testable
against an in-memory mock and the same code path drives `--dry-run`.

## Gameflip API mapping (reference)

| Step | HTTP | Endpoint | Notes |
| --- | --- | --- | --- |
| Auth | header | `Authorization: GFAPI <key>:<totp>` | TOTP: base32 secret, SHA1, 6 digits, 30 s |
| Create listing | `POST` | `/api/v1/listing` | JSON body; returns listing `id` |
| Store code | `PUT` | `/api/v1/listing/{id}/digital_goods` | `{"code": "..."}`, unique per listing |
| Add photo | `POST` | `/api/v1/listing/{id}/photo` | returns `{id, upload_url}` |
| Upload photo | `PUT` | `<upload_url>` | image bytes, `Content-Type: image/png|jpeg`, no auth header |
| Activate photo | `PATCH` | `/api/v1/listing/{id}` | JSON-Patch on `/photo/{pid}/status` + cover/order |
| Set status | `PATCH` | `/api/v1/listing/{id}` | `Content-Type: application/json-patch+json`, `replace /status` |

Base URL: `https://production-gameflip.fingershock.com/api/v1`.

## Safety notes

- `post` is **dry-run by default**; real listings require an explicit `--live`.
- Credentials and code files are gitignored; the dry-run log redacts the OTP.
- Listing prices are in **cents**; titles include the dollar value so Gameflip
  renders the discount versus face value.

## License

MIT — see [LICENSE](LICENSE).
