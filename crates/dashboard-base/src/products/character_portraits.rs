//! Recorded portrait selection shared by character views. Asset resolution
//! remains in PoliticsContent, also shared with ideas and ruling parties.

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum RecordedPortrait {
    /// The recorded holder selected a content portrait of this kind/identity.
    Entry { kind: u32, identity: Vec<u8> },
    /// Native runtime generation is needed; its inputs are not recorded.
    Generated,
    /// The recording cannot establish the requested portrait holder.
    Unrecorded,
}

/// Select the requested portrait role/size, else the first entry of that size.
/// Unset entries require runtime generation.
pub fn recorded_portrait(
    holders: Option<&[replay_data::rows::CharacterPortraitRow]>,
    id: u32,
    role: u32,
    size: u32,
) -> RecordedPortrait {
    let holder = holders.and_then(|rows| {
        rows.binary_search_by_key(&id, |row| row.character)
            .ok()
            .map(|i| &rows[i])
    });
    let Some(holder) = holder else {
        return RecordedPortrait::Unrecorded;
    };
    match replay_data::rows::portrait_entry(&holder.portraits, role, size) {
        Some(entry) if entry.is_set != 0 => RecordedPortrait::Entry {
            kind: entry.kind,
            identity: entry.identity.clone(),
        },
        _ => RecordedPortrait::Generated,
    }
}

pub fn portrait_png(
    content: &super::politics_content::PoliticsContent,
    portrait: &RecordedPortrait,
) -> Result<String, String> {
    match portrait {
        RecordedPortrait::Entry { kind, identity } => content.recorded_portrait(*kind, identity),
        RecordedPortrait::Generated => {
            Err("The game generates this portrait from inputs the recording lacks.".into())
        }
        RecordedPortrait::Unrecorded => Err("The character's portraits were not recorded.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::CharacterPortrait, rows::CharacterPortraitRow, rows::PORTRAIT_LARGE};
    #[test]
    fn distinct_roles_and_missing_role_size_fallback() {
        let holder = CharacterPortraitRow {
            character: 73,
            portraits: vec![
                CharacterPortrait {
                    category: 0,
                    size: PORTRAIT_LARGE,
                    is_set: 1,
                    kind: 0,
                    identity: b"civilian".to_vec(),
                },
                CharacterPortrait {
                    category: 4,
                    size: PORTRAIT_LARGE,
                    is_set: 1,
                    kind: 1,
                    identity: b"scientist".to_vec(),
                },
            ],
        };
        assert_eq!(
            recorded_portrait(Some(std::slice::from_ref(&holder)), 73, 4, PORTRAIT_LARGE),
            RecordedPortrait::Entry {
                kind: 1,
                identity: b"scientist".to_vec()
            }
        );
        assert_eq!(
            recorded_portrait(Some(&[holder]), 73, 3, PORTRAIT_LARGE),
            RecordedPortrait::Entry {
                kind: 0,
                identity: b"civilian".to_vec()
            }
        );
        assert_eq!(
            recorded_portrait(None, 73, 4, PORTRAIT_LARGE),
            RecordedPortrait::Unrecorded
        );
    }
}
