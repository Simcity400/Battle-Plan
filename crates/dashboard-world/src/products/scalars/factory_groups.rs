
use super::branch;
use dashboard_base::{Source, catalogue::{self, Catalogue, View}, products::{LoadContext, Product}};
use content_reader_key_names::buildings::{Buildings, FactoryFlags};
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct FactoryGroups;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactoryGroupsParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactoryGroupBuilding {
    #[doc = " The content's building type."]
    pub token: String,
    #[doc = " Its name in the reader's language; null when the content has none."]
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactoryGroupNames {
    #[doc = " `military`, `naval` or `civilian`, as the factory widgets name the group."]
    pub key: String,
    #[doc = " The production flag that counts a building type in it."]
    pub flag: String,
    #[doc = " The building types counted in it, in the content's declaration order."]
    pub buildings: Vec<FactoryGroupBuilding>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactoryGroupsReply {
    pub groups: Vec<FactoryGroupNames>,
    #[doc = " Why no group can be named: the content is not configured or cannot be read."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FactoryGroupsReply);

const GROUPS: [(&str, &str, fn(&FactoryFlags) -> bool); 3] = [
    ("military", "military_production", FactoryFlags::military),
    ("naval", "naval_production", FactoryFlags::naval),
    ("civilian", "general_production", FactoryFlags::civilian),
];

pub(super) fn groups(buildings: &Buildings, name: impl Fn(&str) -> Option<String>) -> (Vec<FactoryGroupNames>, Vec<String>) {
    let mut warnings: Vec<String> = buildings
        .ambiguous_factory_types
        .iter()
        .map(|token| format!("{token} is declared more than once with different factory flags; which group it counts in is not known, so it names none"))
        .collect();
    let counted = |token: &&String| !buildings.ambiguous_factory_types.contains(*token);
    for token in buildings.declaration_order.iter().filter(counted) {
        if buildings.factory_flags.get(token).is_some_and(FactoryFlags::rocket) {
            warnings.push(format!("{token} has a rocket production flag; that group is not recorded, so its factories count in no group shown"));
        }
    }
    let groups = GROUPS
        .iter()
        .map(|&(key, flag, counts)| {
            let types: Vec<FactoryGroupBuilding> = buildings
                .declaration_order
                .iter()
                .filter(counted)
                .filter(|token| buildings.factory_flags.get(*token).is_some_and(counts))
                .map(|token| FactoryGroupBuilding { token: token.clone(), name: name(token) })
                .collect();
            if types.is_empty() {
                warnings.push(format!("No building in the recorded content has {flag}; its group is named by the game's group"));
            }
            for building in types.iter().filter(|building| building.name.is_none()) {
                warnings.push(format!("{}: no name in the recorded content; its title is unavailable", building.token));
            }
            FactoryGroupNames { key: key.to_owned(), flag: flag.to_owned(), buildings: types }
        })
        .collect();
    (groups, warnings)
}

fn read(catalogue: &Catalogue) -> io::Result<FactoryGroupsReply> {
    let buildings = content_reader_key_names::buildings::read(&catalogue.content.layers)?;
    let (groups, mut warnings) = groups(&buildings, |token| {
        catalogue.localisation.get(token)?;
        catalogue.localised(token, View::Plain)
    });
    warnings.extend(catalogue.verification.iter().cloned());
    warnings.extend(catalogue.refusal_lines());
    Ok(FactoryGroupsReply { groups, missing: None, warnings })
}

impl Product for FactoryGroups {
    const KEY: &'static str = "factory-groups";
    const CONTENT_DEPENDENT: bool = true;
    type Params = FactoryGroupsParams;
    type Reply = FactoryGroupsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FactoryGroupsParams::decl(), FactoryGroupBuilding::decl(), FactoryGroupNames::decl(), FactoryGroupsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, _params: &Self::Params, load: &LoadContext, _cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let named = load.content().and_then(|content| catalogue::catalogue(source, content)).and_then(|catalogue| read(&catalogue));
        Ok(named.unwrap_or_else(|error| FactoryGroupsReply {
            groups: Vec::new(),
            missing: Some(format!("Factory group names are unavailable: {error}")),
            warnings: Vec::new(),
        }))
    }
}
