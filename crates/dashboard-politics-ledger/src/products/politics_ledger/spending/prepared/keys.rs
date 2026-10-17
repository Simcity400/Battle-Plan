//! Borrowed lookup of series operands; publication keeps the flat key map.
use super::*;

#[derive(Default)]
pub(super) struct Keys {
    tags: BTreeMap<String, BTreeMap<String, u64>>,
    len: usize,
    #[cfg(test)]
    encodings: std::cell::Cell<usize>,
}

impl Keys {
    pub(super) fn key(&mut self, tag: &str, field: &str) -> io::Result<u64> {
        let next = u64::try_from(self.len)
            .map_err(|_| dashboard_core::invalid("Politics series key count overflow"))?;
        if let Some(key) = self.tags.get(tag).and_then(|fields| fields.get(field)) {
            return Ok(*key);
        }
        self.tags
            .entry(tag.into())
            .or_default()
            .insert(field.into(), next);
        self.len += 1;
        Ok(next)
    }

    pub(super) fn flat(&self) -> BTreeMap<String, u64> {
        self.tags
            .iter()
            .flat_map(|(tag, fields)| {
                fields.iter().map(move |(field, key)| {
                    #[cfg(test)]
                    self.encodings.set(self.encodings.get() + 1);
                    (series::field_key(tag, field), *key)
                })
            })
            .collect()
    }
}

impl Serialize for Keys {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.flat().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Keys {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let flat = BTreeMap::<String, u64>::deserialize(deserializer)?;
        let mut result = Self {
            len: flat.len(),
            ..Default::default()
        };
        for (name, key) in flat {
            let (tag, field) = serde_json::from_str::<(String, String)>(&name)
                .map_err(serde::de::Error::custom)?;
            result.tags.entry(tag).or_default().insert(field, key);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_key_lookups_encode_only_at_publication_and_keep_flat_bytes_and_ordinals() {
        let operands = [
            ("C1", "Political:coverage"),
            ("C1", "[\"item\",\"quoted \\\" text\\\\\"]:charge"),
            ("C2", "command\n\t\r\u{0}:\u{e9}"),
            ("*", "accepted"),
        ];
        let mut keys = Keys::default();
        let mut flat = BTreeMap::new();
        for _ in 0..100 {
            for (tag, field) in operands {
                let next = flat.len() as u64;
                let old = *flat.entry(series::field_key(tag, field)).or_insert(next);
                assert_eq!(keys.key(tag, field).unwrap(), old);
            }
        }
        assert_eq!(keys.encodings.get(), 0);
        let saved = serde_json::to_vec(&keys).unwrap();
        assert_eq!(saved, serde_json::to_vec(&flat).unwrap());
        assert_eq!(keys.encodings.get(), operands.len());
        let mut resumed: Keys = serde_json::from_slice(&saved).unwrap();
        assert_eq!(resumed.key("C1", "Political:coverage").unwrap(), 0);
        assert_eq!(resumed.key("C3", "new").unwrap(), operands.len() as u64);
        flat.insert(series::field_key("C3", "new"), operands.len() as u64);
        assert_eq!(
            serde_json::to_vec(&resumed).unwrap(),
            serde_json::to_vec(&flat).unwrap()
        );
    }
}
