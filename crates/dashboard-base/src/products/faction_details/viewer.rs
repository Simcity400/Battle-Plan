//! The selected country's scripted text, shared by every dynamic faction text of an hour.
use super::*;
use replay_data::rows::{CountryLinkRow, LocalPlayerRow, mask};
use replay_frame::frame::HourFrame;
use std::cell::OnceCell;

/// The fields the selected country is read by.
const VIEWER: u64 = mask::<LocalPlayerRow>(&["country", "observer_country"]);
pub(super) const LINK: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);

/// The selected country's raw link, as a faction text's environment reads it.
pub(super) fn viewing(frame: HourFrame<'_>) -> Result<Option<u32>, String> {
    let rows = frame
        .complete::<tables::LocalPlayer>(VIEWER)
        .map_err(|why| why.to_string())?;
    Ok(match rows.iter().next() {
        Some(row) => row.map_err(|why| why.to_string())?.viewing_country(),
        None => None,
    })
}

/// The selected country's canonical slot, by `frame`'s links: `none` without a selected
/// country, `unlinked` without its link.
pub(super) fn viewer_slot(
    frame: HourFrame<'_>,
    none: &str,
    unlinked: &str,
) -> Result<u32, String> {
    let raw = viewing(frame)?.ok_or(none)?;
    let links = frame
        .complete::<tables::CountryLinks>(LINK)
        .map_err(|why| why.to_string())?;
    crate::metadata::link_slot_in(&links, raw).ok_or_else(|| unlinked.to_owned())
}

/// The scripted text the dynamic faction texts of `frame`'s hour expand in, built once for
/// all of them and outside any reuse recording: an evaluation records the selected country's
/// reads and its own queries, never the tables the shared text is built from.
pub struct ViewerText<'a> {
    frame: HourFrame<'a>,
    cat: &'a catalogue::Catalogue,
    /// The walk's lexer folds, given to each text built.
    lexicon: Option<&'a dashboard_world::Lexicon>,
    text: OnceCell<(u32, ScriptedText<'a>)>,
}

impl<'a> ViewerText<'a> {
    pub fn new(frame: &'a DecodedFrame, cat: &'a catalogue::Catalogue) -> Self {
        Self::new_in(HourFrame::from(frame), cat)
    }

    /// [`Self::new`] of any frame's tables, a walk's views among them.
    pub fn new_in(frame: HourFrame<'a>, cat: &'a catalogue::Catalogue) -> Self {
        Self {
            frame,
            cat,
            lexicon: None,
            text: OnceCell::new(),
        }
    }

    /// This text on a walk's frame, finding tokens by the walk's lexer folds
    /// ([`ScriptedText::with_lexicon`]).
    pub fn with_lexicon(mut self, lexicon: Option<&'a dashboard_world::Lexicon>) -> Self {
        self.lexicon = lexicon;
        self
    }

    pub(super) fn frame(&self) -> HourFrame<'a> {
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
                ScriptedText::new_in(self.frame, &cat.localisation, content, slot)
                    .with_lexicon(self.lexicon)
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
        let links = self.frame.complete::<tables::CountryLinks>(LINK).ok()?;
        let slot = crate::metadata::link_slot_in(&links, raw)?;
        self.with(slot, |text| text.reread(key)).ok()?
    }
}
