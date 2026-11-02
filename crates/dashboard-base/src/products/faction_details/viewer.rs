//! The selected country's scripted text, shared by every dynamic faction text of an hour.
use super::*;
use std::cell::OnceCell;

/// The selected country's raw link, as a faction text's environment reads it.
pub(super) fn viewing(frame: &DecodedFrame) -> Result<Option<u32>, String> {
    Ok(frame
        .complete::<tables::LocalPlayer>()
        .map_err(|why| why.to_string())?
        .first()
        .and_then(|row| row.viewing_country()))
}

/// The scripted text the dynamic faction texts of `frame`'s hour expand in, built once for
/// all of them and outside any reuse recording: an evaluation records the selected country's
/// reads and its own queries, never the tables the shared text is built from.
pub struct ViewerText<'a> {
    frame: &'a DecodedFrame,
    cat: &'a catalogue::Catalogue,
    text: OnceCell<(u32, ScriptedText<'a>)>,
}

impl<'a> ViewerText<'a> {
    pub fn new(frame: &'a DecodedFrame, cat: &'a catalogue::Catalogue) -> Self {
        Self {
            frame,
            cat,
            text: OnceCell::new(),
        }
    }

    pub(super) fn frame(&self) -> &'a DecodedFrame {
        self.frame
    }

    pub(super) fn cat(&self) -> &'a catalogue::Catalogue {
        self.cat
    }

    /// `read` of the scripted text rooted at `slot`: the one held, else one built for it.
    pub(super) fn with<R>(
        &self,
        slot: u32,
        read: impl FnOnce(&ScriptedText<'a>) -> R,
    ) -> Result<R, String> {
        let cat: &'a catalogue::Catalogue = self.cat;
        let content = cat.scripted_content()?;
        let build = || {
            crate::triggers::inputs::unrecorded(|| {
                ScriptedText::new(self.frame, &cat.localisation, content, slot)
            })
        };
        let (held, text) = self.text.get_or_init(|| (slot, build()));
        if *held == slot {
            return Ok(read(text));
        }
        Ok(read(&build()))
    }

    /// The digest `key` reads at this hour in the selected country's text, as
    /// [`ScriptedText::reread`] reads it; None when it cannot be read again.
    pub fn reread(&self, key: &crate::triggers::inputs::Key) -> Option<u64> {
        let raw = viewing(self.frame).ok()??;
        let links = self.frame.complete::<tables::CountryLinks>().ok()?;
        let slot = crate::metadata::link_slot(links, raw)?;
        self.with(slot, |text| text.reread(key)).ok()?
    }
}
