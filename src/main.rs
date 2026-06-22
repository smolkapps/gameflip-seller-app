//! `gfseller` — command-line front-end for the Gameflip seller library.
//!
//! Subcommands:
//! - `catalog` — print the preset credit-pack catalog (no network, no creds).
//! - `post` — post the preset catalog for sale. Defaults to a safe dry run;
//!   pass `--live` to actually create listings on Gameflip.
//!
//! Credentials are read from `GFAPI_KEY` and `GFAPI_SECRET` (env), matching the
//! reference Gameflip samples. Per-pack digital codes are read from a codes file
//! (`--codes`), one `credits=CODE` line per pack; without it, `post` runs in
//! dry-run with placeholder codes only.

use std::collections::HashMap;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use gameflip_seller::auth::Credentials;
use gameflip_seller::catalog::{format_usd, CatalogConfig, CreditPack, PRESET_PACKS};
use gameflip_seller::client::{GameflipClient, ReqwestTransport};
use gameflip_seller::dryrun::DryRunTransport;
use gameflip_seller::model;

#[derive(Parser)]
#[command(
    name = "gfseller",
    version,
    about = "Post a preset catalog of Gameflip credit-pack listings (10/20/50/100)."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Brand/store name used in listing titles and descriptions.
    #[arg(long, global = true, default_value = "Store")]
    brand: String,

    /// Override price as cents-per-credit for every pack (e.g. 100 = $1/credit).
    #[arg(long, global = true)]
    price_per_credit_cents: Option<u64>,

    /// Currency to accept: USD (default), FLP, or BOTH.
    #[arg(long, global = true)]
    accept_currency: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Print the preset catalog as it would be listed (no credentials needed).
    Catalog,

    /// Post the preset catalog for sale (dry-run unless --live is given).
    Post {
        /// Actually send requests to Gameflip. Without this flag, runs a dry run.
        #[arg(long)]
        live: bool,

        /// Final listing status to set: `ready` (private, ready to list) or
        /// `onsale` (published). Defaults to `onsale`.
        #[arg(long, default_value = "onsale")]
        status: String,

        /// Path to a codes file: one `CREDITS=CODE` line per pack
        /// (e.g. `10=ABC-123`). Required for --live.
        #[arg(long)]
        codes: Option<String>,
    },
}

fn build_config(cli: &Cli) -> CatalogConfig {
    CatalogConfig {
        brand: cli.brand.clone(),
        price_per_credit_cents: cli.price_per_credit_cents,
        accept_currency: cli.accept_currency.clone(),
        ..CatalogConfig::default()
    }
}

/// Parse a codes file into `credits -> code`.
fn parse_codes(path: &str) -> Result<HashMap<u32, String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
    let mut map = HashMap::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| format!("{path}:{}: expected CREDITS=CODE", lineno + 1))?;
        let credits: u32 = k
            .trim()
            .parse()
            .map_err(|_| format!("{path}:{}: bad credit count '{k}'", lineno + 1))?;
        map.insert(credits, v.trim().to_string());
    }
    Ok(map)
}

fn cmd_catalog(cfg: &CatalogConfig) {
    println!(
        "Preset Gameflip credit-pack catalog ({} packs):\n",
        PRESET_PACKS.len()
    );
    for pack in PRESET_PACKS.iter() {
        let req = pack.to_listing_request(cfg);
        println!("- {} credits", pack.credits);
        println!("    title:    {}", req.name);
        println!(
            "    price:    {} ({} cents)",
            format_usd(req.price),
            req.price
        );
        println!("    category: {} / digital={}", req.category, req.digital);
        println!("    tags:     {}", req.tags.join(", "));
    }
}

fn code_for(pack: &CreditPack, codes: &HashMap<u32, String>) -> String {
    codes
        .get(&pack.credits)
        .cloned()
        .unwrap_or_else(|| format!("PLACEHOLDER-{}-CREDITS-CODE", pack.credits))
}

fn cmd_post(
    cfg: &CatalogConfig,
    live: bool,
    status: &str,
    codes_path: Option<&str>,
) -> Result<(), String> {
    // Validate the requested final status up front.
    if status != model::status::READY && status != model::status::ONSALE {
        return Err(format!(
            "invalid --status '{status}'; use '{}' or '{}'",
            model::status::READY,
            model::status::ONSALE
        ));
    }

    let codes = match codes_path {
        Some(p) => parse_codes(p)?,
        None => HashMap::new(),
    };

    if live {
        // Live mode requires real credentials and real codes for every pack.
        let key = std::env::var("GFAPI_KEY")
            .map_err(|_| "GFAPI_KEY env var is required for --live".to_string())?;
        let secret = std::env::var("GFAPI_SECRET")
            .map_err(|_| "GFAPI_SECRET env var is required for --live".to_string())?;
        let creds = Credentials::new(key, &secret).map_err(|e| e.to_string())?;

        for pack in PRESET_PACKS.iter() {
            if !codes.contains_key(&pack.credits) {
                return Err(format!(
                    "--live requires a code for every pack; missing {} (provide via --codes)",
                    pack.credits
                ));
            }
        }

        let client =
            GameflipClient::new(creds, ReqwestTransport::new().map_err(|e| e.to_string())?);
        println!(
            "Posting {} listings LIVE to Gameflip (status={status})...",
            PRESET_PACKS.len()
        );
        for pack in PRESET_PACKS.iter() {
            let listing = pack.to_listing_request(cfg);
            let code = code_for(pack, &codes);
            match client.publish_listing(&listing, &code, status) {
                Ok(id) => println!("  OK  {:>4} credits -> {id}", pack.credits),
                Err(e) => {
                    eprintln!("  ERR {:>4} credits -> {e}", pack.credits);
                    return Err(format!(
                        "aborting after failure on {} credits",
                        pack.credits
                    ));
                }
            }
        }
        println!("Done.");
    } else {
        // Dry run: requires no credentials. Use placeholder creds + the
        // non-networking transport so the seller can preview every request.
        let creds = Credentials::new("dry-run-key", "JBSWY3DPEHPK3PXP")
            .expect("static dry-run credentials are valid");
        let client = GameflipClient::new(creds, DryRunTransport::new(true));
        println!(
            "DRY RUN: previewing {} listings (status={status}). No requests are sent.\n\
             Pass --live with GFAPI_KEY/GFAPI_SECRET and --codes to post for real.\n",
            PRESET_PACKS.len()
        );
        for pack in PRESET_PACKS.iter() {
            let listing = pack.to_listing_request(cfg);
            let code = code_for(pack, &codes);
            let id = client
                .publish_listing(&listing, &code, status)
                .map_err(|e| e.to_string())?;
            println!(
                "  (dry-run) would publish {} credits as {id}\n",
                pack.credits
            );
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let cfg = build_config(&cli);

    let result = match &cli.command {
        Command::Catalog => {
            cmd_catalog(&cfg);
            Ok(())
        }
        Command::Post {
            live,
            status,
            codes,
        } => cmd_post(&cfg, *live, status, codes.as_deref()),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
