use super::*;
use crate::{ContentAdmissionLease, GameFileResolver};
use portable_content_manifest::{UsedContentKey, ValidatedContentAdmission, paradox_text};
use replay_core::country_name_semantics::NamingInput;

const CULTURE_FILE: &str = "common/graphicalculturetype.txt";

pub(super) struct CultureRegistry {
    lease: ContentAdmissionLease,
    source: UsedContentKey,
    names: Vec<String>,
}

impl CultureRegistry {
    pub(super) fn load(resolver: &GameFileResolver, admission: &ValidatedContentAdmission) -> Result<Self> {
        let lease = resolver.content_admission_lease().ok_or("XMAP13 culture registry requires content admission")?;
        if lease.manifest_digest_sha256() != Some(admission.manifest_digest_sha256()) {
            return Err("XMAP13 culture registry resolver and admission disagree".into());
        }
        let source = admission.require_used_content(CULTURE_FILE)?.clone();
        let text = crate::latin1_bytes_to_string(&resolver.read_admitted_used_content(&source)?);
        Ok(Self { lease, source, names: registered_names(&text)? })
    }

    pub(super) fn name<'a>(
        &self, resolver: &GameFileResolver, admission: &ValidatedContentAdmission,
        observation: &'a NamingInput<String>,
    ) -> std::result::Result<&'a str, String> {
        if resolver.content_admission_lease().as_ref() != Some(&self.lease)
            || self.lease.manifest_digest_sha256() != Some(admission.manifest_digest_sha256())
            || admission.require_used_content(CULTURE_FILE).map_err(|error| error.to_string())? != &self.source {
            return Err("XMAP13 culture registry belongs to another content admission".into());
        }
        let token = match observation {
            NamingInput::Measured(token) => token,
            NamingInput::Absent => return Err("XMAP13 creator culture source string is absent".into()),
            NamingInput::Unavailable(reason) => return Err(format!("XMAP13 creator culture source string: {reason:?}")),
        };
        // 140705600 stores the exact name lookup in +1020; an unknown +1028 string yields null.
        Ok(if self.names.iter().any(|name| name == token) { token } else { "" })
    }
}

fn registered_names(text: &str) -> Result<Vec<String>> {
    let tokens = paradox_text::parse_native_token_stream(text)
        .map_err(|error| format!("XMAP13 culture registry: {error}"))?;
    let mut names = Vec::new();
    names.try_reserve(tokens.len().checked_add(1).ok_or("XMAP13 culture count overflow")?)?;
    names.push("noCultureType".into());
    for token in tokens {
        let (paradox_text::Scalar::Bare(mut token) | paradox_text::Scalar::Quoted(mut token)) = token;
        // 140180c00 constructs a fresh CReader with an empty variable table. This
        // raw lexer loop never populates it, so 1424ab540 leaves @names unchanged.
        // 140a2d180 copies through strlen; duplicate registrations retain order.
        if let Some(nul) = token.find('\0') { token.truncate(nul); }
        names.push(token);
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{admitted_fixture_read_set, temp_test_dir, write_modvfs_fixture};

    #[test]
    fn culture_registry_retains_exact_names_null_definition_and_duplicate_order() {
        assert_eq!(registered_names("german GERMAN \"quoted culture\" german\n").unwrap(),
            ["noCultureType", "german", "GERMAN", "quoted culture", "german"]);
        assert_eq!(registered_names("").unwrap(), ["noCultureType"]);
        assert_eq!(registered_names("german").unwrap(), ["noCultureType"]);
        assert_eq!(registered_names("\"german\"").unwrap(), ["noCultureType", "german"]);
        assert_eq!(registered_names("@name = german @name \"@name\" @(german)\n").unwrap(),
            ["noCultureType", "@name", "=", "german", "@name", "@name", "@", "(", "german", ")"]);
        assert_eq!(registered_names("types = { german }\n").unwrap(),
            ["noCultureType", "types", "=", "{", "german", "}"]);
        assert_eq!(registered_names("\"GER\0hidden\" \"unterminated").unwrap(),
            ["noCultureType", "GER", "unterminated"]);
        assert!(registered_names("\u{ff}").unwrap_err().to_string().contains("byte FF"));
    }

    #[test]
    fn actual_admitted_culture_lookup_distinguishes_unknown_name_and_foreign_content() {
        let make = |label, text| {
            let root = temp_test_dir(label);
            write_modvfs_fixture(&root, CULTURE_FILE, text);
            let raw = GameFileResolver::vanilla(&root).unwrap();
            let (resolver, admission) = admitted_fixture_read_set(&raw,
                &[CULTURE_FILE.into()].into_iter().collect(), &[], label);
            (root, resolver, admission)
        };
        let a = make("xmap13-culture-a", "german\n");
        let b = make("xmap13-culture-b", "other\n");
        let table = CultureRegistry::load(&a.1, &a.2).unwrap();
        for (name, expected) in [("german", "german"), ("noCultureType", "noCultureType"),
            ("GERMAN", ""), ("unknown", ""), ("", "")] {
            let observation = NamingInput::Measured(name.into());
            assert_eq!(table.name(&a.1, &a.2, &observation).unwrap(), expected);
        }
        assert!(table.name(&a.1, &a.2, &NamingInput::Absent).is_err());
        assert!(table.name(&a.1, &a.2, &NamingInput::Unavailable(
            replay_core::country_name_semantics::NamingRefusal::Changed)).unwrap_err().contains("Changed"));
        let name = NamingInput::Measured("german".into());
        assert!(table.name(&b.1, &b.2, &name).unwrap_err().contains("another content admission"));
        assert!(table.name(&a.1, &b.2, &name).is_err());
        assert!(CultureRegistry::load(&a.1, &b.2).is_err());
        // A new lease with the same manifest must not authorize the old cache.
        let (replacement, _) = GameFileResolver::vanilla(&a.0).unwrap()
            .with_content_admission(&a.2).unwrap().isolated_content_snapshot().unwrap();
        assert_eq!(replacement.content_admission_lease().unwrap().manifest_digest_sha256(),
            a.1.content_admission_lease().unwrap().manifest_digest_sha256());
        assert!(table.name(&replacement, &a.2, &name).unwrap_err().contains("another content admission"));
        let roots = [a.0.clone(), b.0.clone()];
        drop((table, replacement, a, b));
        for root in roots { portable_test_fs_retry::remove_dir_all(root).unwrap(); }
    }
}
