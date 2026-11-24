
use super::{
    keywords::KeywordTables,
    leak_string,
    pe::{Anchor, Hashed, PeImage},
    verify_digest,
};
use crate::availability::{Verified, Withheld};
use iced_x86::{
    Code, Decoder, DecoderOptions, FlowControl, Instruction, InstructionInfoFactory, Mnemonic,
    OpAccess, OpKind, Register,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

const CATEGORY_ERROR: Anchor = Anchor {
    what: "the category lookup error",
    text: Hashed {
        length: 23,
        fnv: 0xe9b2cb4ff62f6abc,
    },
};
const STAT_DUPLICATE_ERROR: Anchor = Anchor {
    what: "the duplicate stat error",
    text: Hashed {
        length: 31,
        fnv: 0x47443d409978ac64,
    },
};
const STAT_MODIFIER_LABEL: Anchor = Anchor {
    what: "the stat modifier label",
    text: Hashed {
        length: 19,
        fnv: 0x95af13d6e9945da3,
    },
};
const STAT_DESIGNER_LABEL: Anchor = Anchor {
    what: "the designer stat tooltip label",
    text: Hashed {
        length: 26,
        fnv: 0x6d1684e757f1cab5,
    },
};
const STRATEGY_MISSION_ERROR: Anchor = Anchor {
    what: "the strategy mission error",
    text: Hashed {
        length: 16,
        fnv: 0xaa2e4172c73e9eb2,
    },
};
const OPERATIVE_MISSION_ERROR: Anchor = Anchor {
    what: "the operative mission error",
    text: Hashed {
        length: 35,
        fnv: 0x48d4737e76de60ea,
    },
};
const EQUIPMENT_MISSION_ERROR: Anchor = Anchor {
    what: "the equipment mission error",
    text: Hashed {
        length: 25,
        fnv: 0x7f66e3d3284d1d8c,
    },
};
const SELECTOR_TARGET_PREFIX: Anchor = Anchor {
    what: "the event target prefix",
    text: Hashed {
        length: 13,
        fnv: 0x2c8a603d59231069,
    },
};
const AIR_CHANNEL_LABEL: Anchor = Anchor {
    what: "the air view channel label",
    text: Hashed {
        length: 24,
        fnv: 0xb8cfd0731807af9c,
    },
};
const AIR_DETECTION_LABEL: Anchor = Anchor {
    what: "the air detection label",
    text: Hashed {
        length: 31,
        fnv: 0x1e6518e38c26892d,
    },
};
const INVALID_CATEGORY_ERROR: Anchor = Anchor {
    what: "the invalid category error",
    text: Hashed {
        length: 27,
        fnv: 0x9f4f7efcd0c2b930,
    },
};
const VARIANT_ICON_CHECK: Anchor = Anchor {
    what: "the variant icon offset check",
    text: Hashed {
        length: 59,
        fnv: 0xece57ceda286b113,
    },
};
const TEXT_ANCHORS: [Anchor; 12] = [
    CATEGORY_ERROR,
    STAT_DUPLICATE_ERROR,
    STAT_MODIFIER_LABEL,
    STAT_DESIGNER_LABEL,
    STRATEGY_MISSION_ERROR,
    OPERATIVE_MISSION_ERROR,
    EQUIPMENT_MISSION_ERROR,
    SELECTOR_TARGET_PREFIX,
    AIR_CHANNEL_LABEL,
    AIR_DETECTION_LABEL,
    INVALID_CATEGORY_ERROR,
    VARIANT_ICON_CHECK,
];
const FAMILY_WORDS: [Hashed; 3] = [
    Hashed {
        length: 4,
        fnv: 0x0460_dcad_9060_ad5c,
    },
    Hashed {
        length: 3,
        fnv: 0xe701_bb19_0528_8437,
    },
    Hashed {
        length: 5,
        fnv: 0xe604_6cff_7f76_9a5f,
    },
];
const FRONT_CONTROL_TYPE: Anchor = Anchor {
    what: "the front control strategy type",
    text: Hashed {
        length: 31,
        fnv: 0xa5a620d1a16562b7,
    },
};
const AIR_VIEW_TYPE: Anchor = Anchor {
    what: "the air view type",
    text: Hashed {
        length: 21,
        fnv: 0x2c190a005c10bb27,
    },
};
const RULE_TYPE: Anchor = Anchor {
    what: "the rule definition type",
    text: Hashed {
        length: 21,
        fnv: 0x601694fcf84594a5,
    },
};
const TYPE_ANCHORS: [Anchor; 3] = [FRONT_CONTROL_TYPE, AIR_VIEW_TYPE, RULE_TYPE];

const EQUIPMENT_CATEGORIES: &str = "equipment categories";
const CATEGORY_NAMES: &str = "equipment category names";
const STATS: &str = "equipment stats";
const AI_STRATEGY_TYPES: &str = "AI strategy types";
const AI_MISSIONS: &str = "AI missions";
const EQUIPMENT_MISSIONS: &str = "equipment missions";
const AI_EXECUTION_TYPES: &str = "AI execution types";
const AI_ORDER_TYPES: &str = "AI order types";
const AI_SELECTORS: &str = "AI selectors";
const RULES: &str = "power balance rules";
const AIR_GRAPH_CHANNELS: &str = "air graph channels";
const AIR_GRAPH_COLORS: &str = "air graph colours";
const AIR_GRAPH_SCALE: &str = "air graph scale";
const AIR_DETECTION_PREFIX: &str = "air detection override prefix";
pub const CATEGORY_FAMILIES: &str = "equipment category families";

pub const AIR_GRAPH_DISPLAYS: usize = 24;
const BIG_ALLOCATION: u64 = 0x1000;
const RULE_FULFILLED_ARGUMENT: u64 = 0x20;
const RULE_DEFAULT_ARGUMENT: u64 = 0x30;
const COLOR_ALPHA_ARGUMENT: u64 = 0x20;
const OUTPUT: u64 = 0x1000;
const STACK: u64 = 0x10_0000;
const FRAME: u64 = STACK + 0x100;
const HEAP: u64 = 0x30_0000;
const STRING_OBJECT: u64 = 32;

const CATEGORY_NAMES_DIGEST: &str =
    "eb08b560f635730feefc5db7504ea9a866155d79e83636d3fef519a157b19163";
const STAT_METADATA_DIGEST: &str =
    "7263a410fefbf89683dc2b99faeebb04939a4f130d27f01dcacb1d0a71b61c96";
const AI_STRATEGY_DIGEST: &str = "ad14707d0885d0332f4cfc30095c228175795a7abc1f261b6a43d69d12b71be0";
const EQUIPMENT_MISSION_DIGEST: &str =
    "d0df74fd31ef9b8b87ef863817e8b61d81c8f6cbd34916d47bc3660ae07e03d5";
const AIR_GRAPH_CHANNEL_DIGEST: &str =
    "7b8067e808557893a7d9fb1f9dc08f20c2e27fc06e93bf99a35839f567cadb94";
const CATEGORY_FAMILIES_DIGEST: &str =
    "9f9c7e69bf462bce2353642be236522424b7427691b1dba409d8a621b9aa3c63";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CategoryFamilies {
    pub land: u64,
    pub air: u64,
    pub naval: u64,
    pub ship_designer: u64,
    pub tank_designer: u64,
    pub plane_designer: u64,
}

impl CategoryFamilies {
    fn rows(&self) -> [(&'static str, u64); 6] {
        [
            ("land", self.land),
            ("air", self.air),
            ("naval", self.naval),
            ("ship_designer", self.ship_designer),
            ("tank_designer", self.tank_designer),
            ("plane_designer", self.plane_designer),
        ]
    }

    fn text(&self) -> String {
        self.rows()
            .iter()
            .map(|(name, mask)| format!("{name}\t{mask}\n"))
            .collect()
    }
}

#[derive(Debug)]
pub struct AuxiliaryTables {
    pub equipment_categories: Verified<Vec<(u32, u64, &'static str)>>,
    pub category_names: Verified<Vec<(&'static str, u64)>>,
    pub stat_metadata: Verified<Vec<(&'static str, &'static str, bool)>>,
    pub ai_strategy_types: Verified<Vec<(&'static str, u32)>>,
    pub missions: Verified<Vec<(&'static str, u32)>>,
    pub equipment_missions: Verified<Vec<(&'static str, u32)>>,
    pub execution_types: Verified<Vec<(&'static str, u32)>>,
    pub order_types: Verified<Vec<(&'static str, u32)>>,
    pub selectors: Verified<Vec<(&'static str, u32)>>,
    pub rules: Verified<Vec<(&'static str, &'static str, bool, bool)>>,
    pub air_graph_channels: Verified<[u32; AIR_GRAPH_DISPLAYS]>,
    pub air_graph_colors: Verified<[[f32; 4]; AIR_GRAPH_DISPLAYS]>,
    pub air_graph_scale: Verified<f32>,
    pub air_detection_override_prefix: Verified<&'static str>,
    pub category_families: Verified<CategoryFamilies>,
}

impl AuxiliaryTables {
    pub fn content_digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        for (part, text) in self.canonical() {
            if let Ok(text) = text {
                digest.update(part.as_bytes());
                digest.update(b"\n");
                digest.update(text.as_bytes());
                digest.update(b"\n");
            }
        }
        digest.finalize().into()
    }

    pub fn withheld(&self) -> impl Iterator<Item = &Withheld> {
        [
            self.equipment_categories.as_ref().err(),
            self.category_names.as_ref().err(),
            self.stat_metadata.as_ref().err(),
            self.ai_strategy_types.as_ref().err(),
            self.missions.as_ref().err(),
            self.equipment_missions.as_ref().err(),
            self.execution_types.as_ref().err(),
            self.order_types.as_ref().err(),
            self.selectors.as_ref().err(),
            self.rules.as_ref().err(),
            self.air_graph_channels.as_ref().err(),
            self.air_graph_colors.as_ref().err(),
            self.air_graph_scale.as_ref().err(),
            self.air_detection_override_prefix.as_ref().err(),
            self.category_families.as_ref().err(),
        ]
        .into_iter()
        .flatten()
    }

    fn canonical(&self) -> [(&'static str, Result<String, ()>); 15] {
        fn text<T>(part: &Verified<T>, render: impl FnOnce(&T) -> String) -> Result<String, ()> {
            part.as_ref().map(render).map_err(|_| ())
        }
        [
            (
                EQUIPMENT_CATEGORIES,
                text(&self.equipment_categories, |rows| {
                    let mut rows: Vec<_> =
                        rows.iter().map(|&(_, mask, name)| (mask, name)).collect();
                    rows.sort_unstable();
                    rows.iter()
                        .map(|(mask, name)| format!("{name}\t{mask}\n"))
                        .collect()
                }),
            ),
            (
                CATEGORY_NAMES,
                text(&self.category_names, |rows| category_text(rows)),
            ),
            (STATS, text(&self.stat_metadata, |rows| stat_text(rows))),
            (
                AI_STRATEGY_TYPES,
                text(&self.ai_strategy_types, |rows| enum_text(rows)),
            ),
            (AI_MISSIONS, text(&self.missions, |rows| enum_text(rows))),
            (
                EQUIPMENT_MISSIONS,
                text(&self.equipment_missions, |rows| enum_text(rows)),
            ),
            (
                AI_EXECUTION_TYPES,
                text(&self.execution_types, |rows| enum_text(rows)),
            ),
            (
                AI_ORDER_TYPES,
                text(&self.order_types, |rows| {
                    rows.iter().map(|(name, _)| format!("{name}\n")).collect()
                }),
            ),
            (AI_SELECTORS, text(&self.selectors, |rows| enum_text(rows))),
            (RULES, text(&self.rules, |rows| rule_text(rows))),
            (
                AIR_GRAPH_CHANNELS,
                text(&self.air_graph_channels, |ids| channel_text(ids)),
            ),
            (
                AIR_GRAPH_COLORS,
                text(&self.air_graph_colors, |colors| {
                    colors
                        .iter()
                        .map(|color| {
                            let [r, g, b, a] = color.map(f32::to_bits);
                            format!("{r:08x}\t{g:08x}\t{b:08x}\t{a:08x}\n")
                        })
                        .collect()
                }),
            ),
            (
                AIR_GRAPH_SCALE,
                text(&self.air_graph_scale, |scale| {
                    format!("{:08x}", scale.to_bits())
                }),
            ),
            (
                AIR_DETECTION_PREFIX,
                text(&self.air_detection_override_prefix, |prefix| {
                    (*prefix).to_owned()
                }),
            ),
            (
                CATEGORY_FAMILIES,
                text(&self.category_families, CategoryFamilies::text),
            ),
        ]
    }

    pub(crate) fn decode(image: &PeImage, keywords: &KeywordTables) -> io::Result<Self> {
        let anchors = Anchors::find(image);
        let (equipment_categories, category_names) = match part(EQUIPMENT_CATEGORIES, || {
            categories(image, &anchors, keywords)
        }) {
            Ok((rows, names)) => (Ok(rows), Ok(names)),
            Err(withheld) => (
                Err(withheld.clone()),
                Err(Withheld::new(CATEGORY_NAMES, withheld.reason)),
            ),
        };
        let strategy = part("AI strategy reader", || ai_dispatchers(image, &anchors));
        let strategy = strategy.map_err(|withheld| withheld.to_string());
        let front_control = part("front-control reader", || front_control(image, &anchors));
        let front_control = front_control.map_err(|withheld| withheld.to_string());
        Ok(Self {
            equipment_categories,
            category_names,
            stat_metadata: part(STATS, || stats(image, &anchors, keywords)),
            ai_strategy_types: part(AI_STRATEGY_TYPES, || {
                let rows = enum_projection(image, keywords, strategy.clone().map_err(invalid)?.0)?;
                verify_digest(
                    AI_STRATEGY_TYPES,
                    enum_text(&rows).as_bytes(),
                    AI_STRATEGY_DIGEST,
                )?;
                Ok(rows)
            }),
            missions: part(AI_MISSIONS, || {
                let rows = enum_projection(image, keywords, strategy.clone().map_err(invalid)?.1)?;
                distinct_values(&rows)?;
                Ok(rows)
            }),
            equipment_missions: part(EQUIPMENT_MISSIONS, || {
                equipment_missions(image, &anchors, keywords)
            }),
            execution_types: part(AI_EXECUTION_TYPES, || {
                let entry = front_control.as_ref().map_err(invalid)?.1;
                let rows = enum_projection(image, keywords, entry)?;
                distinct_values(&rows)?;
                Ok(rows)
            }),
            order_types: part(AI_ORDER_TYPES, || {
                order_projection(keywords, &front_control.as_ref().map_err(invalid)?.0)
            }),
            selectors: part(AI_SELECTORS, || selectors(image, &anchors, keywords)),
            rules: part(RULES, || rule_projection(image, &anchors, keywords)),
            air_graph_channels: part(AIR_GRAPH_CHANNELS, || air_graph_channels(image, &anchors)),
            air_graph_colors: part(AIR_GRAPH_COLORS, || air_graph_colors(image, &anchors)),
            air_graph_scale: part(AIR_GRAPH_SCALE, || air_graph_scale(image, &anchors)),
            air_detection_override_prefix: part(AIR_DETECTION_PREFIX, || {
                air_detection_prefix(image, &anchors)
            }),
            category_families: part(CATEGORY_FAMILIES, || {
                category_families(image, &anchors, keywords)
            }),
        })
    }
}

fn category_families(
    image: &PeImage,
    anchors: &Anchors,
    keywords: &KeywordTables,
) -> io::Result<CategoryFamilies> {
    let gate = image.function(first_call(image, anchors.function(VARIANT_ICON_CHECK)?)?)?;
    let designers: Vec<u64> = code(image, gate)?
        .iter()
        .filter(|i| {
            matches!(i.mnemonic(), Mnemonic::Mov | Mnemonic::Or)
                && i.op0_kind() == OpKind::Register
                && i.op0_register().size() == 8
                && i.op0_register() != Register::RSP
                && is_immediate(i, 1)
        })
        .map(|i| i.immediate(1))
        .filter(|&mask| mask != 0)
        .collect();
    let mut cases = Vec::new();
    for extent in anchors.functions(INVALID_CATEGORY_ERROR)? {
        cases.extend(family_cases(&code(image, extent)?));
    }
    let families = FAMILY_WORDS.map(|word| {
        let masks: BTreeSet<u64> = cases
            .iter()
            .filter(|&&(token, _)| {
                keywords
                    .names
                    .iter()
                    .any(|&(id, name)| id == token && word.matches(name.as_bytes()))
            })
            .map(|&(_, mask)| mask)
            .collect();
        sole(masks, "category family mask of one family word")
    });
    let [land, air, naval] = families;
    let (land, air, naval) = (land?, air?, naval?);
    let designer = |family: u64| {
        sole(
            designers.iter().copied().filter(|&mask| mask & family == mask),
            "designer category mask inside one family",
        )
    };
    let found = CategoryFamilies {
        land,
        air,
        naval,
        tank_designer: designer(land)?,
        plane_designer: designer(air)?,
        ship_designer: designer(naval)?,
    };
    if designers.len() != 3 || land & air != 0 || land & naval != 0 || air & naval != 0 {
        return Err(invalid(
            "the category families overlap or the designer gate holds other masks",
        ));
    }
    verify_digest(
        CATEGORY_FAMILIES,
        found.text().as_bytes(),
        CATEGORY_FAMILIES_DIGEST,
    )?;
    Ok(found)
}

fn family_cases(code: &[Instruction]) -> Vec<(u32, u64)> {
    let mut found = Vec::new();
    for (at, start) in code.iter().enumerate() {
        if start.mnemonic() != Mnemonic::Sub
            || start.op0_kind() != OpKind::Register
            || start.op0_register().size() != 4
            || !is_immediate(start, 1)
        {
            continue;
        }
        let register = start.op0_register();
        let mut value = start.immediate(1) as u32;
        let mut pending = Some(value);
        for i in &code[at + 1..] {
            let on_register = i.op0_kind() == OpKind::Register
                && i.op0_register() == register
                && is_immediate(i, 1);
            match i.mnemonic() {
                Mnemonic::Je => {
                    let Some(token) = pending.take() else { break };
                    let target = i.near_branch_target();
                    match code.iter().find(|load| load.ip() == target) {
                        Some(load) if load.code() == Code::Mov_r64_imm64 => {
                            found.push((token, load.immediate64()));
                        }
                        _ => break,
                    }
                }
                Mnemonic::Sub if on_register => {
                    value = value.wrapping_add(i.immediate(1) as u32);
                    pending = Some(value);
                }
                Mnemonic::Cmp if on_register => {
                    pending = Some(value.wrapping_add(i.immediate(1) as u32));
                }
                _ => break,
            }
        }
    }
    found
}

fn part<T>(name: &'static str, decode: impl FnOnce() -> io::Result<T>) -> Verified<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode)) {
        Ok(result) => result.map_err(|error| Withheld::new(name, error)),
        Err(_) => Err(Withheld::new(
            name,
            "its decoder stopped on an unexpected image",
        )),
    }
}

struct Anchors<'a> {
    image: &'a PeImage,
    texts: Vec<(Anchor, Vec<u32>)>,
    types: Vec<(Anchor, io::Result<(u32, Vec<u32>)>)>,
}

impl<'a> Anchors<'a> {
    fn find(image: &'a PeImage) -> Self {
        let (strings, vtables) = std::thread::scope(|scope| {
            let vtables = scope.spawn(|| TYPE_ANCHORS.map(|anchor| primary_vtable(image, anchor)));
            let strings = image.find_hashed_strings(&TEXT_ANCHORS.map(|anchor| anchor.text));
            let vtables = vtables
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            (strings, vtables)
        });
        let targets: Vec<u32> = strings
            .iter()
            .flatten()
            .copied()
            .chain(
                vtables
                    .iter()
                    .filter_map(|vtable| vtable.as_ref().ok().copied()),
            )
            .collect();
        let mut references = image.references_to_each(&targets).into_iter();
        let texts = TEXT_ANCHORS
            .iter()
            .zip(&strings)
            .map(|(&text, copies)| {
                (
                    text,
                    copies
                        .iter()
                        .flat_map(|_| references.next().unwrap())
                        .collect(),
                )
            })
            .collect();
        let types = TYPE_ANCHORS
            .into_iter()
            .zip(vtables)
            .map(|(name, vtable)| {
                (
                    name,
                    vtable.map(|vtable| (vtable, references.next().unwrap())),
                )
            })
            .collect();
        Self {
            image,
            texts,
            types,
        }
    }

    fn references(&self, text: Anchor) -> io::Result<&[u32]> {
        let (_, references) = self
            .texts
            .iter()
            .find(|(anchor, _)| *anchor == text)
            .expect("every text anchor is searched");
        if references.is_empty() {
            return Err(invalid(format!("no code references {}", text.what)));
        }
        Ok(references)
    }

    fn holders(&self, references: &[u32]) -> io::Result<Vec<(u32, u32)>> {
        let mut holders = BTreeSet::new();
        for &reference in references {
            holders.insert(self.image.function(reference)?);
        }
        Ok(holders.into_iter().collect())
    }

    fn functions(&self, text: Anchor) -> io::Result<Vec<(u32, u32)>> {
        self.holders(self.references(text)?)
    }

    fn function(&self, text: Anchor) -> io::Result<(u32, u32)> {
        sole(
            self.functions(text)?,
            &format!("function referencing {}", text.what),
        )
    }

    fn vtable(&self, name: Anchor) -> io::Result<(u32, &[u32])> {
        let (_, found) = self
            .types
            .iter()
            .find(|(anchor, _)| *anchor == name)
            .expect("every type anchor is searched");
        match found {
            Ok((vtable, references)) => Ok((*vtable, references)),
            Err(error) => Err(invalid(error.to_string())),
        }
    }
}

fn primary_vtable(image: &PeImage, name: Anchor) -> io::Result<u32> {
    let vtables: Vec<u32> = image
        .classes_named(name.text)
        .filter(|class| class.offset == 0)
        .map(|class| class.vtable)
        .collect();
    sole(vtables, &format!("vtable of {}", name.what))
}

fn sole<T>(candidates: impl IntoIterator<Item = T>, what: &str) -> io::Result<T> {
    let mut candidates = candidates.into_iter();
    match (candidates.next(), candidates.next()) {
        (Some(found), None) => Ok(found),
        (None, _) => Err(invalid(format!("no {what} found"))),
        (Some(_), Some(_)) => Err(invalid(format!("several candidates for the {what}"))),
    }
}

fn code(image: &PeImage, (start, end): (u32, u32)) -> io::Result<Vec<Instruction>> {
    let bytes = image.bytes(start, (end - start) as usize)?;
    let mut decoder = Decoder::with_ip(
        64,
        bytes,
        image.base + u64::from(start),
        DecoderOptions::NONE,
    );
    let mut code = Vec::new();
    while decoder.can_decode() {
        let instruction = decoder.decode();
        if instruction.is_invalid() {
            break;
        }
        code.push(instruction);
    }
    Ok(code)
}

fn call_target(image: &PeImage, i: &Instruction) -> Option<u32> {
    (i.mnemonic() == Mnemonic::Call && i.op0_kind() == OpKind::NearBranch64)
        .then(|| image.va_to_rva(i.near_branch_target()).ok())
        .flatten()
}

fn rip_target(image: &PeImage, i: &Instruction) -> Option<u32> {
    i.is_ip_rel_memory_operand()
        .then(|| image.va_to_rva(i.ip_rel_memory_address()).ok())
        .flatten()
}

fn is_immediate(i: &Instruction, operand: u32) -> bool {
    operand < i.op_count() && immediate(i, operand).is_ok()
}

fn optional_calls(image: &PeImage, code: &[Instruction]) -> Vec<(u64, u32)> {
    let mut found = Vec::new();
    for (index, i) in code.iter().enumerate() {
        let Some(target) = call_target(image, i) else {
            continue;
        };
        let Some(test) = code[index + 1..]
            .iter()
            .find(|next| next.mnemonic() != Mnemonic::Nop)
        else {
            continue;
        };
        if test.mnemonic() != Mnemonic::Cmp
            || test.op0_kind() != OpKind::Memory
            || test.memory_size().size() != 1
            || test.memory_index() != Register::None
            || immediate(test, 1).ok() != Some(0)
        {
            continue;
        }
        let value = if test.memory_base() == Register::RAX {
            Some(0)
        } else {
            code[..index]
                .iter()
                .rev()
                .take_while(|previous| previous.mnemonic() != Mnemonic::Call)
                .find(|previous| {
                    previous.op0_kind() == OpKind::Register
                        && previous.op0_register().full_register() == Register::RCX
                })
                .filter(|lea| {
                    lea.mnemonic() == Mnemonic::Lea
                        && lea.memory_base() == test.memory_base()
                        && lea.memory_index() == Register::None
                })
                .map(Instruction::memory_displacement64)
        };
        if value.is_some_and(|value| test.memory_displacement64() == value.wrapping_add(4)) {
            found.push((i.ip(), target));
        }
    }
    found
}

fn slots(image: &PeImage, vtable: u32) -> Vec<u32> {
    let mut slots = Vec::new();
    let mut at = vtable;
    while let Ok(pointer) = image.u64(at)
        && let Ok(slot) = image.va_to_rva(pointer)
        && image
            .executable_ranges()
            .iter()
            .any(|&(start, end)| start <= slot && slot < end)
    {
        slots.push(slot);
        at += 8;
    }
    slots
}

fn slot_functions(image: &PeImage, vtable: u32) -> io::Result<Vec<Vec<Instruction>>> {
    let mut functions = Vec::new();
    for slot in slots(image, vtable) {
        if let Ok(extent) = image.function(slot)
            && extent.0 == slot
        {
            functions.push(code(image, extent)?);
        }
    }
    Ok(functions)
}

fn first_call(image: &PeImage, extent: (u32, u32)) -> io::Result<u32> {
    code(image, extent)?
        .iter()
        .find_map(|i| call_target(image, i))
        .ok_or_else(|| invalid("the anchored function calls nothing"))
}

fn categories(
    image: &PeImage,
    anchors: &Anchors,
    keywords: &KeywordTables,
) -> io::Result<(Vec<(u32, u64, &'static str)>, Vec<(&'static str, u64)>)> {
    let mut dispatch = Dispatch::new(image, first_call(image, anchors.function(CATEGORY_ERROR)?)?)?;
    let unnamed = dispatch.select(Register::ECX, 0, false)?.result;
    if !unnamed.is_power_of_two() {
        return Err(invalid("the empty token's category is not one bit"));
    }
    let mut rows = Vec::new();
    let mut empty = Vec::new();
    for &(token, name) in &keywords.names {
        let outcome = dispatch.select(Register::ECX, token, false)?;
        match outcome.result {
            0 if outcome.matched => empty.push(name),
            0 => {}
            mask if mask == unnamed => {}
            mask => rows.push((token, mask, name)),
        }
    }
    let empty = sole(empty, "keyword naming no category")?;
    rows.sort_by_key(|row| row.0);
    let mut names: Vec<_> = rows.iter().map(|&(_, mask, name)| (name, mask)).collect();
    names.sort_by_key(|row| row.1);
    names.insert(0, (empty, 0));
    verify_digest(
        CATEGORY_NAMES,
        category_text(&names).as_bytes(),
        CATEGORY_NAMES_DIGEST,
    )?;
    Ok((rows, names))
}

fn stats(
    image: &PeImage,
    anchors: &Anchors,
    keywords: &KeywordTables,
) -> io::Result<Vec<(&'static str, &'static str, bool)>> {
    let mut dispatch = Dispatch::new(
        image,
        first_call(image, anchors.function(STAT_DUPLICATE_ERROR)?)?,
    )?;
    let count = dispatch.select(Register::ECX, 0, false)?.result;
    let mut indexed = BTreeMap::new();
    for &(token, name) in &keywords.names {
        let index = dispatch.select(Register::ECX, token, false)?.result;
        if index != count && (index > count || indexed.insert(index, name).is_some()) {
            return Err(invalid("a stat index repeats or exceeds the stat count"));
        }
    }
    if indexed.len() as u64 != count {
        return Err(invalid(format!(
            "{} stats are named, but the lookup counts {count}",
            indexed.len()
        )));
    }
    let labels = stat_labels(image, anchors)?;
    let polarity = stat_polarity(image, anchors)?;
    let mut rows = Vec::new();
    for (index, name) in indexed {
        let index = index as u32;
        let label = index
            .checked_mul(8)
            .and_then(|offset| labels.checked_add(offset))
            .ok_or_else(|| invalid("stat label table overflows"))?;
        let label = image.string(image.va_to_rva(image.u64(label)?)?)?;
        let positive = image.u8(polarity
            .checked_add(index)
            .ok_or_else(|| invalid("stat polarity table overflows"))?)?;
        if positive > 1 {
            return Err(invalid("stat polarity is not boolean"));
        }
        rows.push((name, leak_string(label), positive != 0));
    }
    verify_digest(STATS, stat_text(&rows).as_bytes(), STAT_METADATA_DIGEST)?;
    Ok(rows)
}

fn stat_labels(image: &PeImage, anchors: &Anchors) -> io::Result<u32> {
    let writer = code(image, anchors.function(STAT_MODIFIER_LABEL)?)?;
    let tables: BTreeSet<u32> = writer
        .windows(2)
        .filter(|pair| {
            let [table, load] = pair else { return false };
            table.mnemonic() == Mnemonic::Lea
                && load.mnemonic() == Mnemonic::Mov
                && load.op1_kind() == OpKind::Memory
                && load.memory_base() == table.op0_register()
                && load.memory_index_scale() == 8
                && load.memory_size().size() == 8
        })
        .filter_map(|pair| rip_target(image, &pair[0]))
        .collect();
    sole(tables, "stat label table")
}

fn stat_polarity(image: &PeImage, anchors: &Anchors) -> io::Result<u32> {
    let tooltip = code(image, anchors.function(STAT_DESIGNER_LABEL)?)?;
    let callees: BTreeSet<u32> = tooltip
        .iter()
        .filter_map(|i| call_target(image, i))
        .collect();
    let mut tables = BTreeSet::new();
    for callee in callees {
        let Ok(bytes) = image.bytes(callee, 32) else {
            continue;
        };
        let mut decoder = Decoder::with_ip(
            64,
            bytes,
            image.base + u64::from(callee),
            DecoderOptions::NONE,
        );
        let leaf: Vec<_> = (0..4).map(|_| decoder.decode()).collect();
        let [index, table, load, ret] = &leaf[..] else {
            continue;
        };
        let read = [load.memory_base(), load.memory_index()];
        if index.mnemonic() == Mnemonic::Movsxd
            && index.op1_kind() == OpKind::Register
            && index.op1_register() == Register::ECX
            && table.mnemonic() == Mnemonic::Lea
            && load.mnemonic() == Mnemonic::Movzx
            && load.op1_kind() == OpKind::Memory
            && load.memory_size().size() == 1
            && load.memory_index_scale() == 1
            && read.contains(&index.op0_register())
            && read.contains(&table.op0_register())
            && ret.mnemonic() == Mnemonic::Ret
            && let Some(found) = rip_target(image, table)
        {
            tables.insert(found);
        }
    }
    sole(tables, "stat polarity table")
}

fn ai_dispatchers(image: &PeImage, anchors: &Anchors) -> io::Result<(u32, u32)> {
    let extent = anchors.function(STRATEGY_MISSION_ERROR)?;
    let reader = code(image, extent)?;
    let lookups = optional_calls(image, &reader);
    let report = sole(
        anchors
            .references(OPERATIVE_MISSION_ERROR)?
            .iter()
            .filter(|&&at| extent.0 <= at && at < extent.1),
        "operative mission error in the strategy reader",
    )?;
    let report = image.base + u64::from(*report);
    let missions = lookups
        .iter()
        .filter(|&&(site, _)| site < report)
        .next_back()
        .ok_or_else(|| invalid("no optional lookup precedes the operative mission error"))?
        .1;
    let strategy = sole(
        lookups
            .iter()
            .map(|&(_, target)| target)
            .filter(|&target| target != missions)
            .collect::<BTreeSet<_>>(),
        "strategy type lookup",
    )?;
    Ok((strategy, missions))
}

fn front_control(image: &PeImage, anchors: &Anchors) -> io::Result<(Vec<Instruction>, u32)> {
    let (vtable, _) = anchors.vtable(FRONT_CONTROL_TYPE)?;
    let mut found = Vec::new();
    for reader in slot_functions(image, vtable)? {
        for run in keyword_runs(&reader) {
            found.push((run, reader.clone()));
        }
    }
    let (run, reader) = sole(found, "order type comparison in the front-control reader")?;
    let execution = sole(
        optional_calls(image, &reader)
            .into_iter()
            .map(|(_, target)| target)
            .collect::<BTreeSet<_>>(),
        "execution type lookup",
    )?;
    Ok((run, execution))
}

fn keyword_runs(code: &[Instruction]) -> Vec<Vec<Instruction>> {
    let mut runs = Vec::new();
    let mut index = 0;
    while index + 1 < code.len() {
        let mut run = Vec::new();
        let mut join = None;
        while let [test, branch, ..] = &code[index..]
            && matches!(test.mnemonic(), Mnemonic::Sub | Mnemonic::Cmp)
            && test.op0_kind() == OpKind::Register
            && test.op0_register() == Register::ECX
            && is_immediate(test, 1)
            && branch.mnemonic() == Mnemonic::Je
            && join.is_none_or(|join| join == branch.near_branch_target())
        {
            join = Some(branch.near_branch_target());
            run.push(*test);
            index += 2;
        }
        if run.is_empty() {
            index += 1;
        } else if run.len() >= 2 {
            runs.push(run);
        }
    }
    runs
}

fn order_projection(
    keywords: &KeywordTables,
    run: &[Instruction],
) -> io::Result<Vec<(&'static str, u32)>> {
    let mut base = 0_u32;
    let mut rows = Vec::new();
    for instruction in run {
        let token = match instruction.mnemonic() {
            Mnemonic::Sub => {
                base = base
                    .checked_add(immediate(instruction, 1)? as u32)
                    .ok_or_else(|| invalid("order token overflows"))?;
                base
            }
            _ => base
                .checked_add(immediate(instruction, 1)? as u32)
                .ok_or_else(|| invalid("order token overflows"))?,
        };
        let name = keywords
            .name(token)
            .ok_or_else(|| invalid("an order type is not a keyword"))?;
        rows.push((name, token));
    }
    rows.sort_by_key(|row| row.0);
    distinct_values(&rows)?;
    Ok(rows)
}

fn selectors(
    image: &PeImage,
    anchors: &Anchors,
    keywords: &KeywordTables,
) -> io::Result<Vec<(&'static str, u32)>> {
    let mut found = Vec::new();
    for extent in anchors.functions(SELECTOR_TARGET_PREFIX)? {
        let reader = code(image, extent)?;
        if let Some(chain) = selector_chain(&reader)
            && let Ok(rows) = selector_projection(keywords, chain)
            && !rows.is_empty()
            && rows.iter().all(|(_, mask)| mask.is_power_of_two())
            && distinct_values(&rows).is_ok()
        {
            found.push(rows);
        }
    }
    sole(found, "selector chain")
}

fn selector_chain(code: &[Instruction]) -> Option<&[Instruction]> {
    let is_test = |at: usize| {
        matches!(&code[at..], [test, branch, ..]
            if test.mnemonic() == Mnemonic::Cmp
                && test.op0_kind() == OpKind::Register
                && test.op0_register() == Register::EAX
                && is_immediate(test, 1)
                && branch.mnemonic() == Mnemonic::Jne)
    };
    let start = (0..code.len()).find(|&at| is_test(at))?;
    let mut at = start;
    while is_test(at) {
        let next = code
            .binary_search_by_key(&code[at + 1].near_branch_target(), Instruction::ip)
            .ok()?;
        if next <= at {
            return None;
        }
        at = next;
    }
    Some(&code[start..at])
}

fn selector_projection(
    keywords: &KeywordTables,
    code: &[Instruction],
) -> io::Result<Vec<(&'static str, u32)>> {
    let mut pending = None;
    let mut rows = Vec::new();
    for (index, instruction) in code.iter().enumerate() {
        if instruction.mnemonic() == Mnemonic::Cmp && instruction.op0_register() == Register::EAX {
            if pending.is_some() {
                return Err(invalid("AI selectors: unmatched comparison"));
            }
            if code
                .get(index + 1)
                .is_none_or(|next| next.mnemonic() != Mnemonic::Jne)
            {
                return Err(invalid("AI selectors: comparison branch differs"));
            }
            pending = Some(immediate(instruction, 1)? as u32);
        } else if instruction.op0_register() == Register::R8D
            && matches!(instruction.mnemonic(), Mnemonic::Or | Mnemonic::Bts)
        {
            let token = pending
                .take()
                .ok_or_else(|| invalid("AI selectors: result has no comparison"))?;
            let operand = immediate(instruction, 1)? as u32;
            let mask = if instruction.mnemonic() == Mnemonic::Bts {
                1_u32
                    .checked_shl(operand)
                    .ok_or_else(|| invalid("AI selectors: bit exceeds result width"))?
            } else {
                operand
            };
            let name = keywords
                .name(token)
                .ok_or_else(|| invalid("AI selectors: keyword is missing"))?;
            rows.push((name, mask));
        }
    }
    if pending.is_some() {
        return Err(invalid("AI selectors: result is missing"));
    }
    rows.sort_by_key(|row| row.0);
    Ok(rows)
}

fn equipment_missions(
    image: &PeImage,
    anchors: &Anchors,
    keywords: &KeywordTables,
) -> io::Result<Vec<(&'static str, u32)>> {
    let reader = code(image, anchors.function(EQUIPMENT_MISSION_ERROR)?)?;
    let entry = reader
        .iter()
        .find(|i| {
            i.mnemonic() == Mnemonic::Cmp
                && i.op0_kind() == OpKind::Register
                && i.op0_register().size() == 4
                && is_immediate(i, 1)
        })
        .ok_or_else(|| invalid("the mission reader tests no keyword"))?;
    let mut jumps = BTreeMap::<u64, usize>::new();
    for i in &reader {
        if i.mnemonic() == Mnemonic::Jmp && i.op0_kind() == OpKind::NearBranch64 {
            *jumps.entry(i.near_branch_target()).or_default() += 1;
        }
    }
    let join = sole(
        jumps
            .into_iter()
            .filter(|&(_, count)| count > 1)
            .map(|(target, _)| target),
        "mission result join",
    )?;
    let mut dispatch = Dispatch::new(image, image.va_to_rva(entry.ip())?)?;
    let mut terminal = Instruction::new();
    terminal.set_code(Code::Retnq);
    dispatch.put(join, terminal);
    let mut rows = Vec::new();
    for &(token, name) in &keywords.names {
        let value = dispatch.select(entry.op0_register(), token, false)?.result as u32;
        if value != 0 {
            rows.push((name, value));
        }
    }
    if rows.is_empty() || !rows.iter().all(|(_, bit)| bit.is_power_of_two()) {
        return Err(invalid("a mission is not one bit"));
    }
    distinct_values(&rows)?;
    rows.sort_unstable_by_key(|row| row.0);
    verify_digest(
        EQUIPMENT_MISSIONS,
        enum_text(&rows).as_bytes(),
        EQUIPMENT_MISSION_DIGEST,
    )?;
    Ok(rows)
}

fn air_graph_channels(image: &PeImage, anchors: &Anchors) -> io::Result<[u32; AIR_GRAPH_DISPLAYS]> {
    let (vtable, _) = anchors.vtable(AIR_VIEW_TYPE)?;
    let mut tables = BTreeSet::new();
    for function in slot_functions(image, vtable)? {
        for pair in function.windows(2) {
            let [table, load] = pair else { continue };
            if table.mnemonic() == Mnemonic::Lea
                && load.mnemonic() == Mnemonic::Mov
                && load.op1_kind() == OpKind::Memory
                && load.memory_base() == table.op0_register()
                && load.memory_index_scale() == 4
                && load.memory_size().size() == 4
                && let Some(found) = rip_target(image, table)
            {
                tables.insert(found);
            }
        }
    }
    let table = sole(tables, "air graph channel table")?;
    let mut channels = [0; AIR_GRAPH_DISPLAYS];
    for (display, channel) in channels.iter_mut().enumerate() {
        *channel = image.u32(table + display as u32 * 4)?;
    }
    verify_digest(
        AIR_GRAPH_CHANNELS,
        channel_text(&channels).as_bytes(),
        AIR_GRAPH_CHANNEL_DIGEST,
    )?;
    Ok(channels)
}

fn air_graph_colors(
    image: &PeImage,
    anchors: &Anchors,
) -> io::Result<[[f32; 4]; AIR_GRAPH_DISPLAYS]> {
    let (_, references) = anchors.vtable(AIR_VIEW_TYPE)?;
    let mut found = Vec::new();
    for extent in anchors.holders(references)? {
        if let Some(colors) = display_colors(image, &code(image, extent)?)? {
            found.push(colors);
        }
    }
    let colors = sole(found, "air graph colour construction")?;
    if !colors
        .iter()
        .flatten()
        .all(|component| (0.0..=1.0).contains(component))
    {
        return Err(invalid("a colour component lies outside 0 to 1"));
    }
    Ok(colors)
}

fn may_write_alpha(info: &mut InstructionInfoFactory, i: &Instruction) -> bool {
    let written = |access: OpAccess| {
        matches!(
            access,
            OpAccess::Write | OpAccess::CondWrite | OpAccess::ReadWrite | OpAccess::ReadCondWrite
        )
    };
    if i.flow_control() == FlowControl::Call {
        return false;
    }
    let alpha_store = i.mnemonic() == Mnemonic::Movss
        && i.op0_kind() == OpKind::Memory
        && i.memory_base() == Register::RSP
        && i.memory_index() == Register::None
        && i.memory_displacement64() == COLOR_ALPHA_ARGUMENT;
    let details = info.info(i);
    details.used_registers().iter().any(|used| {
        written(used.access()) && used.register().full_register() == Register::RSP
    }) || (!alpha_store
        && details.used_memory().iter().any(|used| {
            written(used.access())
                && (used.base() != Register::RSP
                    || used.index() != Register::None
                    || (used.displacement() < COLOR_ALPHA_ARGUMENT + 4
                        && COLOR_ALPHA_ARGUMENT < used.displacement() + used.memory_size().size() as u64))
        }))
}

fn display_colors(
    image: &PeImage,
    code: &[Instruction],
) -> io::Result<Option<[[f32; 4]; AIR_GRAPH_DISPLAYS]>> {
    let mut values = [None::<f32>; 32];
    let mut alpha = None;
    let mut constructors = BTreeMap::new();
    let mut entries: Vec<(Register, u64, [f32; 4])> = Vec::new();
    let xmm = |register: Register| register.is_xmm().then(|| register.number());
    let mut info = InstructionInfoFactory::new();
    for (index, i) in code.iter().enumerate() {
        let target = (i.op0_kind() == OpKind::Register)
            .then(|| xmm(i.op0_register()))
            .flatten();
        if may_write_alpha(&mut info, i) {
            alpha = None;
        }
        match i.mnemonic() {
            Mnemonic::Xorps
                if i.op1_kind() == OpKind::Register && i.op0_register() == i.op1_register() =>
            {
                if let Some(target) = target {
                    values[target] = Some(0.0);
                }
            }
            Mnemonic::Movss | Mnemonic::Movaps if target.is_some() => {
                values[target.unwrap()] = match i.op1_kind() {
                    OpKind::Register => xmm(i.op1_register()).and_then(|source| values[source]),
                    OpKind::Memory if i.memory_size().size() == 4 => rip_target(image, i)
                        .map(|constant| image.u32(constant).map(f32::from_bits))
                        .transpose()?,
                    _ => None,
                };
            }
            Mnemonic::Movss
                if i.op0_kind() == OpKind::Memory
                    && i.memory_base() == Register::RSP
                    && i.memory_index() == Register::None
                    && i.memory_displacement64() == COLOR_ALPHA_ARGUMENT =>
            {
                alpha = xmm(i.op1_register()).and_then(|source| values[source]);
            }
            Mnemonic::Call => {
                let constructor = call_target(image, i).and_then(|callee| {
                    *constructors
                        .entry(callee)
                        .or_insert_with(|| color_constructor(image, callee))
                });
                if let Some(offset) = constructor
                    && let (Some(r), Some(g), Some(b), Some(a)) =
                        (values[1], values[2], values[3], alpha)
                    && let [load, store, ..] = &code[index + 1..]
                    && load.mnemonic() == Mnemonic::Movups
                    && load.op1_kind() == OpKind::Memory
                    && load.memory_base() == Register::RAX
                    && load.memory_index() == Register::None
                    && load.memory_displacement64() == offset
                    && store.mnemonic() == Mnemonic::Movups
                    && store.op0_kind() == OpKind::Memory
                    && store.memory_index() == Register::None
                    && store.op1_kind() == OpKind::Register
                    && store.op1_register() == load.op0_register()
                {
                    entries.push((
                        store.memory_base(),
                        store.memory_displacement64(),
                        [r, g, b, a],
                    ));
                }
                for volatile in &mut values[..6] {
                    *volatile = None;
                }
                if constructor.is_none() {
                    alpha = None;
                }
            }
            _ => {
                if let Some(target) = target {
                    values[target] = None;
                }
            }
        }
    }
    let mut arrays = BTreeMap::<Register, Vec<(u64, [f32; 4])>>::new();
    for (base, offset, color) in entries {
        arrays.entry(base).or_default().push((offset, color));
    }
    let mut found = Vec::new();
    for (_, mut array) in arrays {
        array.sort_by_key(|&(offset, _)| offset);
        let stride = array.get(1).map(|second| second.0.wrapping_sub(array[0].0));
        if array.len() == AIR_GRAPH_DISPLAYS
            && stride.is_some_and(|stride| stride != 0)
            && array
                .windows(2)
                .all(|pair| Some(pair[1].0.wrapping_sub(pair[0].0)) == stride)
        {
            found.push(std::array::from_fn(|display| array[display].1));
        }
    }
    match found.len() {
        0 => Ok(None),
        1 => Ok(found.pop()),
        _ => Err(invalid("several colour arrays of the display count")),
    }
}

fn color_constructor(image: &PeImage, callee: u32) -> Option<u64> {
    let bytes = image.bytes(callee, 64).ok()?;
    let mut decoder = Decoder::with_ip(
        64,
        bytes,
        image.base + u64::from(callee),
        DecoderOptions::NONE,
    );
    let arguments = [Register::XMM1, Register::XMM2, Register::XMM3];
    let mut alpha = None;
    let mut stores = BTreeMap::new();
    let mut vtable_loaded = false;
    let mut returns_object = false;
    let mut pointer_stores = Vec::new();
    for _ in 0..12 {
        let i = decoder.decode();
        let register_operand = |operand: u32| i.op_kind(operand) == OpKind::Register;
        match i.mnemonic() {
            Mnemonic::Ret => {
                let offset = *stores.get(&Register::XMM1)?;
                let at = |register: Register, component: u64| {
                    stores.get(&register) == Some(&(offset + component * 4))
                };
                let clear = pointer_stores
                    .iter()
                    .all(|&at: &u64| at + 8 <= offset || offset + 16 <= at);
                return (returns_object
                    && at(Register::XMM2, 1)
                    && at(Register::XMM3, 2)
                    && at(alpha?, 3)
                    && clear)
                    .then_some(offset);
            }
            Mnemonic::Movss
                if register_operand(0)
                    && alpha.is_none()
                    && !arguments.contains(&i.op0_register())
                    && i.op1_kind() == OpKind::Memory
                    && i.memory_base() == Register::RSP
                    && i.memory_index() == Register::None
                    && i.memory_displacement64() == COLOR_ALPHA_ARGUMENT + 8 =>
            {
                alpha = Some(i.op0_register());
            }
            Mnemonic::Movss
                if i.op0_kind() == OpKind::Memory
                    && register_operand(1)
                    && i.memory_base() == Register::RCX
                    && i.memory_index() == Register::None
                    && (arguments.contains(&i.op1_register()) || alpha == Some(i.op1_register()))
                    && !stores.contains_key(&i.op1_register()) =>
            {
                stores.insert(i.op1_register(), i.memory_displacement64());
            }
            Mnemonic::Mov
                if register_operand(0)
                    && register_operand(1)
                    && i.op0_register() == Register::RAX
                    && i.op1_register() == Register::RCX =>
            {
                vtable_loaded = false;
                returns_object = true;
            }
            Mnemonic::Lea
                if register_operand(0)
                    && i.op0_register() == Register::RAX
                    && i.is_ip_rel_memory_operand() =>
            {
                vtable_loaded = true;
                returns_object = false;
            }
            Mnemonic::Mov
                if vtable_loaded
                    && i.op0_kind() == OpKind::Memory
                    && register_operand(1)
                    && i.op1_register() == Register::RAX
                    && i.memory_base() == Register::RCX
                    && i.memory_index() == Register::None =>
            {
                pointer_stores.push(i.memory_displacement64());
            }
            _ => return None,
        }
    }
    None
}

fn air_graph_scale(image: &PeImage, anchors: &Anchors) -> io::Result<f32> {
    let writer = code(image, anchors.function(AIR_CHANNEL_LABEL)?)?;
    let constants: BTreeSet<u32> = writer
        .iter()
        .filter(|i| i.mnemonic() == Mnemonic::Movss && i.op0_kind() == OpKind::Register)
        .filter_map(|i| rip_target(image, i))
        .collect();
    let scale = f32::from_bits(image.u32(sole(constants, "air graph scale constant")?)?);
    if !scale.is_finite() || scale <= 0.0 {
        return Err(invalid("the air graph scale is not a positive number"));
    }
    Ok(scale)
}

fn air_detection_prefix(image: &PeImage, anchors: &Anchors) -> io::Result<&'static str> {
    let writer = code(image, anchors.function(AIR_DETECTION_LABEL)?)?;
    let mut found = BTreeSet::new();
    for (index, i) in writer.iter().enumerate() {
        if i.mnemonic() != Mnemonic::Lea {
            continue;
        }
        let Some(text) = rip_target(image, i).and_then(|at| image.string(at).ok()) else {
            continue;
        };
        let length = text.len() as u64;
        let near = &writer[index.saturating_sub(2)..(index + 3).min(writer.len())];
        if !text.is_empty()
            && text
                .bytes()
                .all(|byte| byte.is_ascii_graphic() || byte == b' ')
            && near.iter().any(|other| {
                other.mnemonic() == Mnemonic::Mov
                    && other.op_count() == 2
                    && immediate(other, 1).ok() == Some(length)
            })
        {
            found.insert(text);
        }
    }
    Ok(leak_string(sole(found, "air detection override prefix")?))
}

fn distinct_values(rows: &[(&str, u32)]) -> io::Result<()> {
    let names: BTreeSet<_> = rows.iter().map(|(name, _)| name).collect();
    let values: BTreeSet<_> = rows.iter().map(|(_, value)| value).collect();
    if rows.is_empty() || names.len() != rows.len() || values.len() != rows.len() {
        return Err(invalid("rows are empty or repeat a name or value"));
    }
    Ok(())
}

fn boolean(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

fn category_text(rows: &[(&str, u64)]) -> String {
    rows.iter()
        .map(|(name, mask)| format!("{name}\t{mask}\n"))
        .collect()
}

fn stat_text(rows: &[(&str, &str, bool)]) -> String {
    rows.iter()
        .map(|(name, label, polarity)| format!("{name}\t{label}\t{}\n", boolean(*polarity)))
        .collect()
}

fn enum_text(rows: &[(&str, u32)]) -> String {
    rows.iter()
        .map(|(name, id)| format!("{name}\t{id}\n"))
        .collect()
}

fn rule_text(rows: &[(&str, &str, bool, bool)]) -> String {
    rows.iter()
        .map(|(name, label, default, fulfilled)| {
            format!(
                "{name}\t{label}\t{}\t{}\n",
                boolean(*default),
                boolean(*fulfilled)
            )
        })
        .collect()
}

fn channel_text(ids: &[u32]) -> String {
    ids.iter().map(|id| format!("{id}\n")).collect()
}

fn enum_projection(
    image: &PeImage,
    keywords: &KeywordTables,
    entry: u32,
) -> io::Result<Vec<(&'static str, u32)>> {
    let mut dispatch = Dispatch::new(image, entry)?;
    let mut rows = Vec::new();
    for &(token, name) in &keywords.names {
        let writes = dispatch.select(Register::EDX, token, true)?.writes;
        match writes.get(&(OUTPUT + 4)) {
            Some(1) => rows.push((
                name,
                *writes
                    .get(&OUTPUT)
                    .ok_or_else(|| invalid("AI enum: value is missing"))? as u32,
            )),
            Some(0) => {}
            _ => return Err(invalid("AI enum: validity is missing or invalid")),
        }
    }
    rows.sort_by_key(|row| row.0);
    Ok(rows)
}

struct Outcome {
    result: u64,
    writes: BTreeMap<u64, u64>,
    matched: bool,
}

struct Dispatch<'a> {
    image: &'a PeImage,
    entry: u64,
    start: u64,
    bytes: &'a [u8],
    code: Vec<Instruction>,
}

impl<'a> Dispatch<'a> {
    fn new(image: &'a PeImage, entry: u32) -> io::Result<Self> {
        let (start, end) = match image.function(entry) {
            Ok(extent) => extent,
            Err(_) => leaf_extent(image, entry)?,
        };
        Ok(Self {
            image,
            entry: image.base + u64::from(entry),
            start: image.base + u64::from(start),
            bytes: image.bytes(start, (end - start) as usize)?,
            code: Vec::new(),
        })
    }

    fn instruction(&mut self, ip: u64) -> io::Result<Instruction> {
        match self.code.binary_search_by_key(&ip, Instruction::ip) {
            Ok(index) => Ok(self.code[index]),
            Err(index) => {
                let offset = ip
                    .checked_sub(self.start)
                    .and_then(|offset| usize::try_from(offset).ok())
                    .filter(|&offset| offset < self.bytes.len())
                    .ok_or_else(|| invalid("selector: branch leaves its function"))?;
                let instruction =
                    Decoder::with_ip(64, &self.bytes[offset..], ip, DecoderOptions::NONE).decode();
                if instruction.is_invalid() {
                    return Err(invalid("selector: undecodable instruction"));
                }
                self.code.insert(index, instruction);
                Ok(instruction)
            }
        }
    }

    fn put(&mut self, address: u64, mut instruction: Instruction) {
        instruction.set_ip(address);
        match self.code.binary_search_by_key(&address, Instruction::ip) {
            Ok(index) => self.code[index] = instruction,
            Err(index) => self.code.insert(index, instruction),
        }
    }

    fn select(&mut self, input: Register, token: u32, output: bool) -> io::Result<Outcome> {
        let mut regs = Registers::default();
        set_register(&mut regs, Register::RSP, STACK)?;
        set_register(&mut regs, input, u64::from(token))?;
        if output {
            set_register(&mut regs, Register::RCX, OUTPUT)?;
        }
        let mut writes = BTreeMap::new();
        let mut matched = false;
        let mut pc = self.entry;
        let mut flags = (false, false, false);
        let mut visited = Vec::new();
        loop {
            if visited.contains(&pc) {
                return Err(invalid("selector: cyclic branch"));
            }
            visited.push(pc);
            let i = self.instruction(pc)?;
            let mut next = i.next_ip();
            let read = |operand| operand_value(self.image, &regs, &writes, &i, operand);
            match i.mnemonic() {
                Mnemonic::Mov | Mnemonic::Movzx | Mnemonic::Movsx | Mnemonic::Movsxd => {
                    let mut value = read(1)?;
                    if matches!(i.mnemonic(), Mnemonic::Movsx | Mnemonic::Movsxd) {
                        value = signed(value, operand_width(&i, 1)?) as u64;
                    }
                    if i.op0_kind() == OpKind::Register {
                        set_register(&mut regs, i.op0_register(), value)?;
                    } else if i.op0_kind() == OpKind::Memory {
                        let address = memory_address(&regs, &i)?;
                        if address >= self.image.base {
                            return Err(invalid("selector: result writes image storage"));
                        }
                        writes.insert(address, value & mask(operand_width(&i, 0)?));
                    } else {
                        return Err(invalid("selector: unsupported move destination"));
                    }
                }
                Mnemonic::Lea => {
                    let value = memory_address(&regs, &i)?;
                    set_register(&mut regs, i.op0_register(), value)?;
                }
                Mnemonic::Xor if i.op0_register() == i.op1_register() => {
                    set_register(&mut regs, i.op0_register(), 0)?;
                    flags = (true, false, false);
                }
                Mnemonic::Sub | Mnemonic::Add | Mnemonic::Xor | Mnemonic::And | Mnemonic::Or => {
                    let a = read(0)?;
                    let b = read(1)?;
                    let width = operand_width(&i, 0)?;
                    let value = match i.mnemonic() {
                        Mnemonic::Sub => a.wrapping_sub(b),
                        Mnemonic::Add => a.wrapping_add(b),
                        Mnemonic::Xor => a ^ b,
                        Mnemonic::And => a & b,
                        _ => a | b,
                    } & mask(width);
                    flags = if i.mnemonic() == Mnemonic::Sub {
                        compare(a, b, width)
                    } else {
                        compare(value, 0, width)
                    };
                    set_register(&mut regs, i.op0_register(), value)?;
                }
                Mnemonic::Cmp => flags = compare(read(0)?, read(1)?, operand_width(&i, 0)?),
                Mnemonic::Test => flags = compare(read(0)? & read(1)?, 0, operand_width(&i, 0)?),
                Mnemonic::Cmove | Mnemonic::Cmovne => {
                    if (i.mnemonic() == Mnemonic::Cmove) == flags.0 {
                        let value = read(1)?;
                        set_register(&mut regs, i.op0_register(), value)?;
                        matched |= flags.0;
                    }
                }
                Mnemonic::Jmp
                | Mnemonic::Je
                | Mnemonic::Jne
                | Mnemonic::Jg
                | Mnemonic::Jge
                | Mnemonic::Jl
                | Mnemonic::Jle
                | Mnemonic::Ja
                | Mnemonic::Jae
                | Mnemonic::Jb
                | Mnemonic::Jbe => {
                    let (equal, below, less) = flags;
                    let taken = match i.mnemonic() {
                        Mnemonic::Jmp => true,
                        Mnemonic::Je => equal,
                        Mnemonic::Jne => !equal,
                        Mnemonic::Jg => !equal && !less,
                        Mnemonic::Jge => !less,
                        Mnemonic::Jl => less,
                        Mnemonic::Jle => equal || less,
                        Mnemonic::Ja => !equal && !below,
                        Mnemonic::Jae => !below,
                        Mnemonic::Jb => below,
                        _ => equal || below,
                    };
                    matched |= equal && matches!(i.mnemonic(), Mnemonic::Je | Mnemonic::Jne);
                    if taken {
                        next = read(0)?;
                    }
                }
                Mnemonic::Ret => {
                    return Ok(Outcome {
                        result: register_value(&regs, Register::RAX)?,
                        writes,
                        matched,
                    });
                }
                Mnemonic::Nop => {}
                _ => {
                    return Err(invalid(format!(
                        "selector: 1 unsupported instruction {:?}",
                        i.mnemonic()
                    )));
                }
            }
            pc = next;
        }
    }
}

fn leaf_extent(image: &PeImage, entry: u32) -> io::Result<(u32, u32)> {
    let (_, section_end) = *image
        .executable_ranges()
        .iter()
        .find(|&&(start, end)| start <= entry && entry < end)
        .ok_or_else(|| invalid("selector entry lies outside executable code"))?;
    let functions = image.functions();
    let next = functions.partition_point(|&(start, _)| start <= entry);
    let end = functions
        .get(next)
        .map_or(section_end, |&(start, _)| start.min(section_end));
    Ok((entry, end))
}

fn operand_value(
    image: &PeImage,
    regs: &Registers,
    writes: &BTreeMap<u64, u64>,
    i: &Instruction,
    operand: u32,
) -> io::Result<u64> {
    match i.op_kind(operand) {
        OpKind::Register => register_value(regs, i.op_register(operand)),
        OpKind::Memory => {
            let address = memory_address(regs, i)?;
            if let Some(value) = writes.get(&address) {
                return Ok(*value & mask(operand_width(i, operand)?));
            }
            let bytes = image.bytes(image.va_to_rva(address)?, operand_width(i, operand)?)?;
            let mut value = [0; 8];
            value[..bytes.len()].copy_from_slice(bytes);
            Ok(u64::from_le_bytes(value))
        }
        OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64 => {
            Ok(i.near_branch_target())
        }
        _ => immediate(i, operand),
    }
}

fn immediate(i: &Instruction, operand: u32) -> io::Result<u64> {
    if matches!(
        i.op_kind(operand),
        OpKind::Immediate8
            | OpKind::Immediate16
            | OpKind::Immediate32
            | OpKind::Immediate64
            | OpKind::Immediate8to16
            | OpKind::Immediate8to32
            | OpKind::Immediate8to64
            | OpKind::Immediate32to64
    ) {
        Ok(i.immediate(operand))
    } else {
        Err(invalid("metadata: immediate operand is missing"))
    }
}

fn operand_width(i: &Instruction, operand: u32) -> io::Result<usize> {
    let width = match i.op_kind(operand) {
        OpKind::Register => i.op_register(operand).size(),
        OpKind::Memory => i.memory_size().size(),
        _ => 8,
    };
    if width == 0 || width > 8 {
        return Err(invalid("selector: unsupported operand width"));
    }
    Ok(width)
}

fn mask(width: usize) -> u64 {
    if width == 8 {
        u64::MAX
    } else {
        (1_u64 << (width * 8)) - 1
    }
}
fn signed(value: u64, width: usize) -> i64 {
    let shift = 64 - width * 8;
    ((value << shift) as i64) >> shift
}
fn compare(a: u64, b: u64, width: usize) -> (bool, bool, bool) {
    let a = a & mask(width);
    let b = b & mask(width);
    (a == b, a < b, signed(a, width) < signed(b, width))
}

#[derive(Default)]
struct Registers(Vec<(Register, u64, u64)>);

impl Registers {
    fn known(registers: &[(Register, u64)]) -> Self {
        Self(
            registers
                .iter()
                .map(|&(register, value)| (register, value, u64::MAX))
                .collect(),
        )
    }

    fn value(&self, register: Register) -> Option<u64> {
        let (shift, width) = placement(register);
        let bits = mask(width) << shift;
        self.0
            .iter()
            .find(|(full, ..)| *full == register.full_register())
            .filter(|&&(_, _, known)| known & bits == bits)
            .map(|&(_, value, _)| (value >> shift) & mask(width))
    }

    fn set(&mut self, register: Register, value: u64) {
        let (shift, width) = placement(register);
        let full = register.full_register();
        let (value, known) = if width >= 4 {
            (value & mask(width), u64::MAX)
        } else {
            let (previous, known) = self
                .0
                .iter()
                .find(|(entry, ..)| *entry == full)
                .map_or((0, 0), |&(_, value, known)| (value, known));
            let bits = mask(width) << shift;
            ((previous & !bits) | ((value & mask(width)) << shift), known | bits)
        };
        match self.0.iter_mut().find(|(entry, ..)| *entry == full) {
            Some(entry) => *entry = (full, value, known),
            None => self.0.push((full, value, known)),
        }
    }

    fn remove(&mut self, register: &Register) {
        let full = register.full_register();
        self.0.retain(|(entry, ..)| *entry != full);
    }
}

fn placement(register: Register) -> (u32, usize) {
    let shift = if matches!(
        register,
        Register::AH | Register::BH | Register::CH | Register::DH
    ) {
        8
    } else {
        0
    };
    (shift, register.size())
}

fn register_value(regs: &Registers, register: Register) -> io::Result<u64> {
    regs.value(register)
        .ok_or_else(|| invalid("metadata: register value is unknown"))
}

fn set_register(regs: &mut Registers, register: Register, value: u64) -> io::Result<()> {
    let width = register.size();
    if width == 0 || width > 8 {
        return Err(invalid("metadata: unsupported register width"));
    }
    regs.set(register, value);
    Ok(())
}

fn memory_address(regs: &Registers, i: &Instruction) -> io::Result<u64> {
    if i.is_ip_rel_memory_operand() {
        return Ok(i.ip_rel_memory_address());
    }
    let base = if i.memory_base() == Register::None {
        0
    } else {
        register_value(regs, i.memory_base())?
    };
    let index = if i.memory_index() == Register::None {
        0
    } else {
        register_value(regs, i.memory_index())?
    };
    Ok(base
        .wrapping_add(index.wrapping_mul(u64::from(i.memory_index_scale())))
        .wrapping_add(i.memory_displacement64()))
}

#[derive(Clone, Copy, Default)]
struct RuleLayout {
    construct: u32,
    copy: u32,
    assign: u32,
    allocate: u32,
    release: u32,
    base: u64,
    stride: u64,
    label: u64,
    token: u64,
    index: u64,
    fulfilled: u64,
    default: u64,
}

fn rule_layout(image: &PeImage, anchors: &Anchors) -> io::Result<(RuleLayout, Vec<Instruction>)> {
    let (_, references) = anchors.vtable(RULE_TYPE)?;
    let functions = anchors.holders(references)?;
    let starts: BTreeSet<u32> = functions.iter().map(|&(start, _)| start).collect();
    let mut found = Vec::new();
    for &extent in &functions {
        let builder = code(image, extent)?;
        let mut sites = BTreeMap::<u32, Vec<Option<(u64, u64)>>>::new();
        let mut last = BTreeMap::new();
        let mut arguments = 0;
        for (position, i) in builder.iter().enumerate() {
            if i.mnemonic() != Mnemonic::Call {
                continue;
            }
            if let Some(target) = call_target(image, i)
                && starts.contains(&target)
            {
                let written = |register: Register| {
                    builder[arguments..position].iter().rev().find(|argument| {
                        argument.op0_kind() == OpKind::Register
                            && argument.op0_register().full_register() == register
                    })
                };
                let index = written(Register::R8)
                    .filter(|argument| argument.mnemonic() == Mnemonic::Mov)
                    .and_then(|argument| immediate(argument, 1).ok());
                let row = written(Register::RCX)
                    .filter(|argument| {
                        argument.mnemonic() == Mnemonic::Lea
                            && argument.memory_base() == Register::RBP
                            && argument.memory_index() == Register::None
                    })
                    .map(Instruction::memory_displacement64);
                sites.entry(target).or_default().push(index.zip(row));
                last.insert(target, position);
            }
            arguments = position + 1;
        }
        for (target, sites) in sites {
            let Some(sites) = sites.into_iter().collect::<Option<Vec<_>>>() else {
                continue;
            };
            let sites: Vec<(i64, i64)> = sites
                .iter()
                .map(|&(index, row)| (index as i64, row as i64))
                .collect();
            let [(first, at), (second, next), ..] = sites[..] else {
                continue;
            };
            if second == first || (next - at) % (second - first) != 0 {
                continue;
            }
            let stride = (next - at) / (second - first);
            let base = at - first * stride;
            let indices: BTreeSet<_> = sites.iter().map(|&(index, _)| index).collect();
            if stride > 0
                && indices.len() == sites.len()
                && sites
                    .iter()
                    .all(|&(index, row)| row == base + index * stride)
            {
                let builder = builder[..=last[&target]].to_vec();
                found.push((target, base as u64, stride as u64, builder));
            }
        }
    }
    let (construct, base, stride, builder) = sole(found, "rule row constructor")?;
    let release = sole(
        builder
            .windows(2)
            .filter(|pair| {
                pair[0].mnemonic() == Mnemonic::Cmp
                    && pair[0].op0_register() == Register::RDX
                    && immediate(&pair[0], 1).ok() == Some(BIG_ALLOCATION)
                    && pair[1].mnemonic() == Mnemonic::Jb
            })
            .filter_map(|pair| {
                let at = builder
                    .binary_search_by_key(&pair[1].near_branch_target(), Instruction::ip)
                    .ok()?;
                call_target(image, &builder[at])
            })
            .collect::<BTreeSet<_>>(),
        "string release",
    )?;
    let allocate = sole(
        builder
            .windows(2)
            .filter(|pair| {
                pair[0].mnemonic() == Mnemonic::Mov
                    && pair[0].op0_register() == Register::ECX
                    && is_immediate(&pair[0], 1)
            })
            .filter_map(|pair| call_target(image, &pair[1]))
            .collect::<BTreeSet<_>>(),
        "string allocation",
    )?;
    let mut layout = row_layout(image, construct)?;
    layout.construct = construct;
    layout.release = release;
    layout.allocate = allocate;
    layout.base = base;
    layout.stride = stride;
    layout.assign = sole(
        builder
            .iter()
            .filter_map(|i| call_target(image, i))
            .filter(|&target| ![construct, layout.copy, release, allocate].contains(&target))
            .collect::<BTreeSet<_>>(),
        "string assignment",
    )?;
    Ok((layout, builder))
}

fn row_layout(image: &PeImage, construct: u32) -> io::Result<RuleLayout> {
    #[derive(Clone, Copy, PartialEq)]
    enum Origin {
        Parameter(Register, u64),
        Argument(u64),
    }
    let constructor = code(image, image.function(construct)?)?;
    let mut origins: Vec<(Register, Origin)> =
        [Register::RCX, Register::RDX, Register::R8, Register::R9]
            .map(|register| (register, Origin::Parameter(register, 0)))
            .to_vec();
    let origin = |origins: &[(Register, Origin)], register: Register| {
        origins
            .iter()
            .find(|(known, _)| *known == register.full_register())
            .map(|&(_, origin)| origin)
    };
    let set = |origins: &mut Vec<(Register, Origin)>, register: Register, value: Option<Origin>| {
        let register = register.full_register();
        origins.retain(|(known, _)| *known != register);
        if let Some(value) = value {
            origins.push((register, value));
        }
    };
    let mut depth = 0_u64;
    let mut fields = Vec::new();
    let mut label = None;
    for i in &constructor {
        match i.mnemonic() {
            Mnemonic::Ret => break,
            Mnemonic::Push => depth += 8,
            Mnemonic::Sub if i.op0_register() == Register::RSP => depth += immediate(i, 1)?,
            Mnemonic::Call => {
                if label.is_none()
                    && let Some(target) = call_target(image, i)
                    && let Some(Origin::Parameter(Register::RCX, offset)) =
                        origin(&origins, Register::RCX)
                    && origin(&origins, Register::RDX) == Some(Origin::Parameter(Register::R9, 0))
                {
                    label = Some((target, offset));
                }
                for volatile in [
                    Register::RAX,
                    Register::RCX,
                    Register::RDX,
                    Register::R8,
                    Register::R9,
                    Register::R10,
                    Register::R11,
                ] {
                    set(&mut origins, volatile, None);
                }
            }
            Mnemonic::Mov | Mnemonic::Movzx
                if i.op0_kind() == OpKind::Register && i.op1_kind() == OpKind::Register =>
            {
                let value = origin(&origins, i.op1_register());
                set(&mut origins, i.op0_register(), value);
            }
            Mnemonic::Movzx
                if i.op0_kind() == OpKind::Register
                    && i.op1_kind() == OpKind::Memory
                    && i.memory_base() == Register::RSP
                    && i.memory_index() == Register::None =>
            {
                let value = i
                    .memory_displacement64()
                    .checked_sub(depth + 8)
                    .map(Origin::Argument);
                set(&mut origins, i.op0_register(), value);
            }
            Mnemonic::Add if i.op0_kind() == OpKind::Register && is_immediate(i, 1) => {
                let value = match origin(&origins, i.op0_register()) {
                    Some(Origin::Parameter(register, offset)) => Some(Origin::Parameter(
                        register,
                        offset.wrapping_add(immediate(i, 1)?),
                    )),
                    _ => None,
                };
                set(&mut origins, i.op0_register(), value);
            }
            Mnemonic::Mov
                if i.op0_kind() == OpKind::Memory
                    && i.memory_index() == Register::None
                    && i.op1_kind() == OpKind::Register =>
            {
                if origin(&origins, i.memory_base()) == Some(Origin::Parameter(Register::RCX, 0))
                    && let Some(value) = origin(&origins, i.op1_register())
                {
                    fields.push((i.memory_displacement64(), i.memory_size().size(), value));
                }
            }
            _ => {
                if i.op0_kind() == OpKind::Register {
                    set(&mut origins, i.op0_register(), None);
                }
            }
        }
    }
    let field = |what: &str, width: usize, from: Origin| {
        sole(
            fields
                .iter()
                .filter(|&&(_, size, origin)| size == width && origin == from)
                .map(|&(offset, _, _)| offset),
            what,
        )
    };
    let (copy, label) = label.ok_or_else(|| invalid("the row constructor copies no label"))?;
    Ok(RuleLayout {
        copy,
        label,
        token: field("row token field", 4, Origin::Parameter(Register::RDX, 0))?,
        index: field("row index field", 4, Origin::Parameter(Register::R8, 0))?,
        fulfilled: field(
            "row fulfilled field",
            1,
            Origin::Argument(RULE_FULFILLED_ARGUMENT),
        )?,
        default: field(
            "row default field",
            1,
            Origin::Argument(RULE_DEFAULT_ARGUMENT),
        )?,
        ..RuleLayout::default()
    })
}

struct RuleConstruction<'a> {
    image: &'a PeImage,
    layout: RuleLayout,
    regs: Registers,
    vectors: BTreeMap<Register, Vec<u8>>,
    bytes: BTreeMap<u64, u8>,
    strings: BTreeMap<u64, String>,
    rows: BTreeMap<u32, (u32, String, bool, bool)>,
    next_heap: u64,
    called: bool,
}

impl<'a> RuleConstruction<'a> {
    fn new(image: &'a PeImage, layout: RuleLayout) -> Self {
        Self {
            image,
            layout,
            regs: Registers::known(&[(Register::RSP, STACK), (Register::RBP, FRAME)]),
            vectors: BTreeMap::new(),
            bytes: BTreeMap::new(),
            strings: BTreeMap::new(),
            rows: BTreeMap::new(),
            next_heap: HEAP,
            called: false,
        }
    }

    fn row(&self, index: u32) -> u64 {
        FRAME
            .wrapping_add(self.layout.base)
            .wrapping_add(u64::from(index).wrapping_mul(self.layout.stride))
    }

    fn bytes_at(&self, address: u64, length: usize) -> io::Result<Vec<u8>> {
        if address >= self.image.base {
            return Ok(self
                .image
                .bytes(self.image.va_to_rva(address)?, length)?
                .to_vec());
        }
        (0..length)
            .map(|offset| {
                let address = address
                    .checked_add(offset as u64)
                    .ok_or_else(|| invalid("rules: string address overflow"))?;
                self.bytes
                    .get(&address)
                    .copied()
                    .ok_or_else(|| invalid("rules: construction byte is unknown"))
            })
            .collect()
    }

    fn number_at(&self, address: u64, length: usize) -> io::Result<u64> {
        let bytes = self.bytes_at(address, length)?;
        let mut value = [0; 8];
        value[..length].copy_from_slice(&bytes);
        Ok(u64::from_le_bytes(value))
    }

    fn store(&mut self, address: u64, bytes: &[u8]) -> io::Result<()> {
        if address >= self.image.base {
            return Err(invalid("rules: construction writes image storage"));
        }
        self.overwrite_strings(address, bytes.len());
        for (offset, &byte) in bytes.iter().enumerate() {
            let address = address
                .checked_add(offset as u64)
                .ok_or_else(|| invalid("rules: construction address overflow"))?;
            self.bytes.insert(address, byte);
        }
        Ok(())
    }

    fn string(&self, object: u64) -> io::Result<String> {
        if let Some(value) = self.strings.get(&object) {
            return Ok(value.clone());
        }
        let length = usize::try_from(self.number_at(object + 16, 8)?)
            .map_err(|_| invalid("rules: string length overflow"))?;
        let capacity = self.number_at(object + 24, 8)?;
        if length as u64 > capacity {
            return Err(invalid("rules: string length exceeds capacity"));
        }
        let payload = if capacity <= 15 {
            object
        } else {
            self.number_at(object, 8)?
        };
        let bytes = self.bytes_at(
            payload,
            length
                .checked_add(1)
                .ok_or_else(|| invalid("rules: string extent overflow"))?,
        )?;
        if bytes.last() != Some(&0) || bytes[..length].contains(&0) {
            return Err(invalid("rules: label length or terminator differs"));
        }
        String::from_utf8(bytes[..length].to_vec())
            .map_err(|_| invalid("rules: label is not UTF-8"))
    }

    fn source_bytes(&self, i: &Instruction) -> io::Result<Vec<u8>> {
        let width = if i.op0_kind() == OpKind::Memory {
            i.memory_size().size()
        } else {
            i.op0_register().size()
        };
        if width == 0 || width > 16 {
            return Err(invalid("rules: unsupported construction width"));
        }
        let source_width = if i.op1_kind() == OpKind::Memory {
            i.memory_size().size()
        } else {
            width
        };
        let mut bytes = match i.op1_kind() {
            OpKind::Memory => self.bytes_at(memory_address(&self.regs, i)?, source_width)?,
            OpKind::Register if i.op1_register().size() > 8 => self
                .vectors
                .get(&i.op1_register())
                .cloned()
                .ok_or_else(|| invalid("rules: vector is unknown"))?,
            OpKind::Register => register_value(&self.regs, i.op1_register())?
                .to_le_bytes()
                .to_vec(),
            _ => immediate(i, 1)?.to_le_bytes().to_vec(),
        };
        bytes.resize(width, 0);
        bytes.truncate(width);
        Ok(bytes)
    }

    fn clobber_call(&mut self) {
        for register in [
            Register::RAX,
            Register::RCX,
            Register::RDX,
            Register::R8,
            Register::R9,
            Register::R10,
            Register::R11,
        ] {
            self.regs.remove(&register);
        }
        for register in [
            Register::XMM0,
            Register::XMM1,
            Register::XMM2,
            Register::XMM3,
            Register::XMM4,
            Register::XMM5,
        ] {
            self.vectors.remove(&register);
        }
    }

    fn call(&mut self, target: u32) -> io::Result<()> {
        self.called = true;
        let layout = self.layout;
        let mut allocated = None;
        if target == layout.allocate {
            let length = register_value(&self.regs, Register::ECX)?;
            let address = self.next_heap;
            self.next_heap = address
                .checked_add(length)
                .and_then(|end| end.checked_add(16))
                .ok_or_else(|| invalid("rules: allocation extent overflow"))?;
            if self.next_heap >= self.image.base {
                return Err(invalid("rules: allocation overlaps image"));
            }
            allocated = Some(address);
        } else if target == layout.release {
        } else if target == layout.assign {
            let object = register_value(&self.regs, Register::RCX)?;
            let address = register_value(&self.regs, Register::RDX)?;
            let length = usize::try_from(register_value(&self.regs, Register::R8D)?)
                .map_err(|_| invalid("rules: label length overflow"))?;
            let bytes = self.bytes_at(address, length)?;
            if bytes.contains(&0) {
                return Err(invalid("rules: assigned label contains a terminator"));
            }
            let text = String::from_utf8(bytes)
                .map_err(|_| invalid("rules: assigned label is not UTF-8"))?;
            self.forget(object, STRING_OBJECT as usize)?;
            self.strings.insert(object, text);
        } else if target == layout.copy {
            let source = register_value(&self.regs, Register::RDX)?;
            let destination = register_value(&self.regs, Register::RCX)?;
            let text = self.string(source)?;
            self.forget(destination, STRING_OBJECT as usize)?;
            self.strings.insert(destination, text);
        } else if target == layout.construct {
            let index = register_value(&self.regs, Register::R8D)? as u32;
            let token = register_value(&self.regs, Register::EDX)? as u32;
            if register_value(&self.regs, Register::RCX)? != self.row(index) {
                return Err(invalid("rules: index and destination disagree"));
            }
            let label = self.string(register_value(&self.regs, Register::R9)?)?;
            let default = self.number_at(STACK + RULE_DEFAULT_ARGUMENT, 1)?;
            let fulfilled = self.number_at(STACK + RULE_FULFILLED_ARGUMENT, 1)?;
            if default > 1 || fulfilled > 1 {
                return Err(invalid("rules: flag is not boolean"));
            }
            if self
                .rows
                .insert(index, (token, label, default != 0, fulfilled != 0))
                .is_some()
            {
                return Err(invalid("rules: duplicate constructor index"));
            }
        } else {
            return Err(invalid("rules: 1 unsupported construction call"));
        }
        self.clobber_call();
        if let Some(address) = allocated {
            set_register(&mut self.regs, Register::RAX, address)?;
        }
        Ok(())
    }

    fn overwrite_strings(&mut self, address: u64, length: usize) {
        let end = address.saturating_add(length as u64);
        self.strings
            .retain(|&object, _| object.saturating_add(STRING_OBJECT) <= address || end <= object);
    }

    fn forget(&mut self, address: u64, length: usize) -> io::Result<()> {
        if address >= self.image.base {
            return Err(invalid("rules: construction writes image storage"));
        }
        self.overwrite_strings(address, length);
        for offset in 0..length as u64 {
            self.bytes.remove(&address.wrapping_add(offset));
        }
        Ok(())
    }

    fn arithmetic(&mut self, i: &Instruction) -> io::Result<()> {
        if i.op0_kind() == OpKind::Register && i.op0_register() == Register::RSP {
            if self.called {
                return Err(invalid("rules: the stack pointer moves after a call"));
            }
            return Ok(());
        }
        let width = operand_width(i, 0)?;
        let operand = if i.mnemonic() == Mnemonic::Inc {
            Ok(1)
        } else {
            match i.op1_kind() {
                OpKind::Register => register_value(&self.regs, i.op1_register()),
                OpKind::Memory => memory_address(&self.regs, i)
                    .and_then(|address| self.number_at(address, width)),
                _ => immediate(i, 1),
            }
        };
        let apply = |value: u64, operand: u64| {
            if i.mnemonic() == Mnemonic::Sub {
                value.wrapping_sub(operand)
            } else {
                value.wrapping_add(operand)
            }
        };
        match i.op0_kind() {
            OpKind::Register => {
                let register = i.op0_register();
                match (register_value(&self.regs, register), operand) {
                    (Ok(value), Ok(operand)) => {
                        set_register(&mut self.regs, register, apply(value, operand))?
                    }
                    _ => self.regs.remove(&register),
                }
            }
            OpKind::Memory => {
                let address = memory_address(&self.regs, i)
                    .map_err(|_| invalid("rules: an update's address is unknown"))?;
                match (self.number_at(address, width), operand) {
                    (Ok(value), Ok(operand)) => {
                        let bytes = apply(value, operand).to_le_bytes();
                        self.store(address, &bytes[..width])?;
                    }
                    _ => self.forget(address, width)?,
                }
            }
            _ => return Err(invalid("rules: unsupported arithmetic destination")),
        }
        Ok(())
    }

    fn instruction(&mut self, i: &Instruction) -> io::Result<()> {
        match i.mnemonic() {
            Mnemonic::Mov | Mnemonic::Movzx | Mnemonic::Movups | Mnemonic::Movsd => {
                let value = self.source_bytes(i);
                match i.op0_kind() {
                    OpKind::Register if i.op0_register().size() > 8 => {
                        if let Ok(bytes) = value {
                            self.vectors.insert(i.op0_register(), bytes);
                        } else {
                            self.vectors.remove(&i.op0_register());
                        }
                    }
                    OpKind::Register => {
                        if let Ok(bytes) = value {
                            let mut scalar = [0; 8];
                            scalar[..bytes.len()].copy_from_slice(&bytes);
                            set_register(
                                &mut self.regs,
                                i.op0_register(),
                                u64::from_le_bytes(scalar),
                            )?;
                        } else {
                            self.regs.remove(&i.op0_register().full_register());
                        }
                    }
                    OpKind::Memory => {
                        let address = memory_address(&self.regs, i)
                            .map_err(|_| invalid("rules: a store's address is unknown"))?;
                        match value {
                            Ok(bytes) => self.store(address, &bytes)?,
                            Err(_) => self.forget(address, i.memory_size().size())?,
                        }
                    }
                    _ => return Err(invalid("rules: unsupported move destination")),
                }
            }
            Mnemonic::Lea if i.op0_register() != Register::RBP => {
                if let Ok(address) = memory_address(&self.regs, i) {
                    set_register(&mut self.regs, i.op0_register(), address)?;
                } else {
                    self.regs.remove(&i.op0_register().full_register());
                }
            }
            Mnemonic::Xor if i.op0_register() == i.op1_register() => {
                set_register(&mut self.regs, i.op0_register(), 0)?
            }
            Mnemonic::Xorps if i.op0_register() == i.op1_register() => {
                self.vectors.insert(i.op0_register(), vec![0; 16]);
            }
            Mnemonic::Call
                if matches!(i.op0_kind(), OpKind::NearBranch64 | OpKind::NearBranch32) =>
            {
                self.call(self.image.va_to_rva(i.near_branch_target())?)?
            }
            Mnemonic::Call => return Err(invalid("rules: indirect construction call")),
            Mnemonic::Add | Mnemonic::Sub | Mnemonic::Inc => self.arithmetic(i)?,
            Mnemonic::Push if self.called => {
                return Err(invalid("rules: the stack pointer moves after a call"));
            }
            Mnemonic::Lea
            | Mnemonic::Push
            | Mnemonic::Cmp
            | Mnemonic::Jne
            | Mnemonic::Jbe
            | Mnemonic::Jb
            | Mnemonic::Ja
            | Mnemonic::Nop => {}
            _ => {
                return Err(invalid(format!(
                    "rules: 1 unsupported construction instruction {:?}",
                    i.mnemonic()
                )));
            }
        }
        Ok(())
    }
}

fn branch_merges(
    image: &PeImage,
    builder: &[Instruction],
    release: u64,
) -> io::Result<BTreeMap<usize, Vec<Register>>> {
    let mut merges: BTreeMap<usize, Vec<Register>> = BTreeMap::new();
    let last = builder.last().map_or(0, Instruction::ip);
    for (at, branch) in builder.iter().enumerate() {
        if branch.flow_control() != FlowControl::ConditionalBranch {
            continue;
        }
        let target = branch.near_branch_target();
        if target > last && builds_nothing(image, target)? {
            continue;
        }
        let join = builder
            .iter()
            .position(|instruction| instruction.ip() == target)
            .filter(|&join| join > at)
            .ok_or_else(|| invalid("rules: a branch leaves the builder or runs backwards"))?;
        for skipped in &builder[at + 1..join] {
            let calls_other = skipped.mnemonic() == Mnemonic::Call
                && !(skipped.op0_kind() == OpKind::NearBranch64
                    && skipped.near_branch_target() == release);
            let writes_memory = skipped.op0_kind() == OpKind::Memory
                && !matches!(skipped.mnemonic(), Mnemonic::Cmp | Mnemonic::Test);
            if calls_other || writes_memory {
                return Err(invalid("rules: a branch can skip a metadata write"));
            }
            if skipped.op0_kind() == OpKind::Register {
                let register = skipped.op0_register();
                let written = if register.size() > 8 {
                    register
                } else {
                    register.full_register()
                };
                merges.entry(join).or_default().push(written);
            }
        }
    }
    Ok(merges)
}

fn builds_nothing(image: &PeImage, va: u64) -> io::Result<bool> {
    let start = image.va_to_rva(va)?;
    let bytes = image.bytes(start, 64)?;
    let mut decoder = Decoder::with_ip(64, bytes, va, DecoderOptions::NONE);
    for _ in 0..16 {
        let instruction = decoder.decode();
        match instruction.mnemonic() {
            Mnemonic::Ret => return Ok(true),
            Mnemonic::Call if instruction.op0_kind() == OpKind::NearBranch64 => {
                return never_returns(image, image.va_to_rva(instruction.near_branch_target())?);
            }
            Mnemonic::Pop | Mnemonic::Add | Mnemonic::Lea | Mnemonic::Mov
                if instruction.op0_kind() == OpKind::Register => {}
            _ => return Ok(false),
        }
    }
    Ok(false)
}

fn never_returns(image: &PeImage, rva: u32) -> io::Result<bool> {
    let (start, end) = image.function(rva)?;
    if start != rva {
        return Ok(false);
    }
    for instruction in image.instructions(start, end)? {
        let leaves = match instruction.flow_control() {
            FlowControl::Return | FlowControl::IndirectBranch => true,
            FlowControl::UnconditionalBranch | FlowControl::ConditionalBranch => image
                .va_to_rva(instruction.near_branch_target())
                .is_ok_and(|target| !(start..end).contains(&target)),
            _ => false,
        };
        if leaves {
            return Ok(false);
        }
    }
    Ok(true)
}

fn rule_projection(
    image: &PeImage,
    anchors: &Anchors,
    keywords: &KeywordTables,
) -> io::Result<Vec<(&'static str, &'static str, bool, bool)>> {
    let (layout, builder) = rule_layout(image, anchors)?;
    let merges = branch_merges(image, &builder, image.base + u64::from(layout.release))?;
    let mut construction = RuleConstruction::new(image, layout);
    for (index, instruction) in builder.iter().enumerate() {
        for &register in merges.get(&index).into_iter().flatten() {
            construction.regs.remove(&register);
            construction.vectors.remove(&register);
        }
        construction.instruction(instruction)?;
    }
    let inline = *construction
        .rows
        .keys()
        .next()
        .ok_or_else(|| invalid("rules: no constructed rows"))?;
    for index in 0..inline {
        let row = construction.row(index);
        let token = construction.number_at(row + layout.token, 4)? as u32;
        if construction.number_at(row + layout.index, 4)? != u64::from(index) {
            return Err(invalid("rules: inline row index differs"));
        }
        let label = construction.string(row + layout.label)?;
        let default = construction.number_at(row + layout.default, 1)?;
        let fulfilled = construction.number_at(row + layout.fulfilled, 1)?;
        if default > 1 || fulfilled > 1 {
            return Err(invalid("rules: inline flag is not boolean"));
        }
        construction
            .rows
            .insert(index, (token, label, default != 0, fulfilled != 0));
    }
    let mut rows = Vec::new();
    let mut names = BTreeSet::new();
    for (expected, (index, (token, label, default, fulfilled))) in
        construction.rows.into_iter().enumerate()
    {
        if expected != index as usize {
            return Err(invalid("rules: missing row index"));
        }
        let name = keywords
            .name(token)
            .ok_or_else(|| invalid("rules: keyword is missing"))?;
        if label.is_empty() || !names.insert(name) {
            return Err(invalid("rules: a label is empty or a rule repeats"));
        }
        rows.push((name, leak_string(label), default, fulfilled));
    }
    Ok(rows)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(code: &[u8]) -> PeImage {
        let mut bytes = vec![0; 0x600];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        let p16 = |bytes: &mut [u8], at: usize, value: u16| {
            bytes[at..at + 2].copy_from_slice(&value.to_le_bytes())
        };
        let p32 = |bytes: &mut [u8], at: usize, value: u32| {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes())
        };
        p32(&mut bytes, 0x3c, 0x80);
        p16(&mut bytes, 0x84, 0x8664);
        p16(&mut bytes, 0x86, 1);
        p16(&mut bytes, 0x94, 0xf0);
        p16(&mut bytes, 0x98, 0x20b);
        bytes[0xb0..0xb8].copy_from_slice(&super::super::pe::PREFERRED_IMAGE_BASE.to_le_bytes());
        p32(&mut bytes, 0xb8, 0x1000);
        p32(&mut bytes, 0xbc, 0x200);
        p32(&mut bytes, 0xd0, 0x2000);
        p32(&mut bytes, 0xd4, 0x200);
        p32(&mut bytes, 0x104, 5);
        p32(&mut bytes, 0x120, 0x1300);
        p32(&mut bytes, 0x124, 12);
        p32(&mut bytes, 0x190, 0x400);
        p32(&mut bytes, 0x194, 0x1000);
        p32(&mut bytes, 0x198, 0x400);
        p32(&mut bytes, 0x19c, 0x200);
        p32(&mut bytes, 0x1ac, 0x2000_0000);
        p32(&mut bytes, 0x500, 0x1000);
        p32(&mut bytes, 0x504, 0x1000 + code.len() as u32);
        p32(&mut bytes, 0x508, 0x1320);
        bytes[0x200..0x200 + code.len()].copy_from_slice(code);
        PeImage::parse(bytes).unwrap()
    }

    #[test]
    fn enum_validity_accepts_immediate_and_low_byte_register_sources() {
        let immediate = [
            0xc7, 0x01, 7, 0, 0, 0, 0xc6, 0x41, 4, 1, 0x48, 0x89, 0xc8, 0xc3,
        ];
        let register = [
            0xc7, 0x01, 7, 0, 0, 0, 0xba, 1, 0, 0, 0, 0x88, 0x51, 4, 0x48, 0x89, 0xc8, 0xc3,
        ];
        for code in [&immediate[..], &register[..]] {
            let image = image(code);
            let writes = Dispatch::new(&image, 0x1000)
                .unwrap()
                .select(Register::EDX, 42, true)
                .unwrap()
                .writes;
            assert_eq!(writes.get(&OUTPUT), Some(&7));
            assert_eq!(writes.get(&(OUTPUT + 4)), Some(&1));
        }
    }

    #[test]
    fn selector_refuses_cycles_and_unknown_operations() {
        for code in [&[0xeb, 0xfe][..], &[0xd1, 0xe0, 0xc3][..]] {
            let image = image(code);
            assert!(
                Dispatch::new(&image, 0x1000)
                    .unwrap()
                    .select(Register::ECX, 42, false)
                    .is_err()
            );
        }
    }

    #[test]
    fn selector_refuses_a_branch_out_of_its_function() {
        let image = image(&[0xeb, 0x10, 0xc3]);
        assert!(
            Dispatch::new(&image, 0x1000)
                .unwrap()
                .select(Register::ECX, 42, false)
                .is_err()
        );
    }

    #[test]
    fn rule_label_requires_its_declared_whole_length() {
        let image = image(&[0xc3]);
        let mut construction = RuleConstruction::new(&image, RuleLayout::default());
        let object = STACK + 0x80;
        construction.store(object, b"SYNTHETIC\0").unwrap();
        construction
            .store(object + 16, &9_u64.to_le_bytes())
            .unwrap();
        construction
            .store(object + 24, &15_u64.to_le_bytes())
            .unwrap();
        assert_eq!(construction.string(object).unwrap(), "SYNTHETIC");
        construction
            .store(object + 16, &8_u64.to_le_bytes())
            .unwrap();
        assert!(construction.string(object).is_err());
    }
}
