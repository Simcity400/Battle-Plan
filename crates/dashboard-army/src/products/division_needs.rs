//! Equipment required by the template's grouped sub-units, resolved by the
//! same canonical content and local-clone reader as the map's model inputs.
use dashboard_text::{
    catalogue::{CachedResult, Catalogue},
    subunit_requirements::{Content, Sources},
};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, sync::Arc};

/// Content is parsed once for a bound stack; local definitions belong to this
/// reached hour, not to the supplying read of an equipment pool.
pub struct Needs<'a> {
    content: Result<Arc<Content>, String>,
    sources: Result<Sources<'a>, String>,
}

impl<'a> Needs<'a> {
    pub fn read(cat: Option<&Arc<Catalogue>>, frame: &'a DecodedFrame) -> Self {
        Self::from_content(content(cat), frame)
    }

    fn from_content(content: Result<Arc<Content>, String>, frame: &'a DecodedFrame) -> Self {
        Self {
            content,
            sources: sources(frame),
        }
    }

    /// Content entries the native loader reports and drops. They do not make
    /// the surviving requirements unknown, but their names must stay visible.
    pub fn warnings(&self) -> impl Iterator<Item = &str> {
        warnings(&self.content)
    }

    /// A template's need in native first-occurrence archetype order, fixed5.
    /// The visual representative and model exclusions do not restrict demand.
    pub fn template(
        &self,
        frame: &DecodedFrame,
        handle: u64,
    ) -> Result<Vec<(String, i64)>, String> {
        template(&self.content, || &self.sources, frame, handle)
    }

    #[cfg(test)]
    pub(super) fn from_documents(documents: &[&[u8]], frame: &'a DecodedFrame) -> Self {
        Self::from_content(Ok(Arc::new(Content::from_test_documents(documents))), frame)
    }
}

/// The content's needs, read once for a bound stack.
pub fn content(cat: Option<&Arc<Catalogue>>) -> Result<Arc<Content>, String> {
    cat.ok_or_else(|| "Template equipment needs require the recorded content stack".to_owned())
        .and_then(|cat| {
            cat.content
                .layers
                .derived::<CachedResult<Content>>()
                .get_or_init(|| Content::load(&cat.content.layers))
                .map_err(|why| format!("Template equipment content unavailable: {why}"))
        })
}

/// How a need refused for its template's unreadable composition table starts.
pub const COMPOSITION_UNAVAILABLE: &str = "Template equipment composition unavailable: ";

/// The hour's local definitions and token spellings the needs read.
pub fn sources(frame: &DecodedFrame) -> Result<Sources<'_>, String> {
    Sources::read(frame).map_err(|why| format!("Template equipment inputs unavailable: {why}"))
}

/// [`sources`] of an hour whose links, local sub-units and tokens are those `index` was read
/// with.
pub fn reused_sources(
    frame: &DecodedFrame,
    index: Arc<dashboard_text::subunit_requirements::SourceIndex>,
) -> Result<Sources<'_>, String> {
    Sources::reuse(frame, index)
        .map_err(|why| format!("Template equipment inputs unavailable: {why}"))
}

/// [`Needs::warnings`] of `content`.
pub fn warnings(content: &Result<Arc<Content>, String>) -> impl Iterator<Item = &str> {
    content.as_ref().ok().into_iter().flat_map(|content| {
        content
            .dropped
            .iter()
            .chain(&content.equipment_content().misreads)
            .map(String::as_str)
    })
}

/// [`Needs::template`] of `content`, the sources read only once the content has.
pub fn template<'s, 'a: 's>(
    content: &Result<Arc<Content>, String>,
    sources: impl FnOnce() -> &'s Result<Sources<'a>, String>,
    frame: &DecodedFrame,
    handle: u64,
) -> Result<Vec<(String, i64)>, String> {
    let content = content.as_ref().map_err(Clone::clone)?;
    let sources = sources().as_ref().map_err(Clone::clone)?;
    let rows = frame
        .complete::<tables::DivisionTemplateIcons>()
        .map_err(|why| format!("{COMPOSITION_UNAVAILABLE}{why}"))?;
    let at = rows
        .binary_search_by_key(&handle, |row| row.template)
        .map_err(|_| format!("Template {handle} equipment composition was not recorded"))?;
    let template = &rows[at];
    if template.model_unreadable != 0 {
        return Err(format!(
            "Template {handle} grouped sub-units were not readable"
        ));
    }
    let mut needs = Vec::<(String, i64)>::new();
    let mut indices = BTreeMap::<String, usize>::new();
    for unit in template.subunits.iter().filter(|unit| unit.token != 0) {
        let group = content
            .group(template, sources, unit.token, unit.local, unit.count)
            .map_err(|why| format!("Template {handle} equipment need: {why}"))?;
        for (archetype, amount) in group.requirements.iter() {
            let amount = amount.checked_mul(i64::from(group.count)).ok_or_else(|| {
                format!("Template {handle} {archetype} equipment need overflows fixed5")
            })?;
            let index = *indices.entry(archetype.clone()).or_insert_with(|| {
                needs.push((archetype.clone(), 0));
                needs.len() - 1
            });
            needs[index].1 = needs[index].1.checked_add(amount).ok_or_else(|| {
                format!("Template {handle} {archetype} equipment need overflows fixed5")
            })?;
        }
    }
    Ok(needs)
}

#[cfg(test)]
mod tests;
