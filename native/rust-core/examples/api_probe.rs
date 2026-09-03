//! Desktop probe for the live Audible API — the fast loop.
//!
//! Iterating through the Android app costs a Rust build, a Gradle build, an install and
//! a UI drive per attempt. This runs the same client code against the same account in a
//! second, and prints the response *shape* rather than a Kotlin error string.
//!
//! The account comes from a JSON file exported from the device DB. It contains live
//! credentials: keep it outside the repo and delete it afterwards.
//!
//!   cargo run --example api_probe -- --account /path/account.json customer
//!   cargo run --example api_probe -- --account /path/account.json license --asin B002VA9SWS
//!   cargo run --example api_probe -- --account /path/account.json license --asin B002VA9SWS --variant app
//!
//! `license` issues ONE licence request per run. Audible counts them; do not loop it.
//!
//! `--http1` forces HTTP/1.1 and `--raw` prints an equivalent curl command, so the same
//! request can be replayed through a different TLS/HTTP stack. If Audible were
//! fingerprinting the client stack rather than the credentials, those would diverge.

use rust_core::api::auth::{
    exchange_authorization_code_as, generate_authorization_url, Account, AccessToken,
    DeviceProfile, Identity, Locale, OAuthState, PkceChallenge,
};
use rust_core::api::client::AudibleClient;
use rust_core::api::content::{DownloadQuality, DrmType};
use rust_core::api::license::{request_codecs, LicenseRequest, SupportedMediaFeatures};
use serde_json::Value;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn has(flag: &str) -> bool {
    std::env::args().any(|a| a == flag)
}

/// Field names only — values in these responses are personal.
fn keys(value: &Value) -> String {
    value
        .as_object()
        .map(|o| o.keys().cloned().collect::<Vec<_>>().join(", "))
        .unwrap_or_else(|| "<not an object>".into())
}

