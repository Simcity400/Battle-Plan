//! The one way a fixture in this crate's integration tests names a country.
//!
//! Nothing in this workspace turns a bare string into a `CountryIdentityRef`:
//! the constructor takes a frame's contended-key census, and there is no test
//! forge in or out of `replay_core` (lane UB removed the `test_support` feature
//! and the self dev-dependency for exactly this reason). So a fixture that
//! means to name a country opens a one-frame timeline and asks it, which is
//! the same question the decoder's constructor asks.
//!
//! A test ABOUT a contended key builds a frame carrying that account and gets
//! `None` from the same call -- see `country_identity_in`.

#![allow(dead_code)]

use replay_core::{CountryIdentityRef, RawFrame, ReplaySource, SourceError, WorldTimeline};

struct OneFrame {
    rest: serde_json::Map<String, serde_json::Value>,
}

impl ReplaySource for OneFrame {
    fn game_id(&self) -> &str {
        "game_ui_identity_support"
    }
    fn hours(&self) -> &[i64] {
        &[0]
    }
    fn warnings(&self) -> &[String] {
        &[]
    }
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        Ok(RawFrame {
            hour_index,
            date: None,
            save_version: None,
            schema_version: None,
            rest: self.rest.clone().into(),
        })
    }
}

/// This TAG as the census of a frame built from `rest` admits it, or `None`.
pub fn country_identity_in(
    rest: serde_json::Value,
    tag: &str,
) -> Option<CountryIdentityRef> {
    let rest = rest.as_object().expect("a frame object").clone();
    WorldTimeline::open(Box::new(OneFrame { rest }))
        .expect("a one-frame timeline")
        .frame(0)
        .expect("its frame")
        .country_identity_ref(tag)
}

/// This TAG in a frame whose census names no contended key -- the ordinary
/// case a fixture row means when it spells an owner.
pub fn identity(tag: &str) -> CountryIdentityRef {
    country_identity_in(serde_json::json!({}), tag)
        .expect("a census that names no contended key admits every TAG")
}
