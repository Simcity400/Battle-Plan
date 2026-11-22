use super::{Content, invalid};
use replay_data::rows::{TableId, VariantDefinitionRow};
use replay_frame::frame::DecodedFrame;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fmt, io,
    sync::{Arc, Mutex},
};
use viewer_runtime::memo::Keyed;
use dashboard_text::table_access::TokenNames;

const INPUTS: &[TableId] = &[TableId::VariantDefinitions, TableId::LexicalTokens];

#[derive(Clone)]
pub(super) struct Retained(Arc<Keyed<Bank>>);
impl Default for Retained {
    fn default() -> Self {
        Self(Arc::new(Keyed::new(INPUTS)))
    }
}
impl fmt::Debug for Retained {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RetainedClassification")
    }
}
impl Retained {
    pub(super) fn hour(&self, frame: &DecodedFrame) -> Option<Hour> {
        let bank = self
            .0
            .get_or_build(frame, (), |_| {
                Ok::<_, std::convert::Infallible>(Arc::new(Bank::default()))
            })
            .unwrap();
        Some(Hour {
            bank,
            local: RefCell::new(BTreeMap::new()),
        })
    }
}

#[derive(Clone, Copy)]
struct Fact {
    equipment: usize,
    archetype: usize,
    category: u64,
}
#[derive(Default)]
struct Bank(Mutex<BTreeMap<u64, Fact>>);
pub(crate) struct Hour {
    bank: Arc<Bank>,
    local: RefCell<BTreeMap<u64, Fact>>,
}
impl Hour {
    pub(super) fn classify<'a>(
        &self,
        content: &'a Content,
        variant: &VariantDefinitionRow,
        tokens: &TokenNames<'_>,
    ) -> io::Result<(&'a str, &'a str, u64)> {
        let local = self.local.borrow().get(&variant.id).copied();
        let fact = if let Some(fact) = local {
            fact
        } else {
            let held = self
                .bank
                .0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&variant.id)
                .copied();
            let fact = if let Some(fact) = held {
                fact
            } else {
                let key = content.equipment_key(tokens, variant).map_err(invalid)?;
                content.archetype(key)?;
                let equipment = content.equipment_content();
                let at = equipment.get(key).expect("resolved authored equipment");
                let row = &equipment.equipment[at];
                let fact = Fact {
                    equipment: at,
                    archetype: row.archetype.expect("validated archetype"),
                    category: equipment.category(row),
                };
                self.bank
                    .0
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(variant.id, fact);
                fact
            };
            self.local.borrow_mut().insert(variant.id, fact);
            fact
        };
        let equipment = content.equipment_content();
        Ok((
            equipment.equipment[fact.equipment]
                .name_str()
                .expect("validated equipment spelling"),
            equipment.equipment[fact.archetype]
                .name_str()
                .expect("validated archetype spelling"),
            fact.category,
        ))
    }
}
