use super::super::native_shader::draw;
use shader_pipeline::library::{Effect, Selection};
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
pub(super) struct Cache {
    effects: HashMap<usize, NamedEffect>,
}

struct NamedEffect {
    _effect: Arc<Effect>,
    techniques: HashMap<String, Technique>,
}

struct Technique {
    selection: Option<Selection>,
    paths: HashMap<String, CachedLabel>,
}

struct CachedLabel {
    text: Arc<str>,
    used: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Text {
    Fresh(String),
    Shared(Arc<str>),
}

impl std::ops::Deref for Text {
    type Target = str;
    fn deref(&self) -> &str {
        match self {
            Self::Fresh(s) => s,
            Self::Shared(s) => s,
        }
    }
}

impl std::fmt::Debug for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}

pub(super) struct Metadata {
    pub label: Text,
    pub selection: Result<Option<Selection>, String>,
}

impl Cache {
    pub fn begin(&mut self) {
        for effect in self.effects.values_mut() {
            for technique in effect.techniques.values_mut() {
                for label in technique.paths.values_mut() {
                    label.used = false;
                }
            }
        }
    }

    pub fn finish(&mut self) {
        self.effects.retain(|_, effect| {
            effect.techniques.retain(|_, technique| {
                technique.paths.retain(|_, label| label.used);
                !technique.paths.is_empty()
            });
            !effect.techniques.is_empty()
        });
    }

    pub fn metadata(&mut self, draw: &draw::Quad) -> Metadata {
        let asset = &draw.effect.asset;
        let name = &draw.effect.technique;
        let label = || format!("{} / {}:{}", draw.path, asset.source.effect, name);
        let key = Arc::as_ptr(asset) as usize;
        if let Some(technique) = self
            .effects
            .get_mut(&key)
            .and_then(|e| e.techniques.get_mut(name))
        {
            let text = if let Some(path) = technique.paths.get_mut(draw.path.as_str()) {
                path.used = true;
                path.text.clone()
            } else {
                let text = Arc::<str>::from(label());
                technique.paths.insert(
                    draw.path.clone(),
                    CachedLabel {
                        text: text.clone(),
                        used: true,
                    },
                );
                text
            };
            return Metadata {
                label: Text::Shared(text),
                selection: Ok(technique.selection.clone()),
            };
        }
        let label = label();
        let selection = asset.select(name);
        let label = if let Ok(selection) = &selection {
            let label = Arc::<str>::from(label);
            self.effects
                .entry(key)
                .or_insert_with(|| NamedEffect {
                    _effect: asset.clone(),
                    techniques: HashMap::new(),
                })
                .techniques
                .insert(
                    name.clone(),
                    Technique {
                        selection: selection.clone(),
                        paths: [(
                            draw.path.clone(),
                            CachedLabel {
                                text: label.clone(),
                                used: true,
                            },
                        )]
                        .into(),
                    },
                );
            Text::Shared(label)
        } else {
            Text::Fresh(label)
        };
        Metadata { label, selection }
    }
}

pub(super) fn views<T>(
    storage: &mut Vec<(u32, Result<Option<T>, String>)>,
    draw: &draw::Quad,
    mut resolve: impl FnMut(usize) -> Option<T>,
) {
    storage.clear();
    storage.extend(draw.textures.iter().map(|&(slot, id)| {
        (
            slot,
            id.map(|id| {
                resolve(id).ok_or_else(|| {
                    format!(
                        "effect {:?}, draw {:?}: texture {id} is not resident",
                        draw.effect.asset.source.effect, draw.path
                    )
                })
            })
            .transpose(),
        )
    }));
}

#[cfg(test)]
#[path = "packet_tests.rs"]
mod tests;
