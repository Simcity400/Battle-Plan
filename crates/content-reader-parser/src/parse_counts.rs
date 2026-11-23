use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};

#[derive(serde::Serialize)]
pub struct ParseCount {
    pub parser: &'static str,
    pub digest: [u8; 32],
    pub calls: u64,
}

static PARSES: OnceLock<Mutex<BTreeMap<(&'static str, [u8; 32]), u64>>> = OnceLock::new();

pub fn parse(parser: &'static str, bytes: &[u8]) {
    if !enabled() {
        return;
    }
    let digest = Sha256::digest(bytes).into();
    *PARSES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .entry((parser, digest))
        .or_default() += 1;
}

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("BATTLEPLAN_CONTENT_COUNTS_FILE")
            .or_else(|| std::env::var_os("HOI4_CONTENT_COUNTS_FILE"))
            .is_some()
    })
}

pub fn snapshot() -> Vec<ParseCount> {
    PARSES
        .get()
        .map(|parses| {
            parses
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
                .map(|((parser, digest), calls)| ParseCount {
                    parser,
                    digest: *digest,
                    calls: *calls,
                })
                .collect()
        })
        .unwrap_or_default()
}
