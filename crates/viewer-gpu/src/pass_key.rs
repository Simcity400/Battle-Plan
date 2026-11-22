
use super::textures::Content;

#[derive(Default)]
pub struct PassKey {
    bytes: Vec<u8>,
    parts: Vec<(&'static str, usize)>,
    unformed: Option<String>,
}

impl PartialEq for PassKey {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl PassKey {
    pub fn part(&mut self, name: &'static str) -> &mut Self {
        self.parts.push((name, self.bytes.len()));
        self
    }

    pub fn read(&mut self, spec: &shader_pipeline::Buffer, block: &[u8]) -> &mut Self {
        spec.read_bytes(block, &mut self.bytes);
        self
    }

    pub fn number(&mut self, value: u64) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn text(&mut self, text: &str) -> &mut Self {
        self.number(text.len() as u64);
        self.bytes.extend_from_slice(text.as_bytes());
        self
    }

    pub fn content(
        &mut self,
        what: impl FnOnce() -> String,
        content: Option<Content>,
    ) -> &mut Self {
        match content {
            Some(content) => self.number(content.number()),
            None => self.unknown(format!("{} publishes no content", what())),
        }
    }

    pub fn unknown(&mut self, reason: impl Into<String>) -> &mut Self {
        if self.unformed.is_none() {
            self.unformed = Some(reason.into());
        }
        self
    }

    pub fn unformed(&self) -> Option<&str> {
        self.unformed.as_deref()
    }

    fn clear(&mut self) {
        self.bytes.clear();
        self.parts.clear();
        self.unformed = None;
    }

    fn differing_part(&self, held: &Self) -> Option<&'static str> {
        let at = self
            .bytes
            .iter()
            .zip(&held.bytes)
            .position(|(mine, theirs)| mine != theirs)
            .unwrap_or(self.bytes.len().min(held.bytes.len()));
        self.parts
            .iter()
            .rev()
            .find(|(_, start)| *start <= at)
            .map(|(name, _)| *name)
    }
}

#[derive(Default)]
pub struct Keyed {
    held: Option<PassKey>,
    pending: Option<PassKey>,
    spare: PassKey,
    unkept: Option<String>,
    differed: Option<&'static str>,
}

impl Keyed {
    pub fn start(&mut self) -> PassKey {
        let mut key = std::mem::take(&mut self.spare);
        key.clear();
        key
    }

    pub fn draws(&mut self, key: PassKey) -> bool {
        self.unkept = key.unformed.clone();
        let draws = key.unformed.is_some() || self.held.as_ref() != Some(&key);
        self.differed = match (&self.held, draws) {
            (Some(held), true) => key.differing_part(held),
            _ => None,
        };
        let dropped = if draws {
            self.pending.replace(key)
        } else {
            self.pending = None;
            Some(key)
        };
        if let Some(dropped) = dropped {
            self.spare = dropped;
        }
        draws
    }

    pub fn submitted(&mut self) {
        if let Some(drawn) = self.pending.take()
            && let Some(held) = self.held.replace(drawn)
        {
            self.spare = held;
        }
    }

    pub fn lost(&mut self) {
        self.pending = None;
        self.held = None;
    }

    pub fn unkept(&self) -> Option<&str> {
        self.unkept.as_deref()
    }

    pub fn differed(&self) -> Option<&'static str> {
        self.differed
    }
}

#[cfg(test)]
#[path = "pass_key/tests.rs"]
mod tests;
