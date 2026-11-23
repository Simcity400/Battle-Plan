
use dashboard_base::catalogue::{self, Catalogue, View};
pub(super) use dashboard_text::occupation_law::{GroupLaw, LawInputs, admitted, law_for};
use content_reader_politics::occupation_laws::Laws;
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OccupationLawInfo {
    pub token: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub missing: Option<String>,
}

fn icon(cat: &Catalogue, laws: &Laws, token: &str) -> Result<String, String> {
    let law = laws.find(token)?.ok_or_else(|| "Occupation law has no content declaration".to_owned())?;
    let sprite = cat.sprites.find(&law.sprite)?.ok_or_else(|| "Occupation-law sprite not in the recorded content stack".to_owned())?;
    let frame = u32::try_from(law.frame).ok().and_then(|frame| frame.checked_sub(1)).ok_or_else(|| "Occupation-law icon frame is not positive".to_owned())?;
    let decode = cat.gui_decode.clone()?;
    catalogue::sprite_frame_png(&cat.content.layers, sprite, decode, frame, 32).map_err(|error| error.to_string())
}

pub(super) fn info(cat: Result<&Catalogue, &str>, laws: Result<&Laws, &str>, token: &str) -> OccupationLawInfo {
    let name = cat.ok().and_then(|cat| cat.localised(token, View::Plain));
    let icon = match (cat, laws) {
        (Ok(cat), Ok(laws)) => icon(cat, laws, token),
        (Err(why), _) | (_, Err(why)) => Err(why.to_owned()),
    };
    OccupationLawInfo { token: token.to_owned(), name, missing: icon.as_ref().err().cloned(), icon: icon.ok() }
}
