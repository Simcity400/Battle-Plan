//! What content says of the stockpile's equipment, for J's and H's replies: each variant's and
//! logistics token's archetype, and each archetype's name, domain and need note.

use super::{StockType, UNRESOLVED_MARK, need_note, type_name};
use dashboard_base::{
    catalogue,
    products::equipment::{self, Domain, Texts, archetype_of, equipment_warnings},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_series::{PageRead, Snapshot};
use dashboard_text::catalogue::Catalogue;
use serde::Serialize;
use installed_content_core::equipment::Database;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// What a reader reads besides the publication: content, the campaign's key, and where it keeps
/// what it builds between replies (none keeps nothing).
pub struct Reading<'a> {
    pub(crate) content: Content,
    pub(crate) campaign_key: String,
    pub(crate) kept: Option<(&'a Source, &'a store::SourceKey)>,
}

impl<'a> Reading<'a> {
    pub fn new(
        source: &'a Source,
        context: &LoadContext,
        key: &'a store::SourceKey,
    ) -> io::Result<Self> {
        Ok(Self {
            content: Content::load(source, context),
            campaign_key: engine_allocation::string(source.campaign_key())?,
            kept: Some((source, key)),
        })
    }

    /// A reading of `types` alone, keeping nothing.
    #[cfg(test)]
    pub(crate) fn bare(types: Option<Database>) -> Self {
        Self {
            content: Content {
                catalogue: None,
                types: types.map(Arc::new),
                warnings: Vec::new(),
            },
            campaign_key: "campaign".into(),
            kept: None,
        }
    }
}

/// A publication's identity, its generation, segment and hours.
pub(crate) type Published = ([u8; 32], [u8; 32], [u8; 32], u64, u64, u64, i32, i32, i32);

/// The publication a reply is read from: a new one replaces what was kept of the last.
pub(crate) fn publication<P: PageRead>(snapshot: &Snapshot<'_, P>) -> Published {
    let identity = snapshot.identity();
    let publication = snapshot.publication();
    (
        identity.source,
        identity.content,
        identity.branch,
        identity.schema,
        publication.generation,
        publication.segment,
        publication.accepted.first,
        publication.accepted.last,
        publication.reached,
    )
}

impl Reading<'_> {
    /// `build`'s answer, kept per publication where the reading keeps what it builds.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn keep<T: Send + Sync + 'static>(
        &self,
        what: &'static str,
        branch: u64,
        params: &impl Serialize,
        publication: &Published,
        cancelled: &AtomicBool,
        bytes: impl FnOnce(&T) -> usize,
        build: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        match self.kept {
            Some((source, key)) => dashboard_base::prepared::query::observations::scope(
                source,
                what,
                Some(key),
            )
            .get_entry(branch, params, publication, cancelled, bytes, build),
            None => build().map(Arc::new),
        }
    }
}

/// Owned strings' heap bytes, with their own size, for a cache's charge.
pub(crate) fn text_bytes<'s>(texts: impl IntoIterator<Item = &'s String>) -> usize {
    texts
        .into_iter()
        .map(|text| std::mem::size_of::<String>() + text.capacity())
        .sum()
}

/// The recorded content stack's catalogue and equipment types; none without the game folders,
/// where stock still shows by variant under unnamed types.
pub(crate) struct Content {
    catalogue: Option<Arc<Catalogue>>,
    types: Option<Arc<Database>>,
    pub warnings: Vec<String>,
}

impl Content {
    fn load(source: &Source, context: &LoadContext) -> Self {
        let mut warnings = Vec::new();
        let catalogue = match context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
        {
            Ok(catalogue) => {
                warnings.extend(catalogue.verification.iter().cloned());
                warnings.extend(catalogue.warnings.iter().cloned());
                Some(catalogue)
            }
            Err(error) => {
                warnings.push(format!(
                    "Equipment names and types are unavailable: {error}"
                ));
                None
            }
        };
        let types = catalogue
            .as_ref()
            .and_then(|catalogue| match equipment::units(source, catalogue) {
                Ok(types) => {
                    warnings.extend(equipment_warnings(&types));
                    Some(types)
                }
                Err(error) => {
                    warnings.push(format!("Equipment types are unavailable: {error}"));
                    None
                }
            });
        Self {
            catalogue,
            types,
            warnings,
        }
    }

