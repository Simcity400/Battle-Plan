use super::TableId;

impl TableId {
    /// What the table holds in plain lowercase words, for a reason a user reads: its name spelled
    /// out ("DivisionEquipmentDeletions" is "division equipment deletions"), never the name itself.
    pub fn words(self) -> String {
        let name = format!("{self:?}");
        let chars: Vec<char> = name.chars().collect();
        let mut out = String::with_capacity(name.len() + 8);
        for (at, &char) in chars.iter().enumerate() {
            let previous = at.checked_sub(1).map(|at| chars[at]);
            let next = chars.get(at + 1).copied();
            let starts = char.is_ascii_uppercase()
                && previous.is_some_and(|previous| previous.is_ascii_lowercase() || previous.is_ascii_digit() || (previous.is_ascii_uppercase() && next.is_some_and(|next| next.is_ascii_lowercase())));
            let digits = char.is_ascii_digit() && previous.is_some_and(|previous| previous.is_ascii_alphabetic());
            if starts || digits {
                out.push(' ');
            }
            out.push(char.to_ascii_lowercase());
        }
        out
    }

    /// One of its fields in plain words, for a reason a user reads: "pride_name" of
    /// CountryCounterInputs is "pride name of country counter inputs", never `CountryCounterInputs.pride_name`.
    pub fn field_words(self, field: &str) -> String {
        format!("{} of {}", field.replace('_', " "), self.words())
    }
}

#[cfg(test)]
mod tests {
    use super::TableId;

    #[test]
    fn a_table_is_named_to_a_user_in_plain_words_never_by_its_identifier() {
        assert_eq!(TableId::DivisionEquipmentDeletions.words(), "division equipment deletions");
        assert_eq!(TableId::NavalRefitProgress.words(), "naval refit progress");
        assert_eq!(TableId::Campaign.words(), "campaign");
        for table in TableId::ALL {
            let words = table.words();
            assert!(!words.is_empty() && !words.contains("  ") && !words.starts_with(' '), "{table:?}: {words:?}");
            assert!(!words.chars().any(|char| char.is_ascii_uppercase()), "{table:?}: {words:?}");
            assert_eq!(words.replace(' ', ""), format!("{table:?}").to_ascii_lowercase(), "{table:?} keeps every letter");
        }
    }

    #[test]
    fn a_field_is_named_to_a_user_in_plain_words_never_as_table_dot_field() {
        assert_eq!(TableId::CountryCounterInputs.field_words("pride_name"), "pride name of country counter inputs");
        assert_eq!(TableId::ScriptVariables.field_words("seed"), "seed of script variables");
        let words = TableId::AdvisorRoles.field_words("modifier_cache");
        assert!(!words.contains('_') && !words.contains('.') && !words.chars().any(|char| char.is_ascii_uppercase()), "{words}");
    }
}
