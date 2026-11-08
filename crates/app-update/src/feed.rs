//! A channel's update feed in Tauri's updater format, and its installer verified as Tauri's
//! updater verifies one: a minisign signature by the key both apps carry, whose trusted comment
//! names the version the feed offers (Tauri's `requireSignedVersion`).
use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use std::{collections::HashMap, time::Duration};

/// The updater public key; the Tauri CLI signs releases for it (`plugins.updater.pubkey`).
const PUBLIC_KEY: &str = include_str!("../updater.pub");
/// Tauri's name for this build's platform in a feed.
const PLATFORM: &str = "windows-x86_64";
const FEED_TIMEOUT: Duration = Duration::from_secs(30);
/// A download that delivers nothing for this long has stalled.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
struct Manifest {
    version: String,
    notes: Option<String>,
    pub_date: Option<String>,
    platforms: HashMap<String, Platform>,
}

#[derive(Deserialize)]
struct Platform {
    signature: String,
    url: String,
}

/// A release a feed offers.
#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: Option<String>,
    pub released_at: Option<String>,
    url: String,
    signature: String,
}

pub fn client(current: &str) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("battle-plan/{current}"))
        .connect_timeout(FEED_TIMEOUT)
        .read_timeout(STALL_TIMEOUT)
        .build()
        .map_err(|error| format!("The update client could not start: {error}"))
}

/// The feed's release when it is newer than `current`.
pub async fn newer(
    client: &reqwest::Client,
    feed: &str,
    current: &semver::Version,
) -> Result<Option<Release>, String> {
    let response = client
        .get(feed)
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(FEED_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("it could not be reached: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("it answered HTTP {status}"));
    }
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("its answer was cut off: {error}"))?;
    let manifest: Manifest =
        serde_json::from_slice(&body).map_err(|error| format!("it is not a release: {error}"))?;
    let version = semver::Version::parse(&manifest.version)
        .map_err(|error| format!("its version {} is not valid: {error}", manifest.version))?;
    if version <= *current {
        return Ok(None);
    }
    let platform = manifest
        .platforms
        .get(PLATFORM)
        .ok_or_else(|| format!("its release {version} has no Windows build"))?;
    Ok(Some(Release {
        version: manifest.version,
        notes: manifest.notes,
        released_at: manifest.pub_date,
        url: platform.url.clone(),
        signature: platform.signature.clone(),
    }))
}

/// Download `release`'s installer, telling `progress` the bytes so far and the total when known,
/// then verify it.
pub async fn download(
    client: &reqwest::Client,
    release: &Release,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(&release.url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .map_err(|error| format!("the download could not start: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("the download answered HTTP {status}"));
    }
    let total = response.content_length();
    let mut package = Vec::with_capacity(total.unwrap_or(0) as usize);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("the download was cut off: {error}"))?
    {
        package.extend_from_slice(&chunk);
        progress(package.len() as u64, total);
    }
    verify(&package, &release.signature, &release.version)?;
    Ok(package)
}

fn minisign_text(encoded: &str, what: &str) -> Result<String, String> {
    STANDARD
        .decode(encoded.trim())
        .ok()
        .and_then(|text| String::from_utf8(text).ok())
        .ok_or_else(|| format!("its {what} is not a Tauri {what}"))
}

/// Tauri's check: the signature verifies, and its trusted comment binds it to `version`.
fn verify(package: &[u8], signature: &str, version: &str) -> Result<(), String> {
    let key = PublicKey::decode(&minisign_text(PUBLIC_KEY, "public key")?)
        .map_err(|error| format!("this build's updater key is not valid: {error}"))?;
    let signature = Signature::decode(&minisign_text(signature, "signature")?)
        .map_err(|error| format!("its signature is not valid: {error}"))?;
    // Tauri's updater accepts signatures of the whole file as well as of its hash.
    key.verify(package, &signature, true)
        .map_err(|error| format!("it failed signature verification: {error}"))?;
    let signed = signature
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
        .ok_or("its signature names no version, which this updater requires")?;
    let bound = semver::Version::parse(signed)
        .is_ok_and(|signed| semver::Version::parse(version).is_ok_and(|offered| offered == signed));
    if !bound {
        return Err(format!(
            "its signature is for version {signed}, not the {version} its feed offers"
        ));
    }
    Ok(())
}
