use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};

const PUBLIC_KEY: &str = include_str!("../updater.pub");
const PLATFORM: &str = "windows-x86_64";
const FEED_TIMEOUT: Duration = Duration::from_secs(30);
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    releases: Vec<ReleaseNotes>,
    platforms: HashMap<String, Platform>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Highlight {
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Pull {
    pub number: u64,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReleaseNotes {
    pub version: String,
    pub since: Option<String>,
    pub date: Option<String>,
    pub changes: u32,
    pub highlights: Option<Vec<Highlight>>,
    pub more: Option<u32>,
    pub pulls: Vec<Pull>,
}

#[derive(Deserialize)]
struct Platform {
    signature: String,
    url: String,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub releases: Vec<ReleaseNotes>,
    pub unlisted: bool,
    pub older: bool,
    url: String,
    signature: String,
}

fn offered(version: &semver::Version, current: &semver::Version, switching: bool) -> bool {
    version > current || (switching && version != current)
}

fn listed(
    history: Vec<ReleaseNotes>,
    offered: &semver::Version,
    current: &semver::Version,
) -> (Vec<ReleaseNotes>, bool) {
    let mut kept: Vec<_> = history
        .into_iter()
        .filter_map(|notes| {
            let version = semver::Version::parse(&notes.version).ok()?;
            let wanted = if offered < current {
                version == *offered
            } else {
                version > *current && version <= *offered
            };
            wanted.then_some((version, notes))
        })
        .collect();
    kept.sort_by(|(left, _), (right, _)| right.cmp(left));
    let unlisted = kept.is_empty()
        || (offered > current
            && kept.iter().any(|(_, notes)| {
                notes.since.as_deref().is_some_and(|since| {
                    semver::Version::parse(since).ok().is_none_or(|since| {
                        since > *current && !kept.iter().any(|(version, _)| *version == since)
                    })
                })
            }));
    (kept.into_iter().map(|(_, notes)| notes).collect(), unlisted)
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
    let (releases, unlisted) = listed(manifest.releases, &version, current);
    Ok(Some(Release {
        older: version < *current,
        version: manifest.version,
        releases,
        unlisted,
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
    let mut body = Body::new(total)?;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("the download was cut off: {error}"))?
    {
        body.take(&chunk)?;
        progress(body.bytes.len() as u64, total);
    }
    let package = body.finish()?;
    verify(&package, &release.signature, &release.version)?;
    Ok(package)
}

struct Body {
    bytes: Vec<u8>,
}

impl Body {
    fn new(declared: Option<u64>) -> Result<Self, String> {
        Ok(Self {
            bytes: Vec::with_capacity(declared.unwrap_or(0) as usize),
        })
    }

    fn take(&mut self, chunk: &[u8]) -> Result<(), String> {
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    fn finish(self) -> Result<Vec<u8>, String> {
        Ok(self.bytes)
    }
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
    use super::{Body, ReleaseNotes, listed, offered};

    #[test]
    fn a_body_is_held_to_the_length_its_response_declares() {
        let read = |declared: Option<u64>, chunks: &[&[u8]]| -> Result<Vec<u8>, String> {
            let mut body = Body::new(declared)?;
            for chunk in chunks {
                body.take(chunk)?;
            }
            body.finish()
        };
        assert_eq!(read(Some(6), &[b"abc", b"def"]), Ok(b"abcdef".to_vec()));
        assert!(read(Some(4), &[b"abc", b"def"]).is_err());
        assert!(read(Some(5), &[b"abc"]).is_err());
        assert!(read(None, &[b"abc"]).is_err());
        assert!(read(Some(u64::MAX), &[b"abc"]).is_err());
    }

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

    #[test]
    fn an_app_several_releases_behind_lists_each_one_newest_first_and_names_a_gap() {
        let version = |text: &str| semver::Version::parse(text).unwrap();
        let notes = |text: &str, since: Option<&str>| ReleaseNotes {
            version: text.into(),
            since: since.map(Into::into),
            date: None,
            changes: 1,
            highlights: None,
            more: None,
            pulls: Vec::new(),
        };
        let history = || {
            vec![
                notes("0.1.2-nightly.20261121.3", Some("0.1.2-nightly.20261120.2")),
                notes("0.1.2-nightly.20261123.5", Some("0.1.2-nightly.20261122.4")),
                notes("0.1.2-nightly.20261122.4", Some("0.1.2-nightly.20261121.3")),
            ]
        };
        let shown = |listed: (Vec<ReleaseNotes>, bool)| {
            let versions: Vec<_> = listed.0.into_iter().map(|notes| notes.version).collect();
            (versions, listed.1)
        };
        let offered = version("0.1.2-nightly.20261123.5");
        assert_eq!(
            shown(listed(
                history(),
                &offered,
                &version("0.1.2-nightly.20261121.3")
            )),
            (
                vec![
                    "0.1.2-nightly.20261123.5".to_owned(),
                    "0.1.2-nightly.20261122.4".to_owned()
                ],
                false
            )
        );
        let behind = shown(listed(
            history(),
            &offered,
            &version("0.1.2-nightly.20261119.1"),
        ));
        assert_eq!((behind.0.len(), behind.1), (3, true));
        let switched = shown(listed(
            history(),
            &version("0.1.2-nightly.20261121.3"),
            &version("0.1.3-nightly.20261201.9"),
        ));
        assert_eq!(
            switched,
            (vec!["0.1.2-nightly.20261121.3".to_owned()], false)
        );
        let mut skipped = history();
        skipped.retain(|notes| notes.version != "0.1.2-nightly.20261122.4");
        assert!(listed(skipped, &offered, &version("0.1.2-nightly.20261120.2")).1);
        assert!(listed(Vec::new(), &offered, &version("0.1.1")).1);
        assert!(
            !listed(
                vec![notes("0.1.2", None)],
                &version("0.1.2"),
                &version("0.1.1")
            )
            .1
        );
    }
}
