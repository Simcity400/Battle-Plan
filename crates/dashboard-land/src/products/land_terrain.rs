use dashboard_base::{
    catalogue,
    products::{MAX_PRODUCT_REPLY_BYTES, Product, ReplyWarnings},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_text::catalogue::Catalogue;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct LandBattleTerrains;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LandBattleTerrainParams {
    pub provinces: Vec<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandTerrainName {
    pub province_id: u32,
    #[doc = " The terrain title the combat window shows; null when it cannot be resolved."]
    pub name: Option<String>,
    pub missing: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct LandBattleTerrainReply {
    pub provinces: Vec<LandTerrainName>,
    pub warnings: Vec<String>,
}
impl ReplyWarnings for LandBattleTerrainReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

pub struct Terrains {
    provinces: BTreeMap<u32, Vec<u8>>,
    categories: Vec<Vec<u8>>,
}
impl Terrains {
    pub fn read(layers: &content_reader_layers::ResolvedLayers) -> io::Result<Self> {
        let (definitions, map) = content_reader_key_names::terrain::read(layers)?;
        let provinces =
            content_reader_map::map::province_terrains(&layers.read(&map.file("definitions")?)?)?;
        Ok(Self {
            provinces,
            categories: definitions
                .categories
                .into_iter()
                .map(|category| category.name)
                .collect(),
        })
    }
    pub fn category(&self, province: u32) -> Result<&[u8], String> {
        let name = self
            .provinces
            .get(&province)
            .ok_or_else(|| format!("province {province} is not in the content map definitions"))?;
        self.categories
            .iter()
            .find(|category| category.eq_ignore_ascii_case(name))
            .map(Vec::as_slice)
            .ok_or_else(|| {
                format!(
                    "province {province}: content terrain category {} is absent",
                    String::from_utf8_lossy(name)
                )
            })
    }
    pub fn used(&self) -> impl Iterator<Item = (u32, &[u8])> {
        let mut seen = std::collections::BTreeSet::new();
        self.provinces
            .iter()
            .filter(move |(_, name)| seen.insert(name.to_ascii_lowercase()))
            .map(|(province, name)| (*province, name.as_slice()))
    }
}

pub fn check_content(cat: &Catalogue) -> io::Result<Vec<String>> {
    let terrains = Terrains::read(&cat.content.layers)?;
    Ok(terrains
        .used()
        .filter_map(|(province, _)| {
            terrains
                .category(province)
                .and_then(|category| title(cat, category))
                .err()
        })
        .collect())
}

pub fn title(cat: &Catalogue, category: &[u8]) -> Result<String, String> {
    let key = String::from_utf8_lossy(category);
    if !cat.has_presentation_key(&key) {
        return Err(format!(
            "localisation {key} is missing; terrain title is unavailable"
        ));
    }
    cat.localisation
        .localise(category)
        .map_err(|error| error.to_string())
        .and_then(|text| catalogue::plain_text(&text))
}

pub fn name(cat: &Catalogue, terrains: &Terrains, province: u32) -> LandTerrainName {
    match terrains
        .category(province)
        .and_then(|category| title(cat, category))
    {
        Ok(name) => LandTerrainName {
            province_id: province,
            name: Some(name),
            missing: None,
        },
        Err(reason) => LandTerrainName {
            province_id: province,
            name: None,
            missing: Some(reason),
        },
    }
}

type Cached = (Arc<Catalogue>, Result<Arc<Terrains>, String>);
#[derive(Default)]
struct TerrainState {
    content: Option<Cached>,
}
fn terrains(source: &Source, cat: &Arc<Catalogue>) -> Result<Arc<Terrains>, String> {
    let state = source.state::<TerrainState>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((cached, terrains)) = &state.content
        && Arc::ptr_eq(cached, cat)
    {
        return terrains.clone();
    }
    let terrains = Terrains::read(&cat.content.layers)
        .map(Arc::new)
        .map_err(|error| format!("Land terrain unavailable: {error}"));
    state.content = Some((Arc::clone(cat), terrains.clone()));
    terrains
}

impl Product for LandBattleTerrains {
    const KEY: &'static str = "land-battle-terrain";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = LandBattleTerrainParams;
    type Reply = LandBattleTerrainReply;
    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            LandBattleTerrainParams::decl(),
            LandTerrainName::decl(),
            LandBattleTerrainReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        let mut seen = std::collections::BTreeSet::new();
        if params
            .provinces
            .iter()
            .any(|province| !seen.insert(*province))
        {
            return Err("Invalid terrain selection");
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        _: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let cat = catalogue::catalogue(source, context.content()?)?;
        let reply = match terrains(source, &cat) {
            Ok(terrains) => LandBattleTerrainReply {
                provinces: params
                    .provinces
                    .iter()
                    .map(|&province| name(&cat, &terrains, province))
                    .collect(),
                warnings: Vec::new(),
            },
            Err(reason) => LandBattleTerrainReply {
                provinces: params
                    .provinces
                    .iter()
                    .map(|&province| LandTerrainName {
                        province_id: province,
                        name: None,
                        missing: Some(reason.clone()),
                    })
                    .collect(),
                warnings: vec![reason],
            },
        };
        dashboard_base::products::check_reply_budget(&reply).map_err(|_| {
            dashboard_core::invalid(format!(
                "Terrain reply exceeds MAX_PRODUCT_REPLY_BYTES ({MAX_PRODUCT_REPLY_BYTES} bytes); no province was omitted."
            ))
        })?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terrains() -> Terrains {
        Terrains {
            provinces: content_reader_map::map::province_terrains(
                b"0;0;0;0;land;false;unknown;0\n1;10;20;30;land;false;Mod_Hills;1\n2;10;20;31;land;false;plains;1\n3;10;20;32;land;false;swamp;1\n",
            )
            .unwrap(),
            categories: vec![b"unknown".to_vec(), b"plains".to_vec(), b"mod_hills".to_vec(), b"MOD_HILLS".to_vec()],
        }
    }

    #[test]
    fn province_terrain_matches_the_first_category_without_regard_to_case() {
        let terrains = terrains();
        assert_eq!(terrains.category(1).unwrap(), b"mod_hills");
        assert_eq!(terrains.category(2).unwrap(), b"plains");
        assert!(
            terrains
                .category(3)
                .unwrap_err()
                .contains("swamp is absent")
        );
        assert!(
            terrains
                .category(9)
                .unwrap_err()
                .contains("not in the content map definitions")
        );
        assert_eq!(
            terrains
                .used()
                .map(|(_, name)| name.to_vec())
                .collect::<Vec<_>>(),
            [
                b"unknown".to_vec(),
                b"Mod_Hills".to_vec(),
                b"plains".to_vec(),
                b"swamp".to_vec()
            ]
        );
    }

    #[test]
    fn terrain_selection_has_no_chosen_limit_and_refuses_duplicates() {
        let params = LandBattleTerrainParams {
            provinces: (0..5000).collect(),
        };
        assert!(LandBattleTerrains::validate(&params).is_ok());
        assert!(
            LandBattleTerrains::validate(&LandBattleTerrainParams {
                provinces: vec![1, 1]
            })
            .is_err()
        );
        assert!(
            serde_json::from_value::<LandBattleTerrainParams>(
                serde_json::json!({"provinces": [1], "branchId": "1"})
            )
            .is_err()
        );
    }
}