    pub(crate) fn texts(&self) -> Option<&dyn Texts> {
        self.catalogue
            .as_deref()
            .map(|catalogue| catalogue as &dyn Texts)
    }

    /// The content refusals a reply ends with.
    pub(crate) fn refusals(&self) -> Vec<String> {
        self.catalogue
            .as_ref()
            .map(|catalogue| catalogue.refusal_lines())
            .unwrap_or_default()
    }
}

/// The archetypes a reply meets, with the types content does not resolve.
#[derive(Default)]
pub(crate) struct Types {
    found: BTreeMap<String, (usize, Domain)>,
    resolved: BTreeMap<String, Result<(String, usize, Domain), String>>,
    untyped: BTreeSet<String>,
    untyped_count: usize,
}

impl Types {
    fn resolve(&mut self, content: &Content, token: &str) -> Option<String> {
        let types = content.types.as_ref()?;
        let resolved = self
            .resolved
            .entry(token.to_owned())
            .or_insert_with(|| archetype_of(types, token));
        match resolved {
            Ok((archetype, ordinal, domain)) => {
                self.found.insert(archetype.clone(), (*ordinal, *domain));
                Some(archetype.clone())
            }
            Err(reason) => {
                self.untyped.insert(reason.clone());
                self.untyped_count += 1;
                None
            }
        }
    }

    /// A variant's archetype from its final nonempty token; none where content does not
    /// resolve it, or no read named its token.
    pub(crate) fn variant(&mut self, content: &Content, token: &str) -> Option<String> {
        if token.is_empty() {
            return None;
        }
        self.resolve(content, token)
    }

    /// A logistics token's key: its archetype, else the unresolved mark and its spelling.
    pub(crate) fn logistics(&mut self, content: &Content, spelling: &str) -> String {
        self.resolve(content, spelling)
            .unwrap_or_else(|| format!("{UNRESOLVED_MARK}{spelling}"))
    }

    /// The archetype's domain, once met.
    pub(crate) fn domain(&self, archetype: &str) -> Option<Domain> {
        self.found.get(archetype).map(|(_, domain)| *domain)
    }

    /// Every archetype met, in the equipment database's order, with the warnings of types
    /// content does not resolve.
    pub(crate) fn finish(self, content: &Content, warnings: &mut Vec<String>) -> Vec<StockType> {
        let mut types: Vec<_> = self.found.into_iter().collect();
        types.sort_by_key(|(_, (ordinal, _))| *ordinal);
        let marked: Vec<&str> = types
            .iter()
            .map(|(key, _)| key.as_str())
            .filter(|key| key.starts_with(UNRESOLVED_MARK))
            .collect();
        if !marked.is_empty() {
            warnings.push(format!(
                "Equipment types whose key starts with \"{UNRESOLVED_MARK}\" are not supported: the widget marks unresolved types that way, so they could be mistaken for one ({}).",
                marked.join(", ")
            ));
        }
        if self.untyped_count > 0 {
            warnings.push(format!(
                "{} recorded variants or archetypes have an equipment type the content does not resolve; their type titles and every total they could belong to are unavailable ({}).",
                self.untyped_count,
                self.untyped.into_iter().collect::<Vec<_>>().join("; ")
            ));
        }
        types
            .into_iter()
            .map(|(key, (_, domain))| StockType {
                name: content
                    .catalogue
                    .as_ref()
                    .and_then(|catalogue| type_name(catalogue, &key)),
                need_note: match &content.types {
                    Some(types) => need_note(types, &key).unwrap_or_else(Some),
                    None => Some(
                        "Content is unavailable, so what is needed cannot be derived.".to_owned(),
                    ),
                },
                key,
                domain,
            })
            .collect()
    }
}
