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
        let content = cat
            .ok_or_else(|| "Template equipment needs require the recorded content stack".to_owned())
            .and_then(|cat| {
                cat.content
                    .layers
                    .derived::<CachedResult<Content>>()
                    .get_or_init(|| Content::load(&cat.content.layers))
                    .map_err(|why| format!("Template equipment content unavailable: {why}"))
            });
        Self::from_content(content, frame)
    }

    fn from_content(content: Result<Arc<Content>, String>, frame: &'a DecodedFrame) -> Self {
        Self {
            content,
            sources: Sources::read(frame)
                .map_err(|why| format!("Template equipment inputs unavailable: {why}")),
        }
    }

    /// Content entries the native loader reports and drops. They do not make
    /// the surviving requirements unknown, but their names must stay visible.
    pub fn warnings(&self) -> impl Iterator<Item = &str> {
        self.content.as_ref().ok().into_iter().flat_map(|content| {
            content
                .dropped
                .iter()
                .chain(&content.equipment_content().misreads)
                .map(String::as_str)
        })
    }

    /// A template's need in native first-occurrence archetype order, fixed5.
    /// The visual representative and model exclusions do not restrict demand.
    pub fn template(
        &self,
        frame: &DecodedFrame,
        handle: u64,
    ) -> Result<Vec<(String, i64)>, String> {
        let content = self.content.as_ref().map_err(Clone::clone)?;
        let sources = self.sources.as_ref().map_err(Clone::clone)?;
        let rows = frame
            .complete::<tables::DivisionTemplateIcons>()
            .map_err(|why| format!("Template equipment composition unavailable: {why}"))?;
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
    pub(super) fn from_documents(documents: &[&[u8]], frame: &'a DecodedFrame) -> Self {
        Self::from_content(Ok(Arc::new(Content::from_test_documents(documents))), frame)
    }
}

#[cfg(test)]
mod tests;
