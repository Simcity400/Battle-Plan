//! A render pass drawn only while something its compiled programs read changes.
//!
//! What a pass draws is decided entirely by what its programs read: the bytes of each constant
//! buffer they read (never a byte they do not), what every texture they bind holds, the target
//! they draw into and the list of draws. Those bytes are the pass's key. Two frames whose keys
//! are equal draw the same picture, so the target still holds the first one's and the second is
//! not drawn at all.
//!
//! The key is never a timer and never a heuristic. An input nothing can describe leaves the key
//! unformed, an unformed key equals no other, and the pass is then drawn every frame and names
//! why ([`Keyed::unkept`]).

use super::textures::Content;

/// Everything one pass's compiled programs read this frame, as the bytes themselves.
#[derive(Default)]
pub struct PassKey {
    bytes: Vec<u8>,
    /// Where each named part of the key starts, so a frame that drew again can say which part
    /// of what it reads changed.
    parts: Vec<(&'static str, usize)>,
    /// Why no key could be formed.
    unformed: Option<String>,
}

impl PartialEq for PassKey {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl PassKey {
    /// Names the part of the pass's inputs the next bytes belong to, so a frame that drew again
    /// can say what changed.
    pub fn part(&mut self, name: &'static str) -> &mut Self {
        self.parts.push((name, self.bytes.len()));
        self
    }

    /// What one of the pass's constant bindings reads of the block bound to it, and nothing
    /// else of that block ([`shader_pipeline::Buffer::read_bytes`]).
    pub fn read(&mut self, spec: &shader_pipeline::Buffer, block: &[u8]) -> &mut Self {
        spec.read_bytes(block, &mut self.bytes);
        self
    }

    /// A number the pass's picture depends on: a count, a size, a sample count.
    pub fn number(&mut self, value: u64) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    /// A name: the technique, the mode, the target.
    pub fn text(&mut self, text: &str) -> &mut Self {
        self.number(text.len() as u64);
        self.bytes.extend_from_slice(text.as_bytes());
        self
    }

    /// What a texture or target the pass binds holds, as its writer says. `None` is a writer
    /// that says nothing: the key cannot be formed, and `what` then names the input.
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

    /// Nothing describes this input, so the pass is drawn every frame and `reason` says why.
    pub fn unknown(&mut self, reason: impl Into<String>) -> &mut Self {
        if self.unformed.is_none() {
            self.unformed = Some(reason.into());
        }
        self
    }

    /// Why no key could be formed.
    pub fn unformed(&self) -> Option<&str> {
        self.unformed.as_deref()
    }

    fn clear(&mut self) {
        self.bytes.clear();
        self.parts.clear();
        self.unformed = None;
    }

    /// The part of this key the first byte differing from `held` belongs to.
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

/// One pass's keys: the one its target holds, and whether this frame's repeats it.
#[derive(Default)]
pub struct Keyed {
    /// The key the target's content was drawn from.
    held: Option<PassKey>,
    /// The key the frame being encoded draws, until that frame reaches the queue.
    pending: Option<PassKey>,
    /// A finished key's buffer, which the next frame's key fills again.
    spare: PassKey,
    /// Why the last key could not be formed.
    unkept: Option<String>,
    /// The part of the last key that differed from the held one.
    differed: Option<&'static str>,
}

impl Keyed {
    /// This frame's key, on the buffer an earlier frame's key left behind.
    pub fn start(&mut self) -> PassKey {
        let mut key = std::mem::take(&mut self.spare);
        key.clear();
        key
    }

    /// Whether the pass must be drawn for `key`: its target holds another key, or none, or no
    /// key could be formed. A pass told to draw holds `key` until [`Self::submitted`] says the
    /// frame that drew it reached the queue.
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

    /// The frame that drew reached the queue: the target now holds the key it drew.
    pub fn submitted(&mut self) {
        if let Some(drawn) = self.pending.take()
            && let Some(held) = self.held.replace(drawn)
        {
            self.spare = held;
        }
    }

    /// The target holds nothing a key describes: it was remade, or the pass did not draw.
    pub fn lost(&mut self) {
        self.pending = None;
        self.held = None;
    }

    /// Why the pass cannot be kept at all, from the last key built.
    pub fn unkept(&self) -> Option<&str> {
        self.unkept.as_deref()
    }

    /// Which named part of the last key differed from the one the target held; `None` when the
    /// target held nothing or the key repeated.
    pub fn differed(&self) -> Option<&'static str> {
        self.differed
    }
}

#[cfg(test)]
#[path = "pass_key/tests.rs"]
mod tests;
