use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use std::{collections::HashMap, time::Duration};

const PUBLIC_KEY: &str = include_str!("../updater.pub");
const PLATFORM: &str = "windows-x86_64";
const FEED_TIMEOUT: Duration = Duration::from_secs(30);
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

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: Option<String>,
    pub released_at: Option<String>,
    pub older: bool,
    url: String,
    signature: String,
}

fn offered(version: &semver::Version, current: &semver::Version, switching: bool) -> bool {
    version > current || (switching && version != current)
}

pub fn client(current: &str) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("battleplan/{current}"))
        .connect_timeout(FEED_TIMEOUT)
        .read_timeout(STALL_TIMEOUT)
        .build()
        .map_err(|error| format!("The update client couldn’t start: {error}"))
}

pub async fn offer(
    client: &reqwest::Client,
    feed: &str,
    current: &semver::Version,
    switching: bool,
) -> Result<Option<Release>, String> {
    let response = client
        .get(feed)
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(FEED_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("it couldn’t be reached: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("it answered HTTP {status}"));
    }
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("its answer was cut off: {error}"))?;
    let manifest: Manifest =
        serde_json::from_slice(&body).map_err(|error| format!("it isn’t a release: {error}"))?;
    let version = semver::Version::parse(&manifest.version)
        .map_err(|error| format!("its version {} isn’t valid: {error}", manifest.version))?;
    if !offered(&version, current, switching) {
        return Ok(None);
    }
    let platform = manifest
        .platforms
        .get(PLATFORM)
        .ok_or_else(|| format!("its release {version} has no Windows build"))?;
    Ok(Some(Release {
        older: version < *current,
        version: manifest.version,
        notes: manifest.notes,
        released_at: manifest.pub_date,
        url: platform.url.clone(),
        signature: platform.signature.clone(),
    }))
}

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
        .map_err(|error| format!("the download couldn’t start: {error}"))?;
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
        .ok_or_else(|| format!("its {what} isn’t a Tauri {what}"))
}

fn verify(package: &[u8], signature: &str, version: &str) -> Result<(), String> {
    let key = PublicKey::decode(&minisign_text(PUBLIC_KEY, "public key")?)
        .map_err(|error| format!("this build’s updater key isn’t valid: {error}"))?;
    let signature = Signature::decode(&minisign_text(signature, "signature")?)
        .map_err(|error| format!("its signature isn’t valid: {error}"))?;
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

#[cfg(test)]
mod tests {
    use super::offered;

    #[test]
    fn a_channel_switch_offers_the_chosen_channels_release_even_when_it_is_older() {
        let version = |text: &str| semver::Version::parse(text).unwrap();
        let nightly = version("0.1.2-nightly.20261123.5");
        let stable = version("0.1.1");
        assert!(!offered(&stable, &nightly, false));
        assert!(offered(&stable, &nightly, true));
        assert!(offered(&nightly, &stable, false));
        assert!(offered(&version("0.1.2"), &nightly, false));
        assert!(!offered(&nightly, &nightly, true));
    }
}