/// Audible embeds the customer id in denial messages; strip it like upstream does.
fn redact(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(rel) = rest[open..].find(']') else { break };
        let close = open + rel;
        let inner = &rest[open + 1..close];
        let is_customer = inner.len() >= 11
            && inner.starts_with('A')
            && inner[1..].chars().all(|c| c.is_ascii_alphanumeric());
        out.push_str(&rest[..open]);
        out.push_str(if is_customer { "[#redacted#]" } else { &rest[open..=close] });
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Register a new device against a real Amazon login, with a chosen device profile.
///
/// This is the controlled experiment for the 2026-09 licence denials: register twice,
/// varying only what the device claims to be, and compare licence outcomes. Every
/// third-party client currently registers as an Android emulator on a year-old app
/// build, and every third-party client is currently denied.
///
/// Each run registers a real device on the account — deregister spares at
/// audible.com → Account Details → Registered Devices.
async fn register(profile_name: &str, out_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let profile = match profile_name {
        // What we and Libation have always claimed: an emulator, stale app build.
        "emulator" => DeviceProfile {
            manufacturer: "Google".into(),
            model: "sdk_gphone64_x86_64".into(),
            product: "sdk_phone64_x86_64".into(),
            os_version: "google/sdk_gphone64_x86_64/emu64xa:14/UPB5.230623.003/10615560:userdebug/dev-keys".into(),
            os_version_number: "34".into(),
            app_version: "2090253826".into(),
            software_version: "130050002".into(),
        },
        // A real handset on the current app build.
        "real" => DeviceProfile::default(),
        other => return Err(format!("--profile must be real or emulator, got {other}").into()),
    };

    // Two steps, no stdin: the login happens in a browser, and the code comes back on a
    // later invocation. The serial and PKCE verifier must survive in between — they are
    // what the exchange is bound to, and a lost verifier makes the code unusable.
    let state_path = format!("{out_path}.pending.json");

    // Pull the code out of whatever form it arrives in. An unquoted redirect URL gets
    // chopped by the shell at the first `&`, so fall back to a substring search rather
    // than requiring a well-formed URL.
    let supplied_code = arg("--code").or_else(|| {
        arg("--redirect").and_then(|r| {
            url::Url::parse(&r)
                .ok()
                .and_then(|u| {
                    u.query_pairs()
                        .find(|(k, _)| k == "openid.oa2.authorization_code")
                        .map(|(_, v)| v.to_string())
                })
                .or_else(|| {
                    r.find("authorization_code=").map(|i| {
                        r[i + "authorization_code=".len()..]
                            .split('&')
                            .next()
                            .unwrap_or_default()
                            .to_string()
                    })
                })
        })
    });

    // Asking to exchange and failing to find a code must NOT fall through to step 1:
    // that would mint a new verifier and silently invalidate the code just obtained.
    if supplied_code.as_deref().is_none_or(str::is_empty)
        && (arg("--code").is_some() || arg("--redirect").is_some())
    {
        return Err("no authorization code found in --code/--redirect. \
                    Quote the URL ('…') or pass just the openid.oa2.authorization_code value. \
                    The pending registration has been left untouched."
            .into());
    }

    match supplied_code {
        // Step 2: exchange.
        Some(code) => {
            let pending: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&state_path).map_err(|_| {
                    format!("no pending registration at {state_path}; run without --code first")
                })?)?;
            let serial = pending["serial"].as_str().ok_or("pending state has no serial")?;
            let verifier = pending["verifier"].as_str().ok_or("pending state has no verifier")?;

            let locale = Locale::us();
            let pkce = PkceChallenge {
                verifier: verifier.to_string(),
                challenge: String::new(),
                method: "S256".to_string(),
            };

            println!("→ POST /auth/register  [profile: {profile_name}, serial {}…]", &serial[..8]);
            let registration =
                exchange_authorization_code_as(&locale, &code, serial, &pkce, &profile).await?;

            let expires_at = chrono::Utc::now()
                + chrono::Duration::seconds(registration.bearer.expires_in.parse().unwrap_or(3600));
            let identity = Identity::new(
                AccessToken { token: registration.bearer.access_token.clone(), expires_at },
                registration.bearer.refresh_token.clone(),
                registration.mac_dms.device_private_key.clone(),
                registration.mac_dms.adp_token.clone(),
                locale.clone(),
            );

            let account = Account {
                account_id: format!("probe-{profile_name}"),
                account_name: format!("probe ({profile_name})"),
                library_scan: true,
                decrypt_key: String::new(),
                identity: Some(identity),
            };

            std::fs::write(out_path, serde_json::to_string_pretty(&account)?)?;
            std::fs::remove_file(&state_path).ok();
            println!("✓ registered as '{profile_name}'");
            println!("  model={} os={}", profile.model, profile.os_version);
            println!("  account written to {out_path} (contains live credentials)");
        }

        // Step 1: mint the URL and remember what the exchange will need.
        None => {
            let locale = Locale::us();
            let serial: String = (0..16)
                .map(|_| format!("{:02X}", rand::random::<u8>()))
                .collect();
            let pkce = PkceChallenge::generate()?;
            let state = OAuthState::generate();
            let url = generate_authorization_url(&locale, &serial, &pkce, &state)?;

            std::fs::write(
                &state_path,
                serde_json::to_string_pretty(&serde_json::json!({
                    "serial": serial,
                    "verifier": pkce.verifier,
                    "profile": profile_name,
                }))?,
            )?;

            println!("Sign in here, and let it land on the blank maplanding page:\n");
            println!("{url}\n");
            println!("Then re-run with the code from the redirect URL:");
            println!("  cargo run --example api_probe -- register --profile {profile_name} \\");
            println!("    --out {out_path} --code <openid.oa2.authorization_code value>");
            println!("\n(or pass the whole redirect URL with --redirect '<url>')");
            println!("\npending state: {state_path}");
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let command = std::env::args()
        .nth(1)
        .filter(|a| !a.starts_with("--"))
        .ok_or("command must be one of: register, customer, metadata, license, download")?;

    // `register` mints an account rather than consuming one.
    if command == "register" {
        return register(&arg("--profile").unwrap_or_else(|| "real".into()),
                        &arg("--out").unwrap_or_else(|| "account-new.json".into()))
            .await;
    }

    let account_path = arg("--account").ok_or("--account <path to account.json> is required")?;
    let account: Account = serde_json::from_str(&std::fs::read_to_string(&account_path)?)?;
    let client = AudibleClient::new(account)?;

    match command.as_str() {
        // Signed request (x-adp-token/alg/signature). Read-only, safe to repeat.
        "customer" => {
            println!("→ GET /1.0/customer/information (ADP-signed)");
            match client.get_customer_information().await {
                Ok(info) => println!("✓ granted: name={:?} given_name={:?}", info.name, info.given_name),
                Err(e) => println!("✗ {e}"),
            }
        }

        // Read-only. Does the library API still say this account owns the title?
        //
        // Worth asking, because the licence call's Ownership lane answers
        // "CustomerThrottled" for the same ASIN — if the library says we own it while
        // the licence ownership check will not confirm it, that is a contradiction
        // inside Audible's own API, not an entitlement problem.
        "owns" => {
            let asin = arg("--asin").ok_or("--asin is required")?;
            println!("→ GET /1.0/library/{asin}");
            let raw: Value = client
                .get_with_query(
                    &format!("/1.0/library/{asin}"),
                    &[("response_groups", "product_desc,product_attrs")],
                )
                .await?;
            let item = raw.get("item").unwrap_or(&raw);
            println!("  keys: {}", keys(item));
            for field in ["asin", "title", "is_downloadable", "purchase_date", "status"] {
                if let Some(v) = item.get(field) {
                    println!("  {field:18}{v}");
                }
            }
        }

        // Read-only. Proves whether an ASIN still has chapter/content metadata.
        "metadata" => {
            let asin = arg("--asin").ok_or("--asin is required")?;
            println!("→ GET /1.0/content/{asin}/metadata");
            match client.get_content_metadata(&asin).await {
                Ok(meta) => {
                    let codec = meta
                        .content_reference
                        .as_ref()
                        .and_then(|r| r.codec.clone())
                        .unwrap_or_else(|| "-".into());
                    let chapters = meta
                        .chapter_info
                        .as_ref()
                        .map(|c| c.chapters.len())
                        .unwrap_or(0);
                    println!(
                        "✓ codec={codec:<12} chapters={chapters:<4} content_url={}",
                        meta.content_url.is_some()
                    )
                }
                Err(e) => println!("✗ {e}"),
            }
        }

        // ONE licence request. `--variant app` mirrors what the official Android app
        // sends (captured 2026-09-03): Widevine offered, xHE-AAC offered, spatial, Normal.
        "license" => {
            let asin = arg("--asin").ok_or("--asin is required")?;
            let variant = arg("--variant").unwrap_or_else(|| "ours".into());
            let app_variant = variant == "app";

            let request = if variant == "widevine" {
                // Libation issue 2021: a user reports Widevine downloads still working
                // while the ADRM path is denied. Ask for Widevine alone.
                LicenseRequest {
                    supported_media_features: SupportedMediaFeatures {
                        drm_types: vec![DrmType::Widevine],
                        codecs: vec![
                            request_codecs::AAC_LC.to_string(),
                            request_codecs::XHE_AAC.to_string(),
                        ],
                        chapter_titles_type: rust_core::api::content::ChapterTitlesType::Tree,
                        previews: false,
                        catalog_samples: false,
                    },
                    spatial: false,
                    consumption_type: rust_core::api::license::ConsumptionType::Download,
                    tenant_id: "Audible".to_string(),
                    quality: DownloadQuality::High,
                    response_groups: LicenseRequest::RESPONSE_GROUPS.to_string(),
                }
            } else if app_variant {
                LicenseRequest {
                    supported_media_features: SupportedMediaFeatures {
                        drm_types: vec![DrmType::Adrm, DrmType::Mpeg, DrmType::Widevine],
                        codecs: vec![
                            request_codecs::AAC_LC.to_string(),
                            request_codecs::XHE_AAC.to_string(),
                        ],
                        chapter_titles_type: rust_core::api::content::ChapterTitlesType::Tree,
                        previews: false,
                        catalog_samples: false,
                    },
                    spatial: true,
                    consumption_type: rust_core::api::license::ConsumptionType::Download,
                    tenant_id: "Audible".to_string(),
                    quality: DownloadQuality::Normal,
                    response_groups:
                        "content_reference,chapter_info,pdf_url,ad_insertion,narration_speed"
                            .to_string(),
                }
            } else {
                LicenseRequest::download(DownloadQuality::High, DrmType::Adrm)
            };

            // --prime replays the two non-UI calls the official app makes immediately
            // before its (granted) licence request: the product detail screen for this
            // ASIN, then customer/status. If Audible requires a recent ASIN-scoped view
            // to release a licence, this is where it would show up.
            if has("--prime") {
                for path in [
                    format!("/1.0/screens/audible-android-detail-v2/{asin}"),
                    "/1.0/customer/status".to_string(),
                ] {
                    let outcome: Result<Value, _> = client.get(&path).await;
                    println!(
                        "  primed {path} → {}",
                        match &outcome {
                            Ok(_) => "ok".to_string(),
                            Err(e) => format!("{e}"),
                        }
                    );
                }
            }

            println!(
                "→ POST /1.0/content/{asin}/licenserequest  [variant: {}{}]",
                variant,
                if has("--prime") { ", primed" } else { "" }
            );
            println!("  body: {}", serde_json::to_string(&request)?);

            if has("--raw") {
                // Same body, different HTTP/TLS stack. The token is deliberately left as
                // a shell variable so the command can be pasted without leaking it.
                println!(
                    "\n  replay with curl (export AUDIBLE_TOKEN first):\n  \
                     curl -sS -X POST 'https://api.audible.com/1.0/content/{asin}/licenserequest' \\\n    \
                     -H \"x-amz-access-token: $AUDIBLE_TOKEN\" \\\n    \
                     -H 'x-device-type-id: A10KISP2GWF0E4' -H 'content-type: application/json' \\\n    \
                     -d '{}'\n",
                    serde_json::to_string(&request)?
                );
            }

            let raw: Value = client
                .post_once(&format!("/1.0/content/{asin}/licenserequest"), &request)
                .await?;

            let license = raw.get("content_license").unwrap_or(&raw);
            println!("\n  root keys:            {}", keys(&raw));
            println!("  content_license keys: {}", keys(license));
            for field in ["status_code", "granted_right", "drm_type", "license_response_type"] {
                if let Some(v) = license.get(field) {
                    println!("  {field:22}{v}");
                }
            }
            if let Some(message) = license.get("message").and_then(|m| m.as_str()) {
                println!("  message:              {}", redact(message));
            }
            if let Some(meta) = license.get("content_metadata") {
                println!("  content_metadata:     {}", keys(meta));
                if let Some(cr) = meta.get("content_reference") {
                    println!(
                        "    codec={} format={}",
                        cr.get("codec").unwrap_or(&Value::Null),
                        cr.get("content_format").unwrap_or(&Value::Null)
                    );
                }
                println!(
                    "    has content_url:    {}",
                    meta.get("content_url").is_some()
                );
            }
            if let Some(reasons) = license.get("license_denial_reasons").and_then(|r| r.as_array()) {
                println!("  denial reasons:");
                for reason in reasons {
                    println!(
                        "    validationType={} rejectionReason={} message={}",
                        reason.get("validationType").unwrap_or(&Value::Null),
                        reason.get("rejectionReason").unwrap_or(&Value::Null),
                        redact(reason.get("message").and_then(|m| m.as_str()).unwrap_or(""))
                    );
                }
            }
        }

        // The download half of the loop: ask for a licence and, if granted, HEAD the
        // offline URL to prove the asset is actually fetchable with these credentials.
        // Nothing here writes audio to disk — it is a reachability check, not a ripper.
        "download" => {
            let asin = arg("--asin").ok_or("--asin is required")?;
            println!("→ licence + reachability check for {asin}");

            match client
                .build_download_license(&asin, DownloadQuality::High, false)
                .await
            {
                Ok(license) => {
                    println!("✓ licence granted: drm_type={:?}", license.drm_type);
                    println!(
                        "  keys: {} decryption key(s)",
                        license.decryption_keys.as_ref().map(|k| k.len()).unwrap_or(0)
                    );
                    let head = reqwest::Client::new()
                        .head(&license.download_url)
                        .send()
                        .await?;
                    println!(
                        "  asset: HTTP {} content-length={:?} content-type={:?}",
                        head.status(),
                        head.headers().get("content-length"),
                        head.headers().get("content-type")
                    );
                }
                Err(e) => println!("✗ {e}"),
            }
        }

        // Activation bytes: the 4-byte key that decrypts classic AAX. Signed request.
        "activation" => {
            println!("→ GET /license/token?action=register (ADP-signed)");
            let (adp, key, locale) = {
                let account: Account =
                    serde_json::from_str(&std::fs::read_to_string(&account_path)?)?;
                let id = account.identity.clone().ok_or("account is not signed in")?;
                (id.adp_token.clone(), id.device_private_key.clone(), id.locale.clone())
            };
            match rust_core::api::auth::get_activation_bytes(&locale, &adp, &key).await {
                Ok(bytes) => println!("✓ activation bytes: {} chars", bytes.len()),
                Err(e) => println!("✗ {e}"),
            }
        }

        // The AAX workaround: a completely different service from `licenserequest`.
        // Upstream falls back to this when the licence call throws, but NOT when it
        // returns 200 with status_code "Denied" — which is exactly what the current
        // throttle produces, so the fallback never fires for anyone hitting it.
        // Reference: AudibleApi/Api.Download.cs:334-368 (DownloadAaxWorkaroundAsync).
        "aax" => {
            let asin = arg("--asin").ok_or("--asin is required")?;

            // 1. which codecs does this title actually have?
            let item: Value = client
                .get_with_query(
                    &format!("/1.0/library/{asin}"),
                    &[("response_groups", "product_attrs,relationships")],
                )
                .await?;
            let codecs: Vec<String> = item
                .get("item")
                .unwrap_or(&item)
                .get("available_codecs")
                .and_then(|c| c.as_array())
                .map(|list| {
                    list.iter()
                        .filter_map(|c| c.get("enhanced_codec").and_then(|e| e.as_str()))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            println!("  available codecs: {}", codecs.join(", "));

            // Upstream's preference order (Api.Download.cs:366).
            const PREFERRED: [&str; 5] = [
                "LC_128_44100_stereo", "LC_64_44100_stereo", "LC_64_22050_stereo",
                "LC_32_22050_stereo", "AAX",
            ];
            let codec = PREFERRED
                .iter()
                .find(|p| codecs.iter().any(|c| c.eq_ignore_ascii_case(p)))
                .map(|s| s.to_string())
                .or_else(|| codecs.first().cloned())
                .ok_or("title reports no codecs")?;
            println!("  chosen codec:     {codec}");

            // 2. signed GET to the CDE service; the answer is a 302, not a body.
            let path = format!(
                "/FionaCDEServiceEngine/FSDownloadContent?type=AUDI&currentTransportMethod=WIFI&key={asin}&codec={codec}"
            );
            let (adp, key) = {
                let account: Account =
                    serde_json::from_str(&std::fs::read_to_string(&account_path)?)?;
                let id = account.identity.clone().ok_or("account is not signed in")?;
                (id.adp_token.clone(), id.device_private_key.clone())
            };
            let signature = rust_core::api::signing::sign_request(
                "GET", &path, "", &adp, &key, chrono::Utc::now(),
            )?;

            println!("→ GET https://cde-ta-g7g.amazon.com{}", path.split('?').next().unwrap_or(""));
            let redirectless = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()?;
            let mut request = redirectless.get(format!("https://cde-ta-g7g.amazon.com{path}"));
            for (name, value) in signature.headers() {
                request = request.header(name, value);
            }
            let response = request.send().await?;
            let status = response.status();
            let location = response
                .headers()
                .get("location")
                .and_then(|l| l.to_str().ok())
                .map(str::to_string);

            println!("  HTTP {status}");
            match location {
                Some(url) => {
                    // The URL is signed and personal: report only its shape.
                    let host = url.split('/').nth(2).unwrap_or("?");
                    println!("  ✓ redirected to {host} ({} chars)", url.len());
                    let head = reqwest::Client::new().head(&url).send().await?;
                    println!(
                        "  asset: HTTP {} content-length={:?} content-type={:?}",
                        head.status(),
                        head.headers().get("content-length"),
                        head.headers().get("content-type")
                    );
                }
                None => {
                    let body = response.text().await.unwrap_or_default();
                    println!("  no Location header; body starts: {}", &body[..body.len().min(200)]);
                }
            }
        }

        other => return Err(format!("unknown command: {other}").into()),
    }

    if has("--help") {
        println!("commands: customer | metadata --asin X | license --asin X [--variant app]");
    }
    Ok(())
}
