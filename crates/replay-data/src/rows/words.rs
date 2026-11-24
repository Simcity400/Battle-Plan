use super::TableId;

impl TableId {
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

    pub fn field_words(self, field: &str) -> String {
        format!("{} of {}", field.replace('_', " "), self.words())
    }
}

