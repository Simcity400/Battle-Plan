use content_reader_localisation::localisation::Localisation;
use replay_data::rows::tables;
use std::fmt::Write;

fn reference(
    frame: &dyn replay_frame::frame::DecodedSource,
    handle: u64,
    loc: &Localisation,
) -> Result<String, String> {
    if handle == 0 {
        return Ok("None".into());
    }
    let rows = frame
        .complete::<tables::DivisionTemplates>()
        .map_err(|e| e.to_string())?;
    if rows.iter().any(|row| row.template == handle) {
        Ok(format!("{handle} ({})", display_name(frame, handle, loc)?))
    } else {
        Ok(format!("{handle} (unresolved template)"))
    }
}

pub fn display_name(
    frame: &dyn replay_frame::frame::DecodedSource,
    handle: u64,
    loc: &Localisation,
) -> Result<String, String> {
    display_naming(frame, handle)?.localise(loc)
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum TemplateNaming {
    Text(String),
    Localised { tag: String, key: String },
}

impl TemplateNaming {
    pub fn localise(&self, loc: &Localisation) -> Result<String, String> {
        match self {
            Self::Text(text) => Ok(text.clone()),
            Self::Localised { tag, key } => {
                let qualified = format!("{tag}_{key}");
                loc.localise_with(
                    if loc.get(&qualified).is_some() {
                        qualified.as_bytes()
                    } else {
                        key.as_bytes()
                    },
                    &[],
                )
                .map_err(|e| e.to_string())
            }
        }
    }
}

pub fn display_naming(
    frame: &dyn replay_frame::frame::DecodedSource,
    handle: u64,
) -> Result<TemplateNaming, String> {
    let dictionaries = frame
        .complete::<tables::DivisionTemplates>()
        .map_err(|e| e.to_string())?;
    let dictionary = dictionaries
        .binary_search_by_key(&handle, |r| r.template)
        .ok()
        .map(|at| &dictionaries[at])
        .ok_or_else(|| format!("template {handle} dictionary missing"))?;
    let icons = frame
        .complete::<tables::DivisionTemplateIcons>()
        .map_err(|e| e.to_string())?;
    let icon = icons
        .binary_search_by_key(&handle, |r| r.template)
        .ok()
        .map(|at| &icons[at])
        .ok_or_else(|| format!("template {handle} naming operands missing"))?;
    crate::unit_names::template_name_with(
        icon.display_name_inputs(frame.hour(), Some(dictionary))?,
        icon.country,
        TemplateNaming::Text,
        |raw, key| {
            let countries = frame
                .complete::<tables::Countries>()
                .map_err(|e| e.to_string())?;
            let missing = || format!("template country {raw} missing");
            let country = match countries.binary_search_by_key(&(raw as u32), |r| r.id) {
                Ok(at) => &countries[at],
                Err(_) => {
                    let link = u32::try_from(raw)
                        .ok()
                        .filter(|link| *link > 0)
                        .ok_or_else(missing)?;
                    let links = frame
                        .complete::<tables::CountryLinks>()
                        .map_err(|e| e.to_string())?;
                    let slot = links
                        .binary_search_by_key(&link, |r| r.id)
                        .map(|at| links[at].canonical_slot)
                        .map_err(|_| missing())?;
                    let mut found = countries.iter().filter(|r| r.canonical_slot == slot);
                    let country = found.next().ok_or_else(missing)?;
                    if found.next().is_some() {
                        return Err(format!("Countries duplicates canonical slot {slot}"));
                    }
                    country
                }
            };
            let tag = std::str::from_utf8(&country.tag)
                .map_err(|_| "template country tag is not UTF-8")?;
            let key = std::str::from_utf8(key).map_err(|_| "template name key is not UTF-8")?;
            Ok(TemplateNaming::Localised {
                tag: tag.to_owned(),
                key: key.to_owned(),
            })
        },
    )
}

struct Tokens<'a> {
    rows: replay_frame::frame::Rows<'a, replay_data::rows::LexicalTokenRow>,
    complete: bool,
}
fn tokens(frame: &dyn replay_frame::frame::DecodedSource) -> Result<Tokens<'_>, String> {
    use replay_frame::frame::OperandStatus;
    let rows = frame.lexical_rows();
    let complete = match frame.lexical_status() {
        OperandStatus::Complete => true,
        OperandStatus::Truncated(_) | OperandStatus::Unavailable(_) => false,
        _ => return Err("lexical tokens holds another table's rows".into()),
    };
    if !rows.is_sorted_by(|a, b| a.token < b.token) {
        return Err("lexical tokens rows are not in token order".into());
    }
    Ok(Tokens { rows, complete })
}

fn definition(tokens: &Result<Tokens<'_>, String>, token: u32) -> Result<String, String> {
    if token == 0 {
        return Ok("Null definition".into());
    }
    let tokens = tokens.as_ref().map_err(Clone::clone)?;
    let at = tokens
        .rows
        .binary_search_by_key(&token, |r| r.token)
        .map_err(|_| {
            if tokens.complete {
                format!("definition token {token} is not in lexical tokens")
            } else {
                format!("definition token {token} is among lexical tokens' missing rows")
            }
        })?;
    let name = std::str::from_utf8(&tokens.rows[at].literal)
        .map_err(|_| format!("definition token {token} is not text"))?;
    if name.contains('\0') {
        return Err(format!("definition token {token} contains a null"));
    }
    Ok(name.to_owned())
}

#[derive(Debug, Default)]
pub struct Rendered {
    pub text: String,
    pub refused: Vec<(String, String)>,
}
impl Rendered {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            refused: Vec::new(),
        }
    }
    fn value<T>(&mut self, label: &str, value: Result<T, String>) -> Option<T> {
        match value {
            Ok(value) => Some(value),
            Err(reason) => {
                writeln!(self.text, "{label}: Unavailable").unwrap();
                self.refused.push((label.into(), reason));
                None
            }
        }
    }
    fn line(&mut self, label: &str, value: Result<String, String>) {
        if let Some(value) = self.value(label, value) {
            writeln!(self.text, "{label}: {value}").unwrap();
        }
    }
    fn spelling(&mut self, field: &str, value: Result<String, String>) -> String {
        value.unwrap_or_else(|reason| {
            self.refused.push((field.into(), reason));
            "Unavailable".into()
        })
    }
    fn reference(
        &mut self,
        label: &str,
        frame: &dyn replay_frame::frame::DecodedSource,
        handle: u64,
        loc: &Localisation,
        validate: &dyn Fn(&str) -> Result<(), String>,
    ) {
        let text = checked(reference(frame, handle, loc), validate).unwrap_or_else(|reason| {
            self.refused.push((format!("{label} name"), reason));
            format!("{handle} (name unavailable)")
        });
        self.line(label, Ok(text));
    }
}

fn checked(
    value: Result<String, String>,
    validate: &dyn Fn(&str) -> Result<(), String>,
) -> Result<String, String> {
    let text = value?;
    validate(&text)?;
    Ok(text)
}

pub fn template(
    frame: &dyn replay_frame::frame::DecodedSource,
    handle: u64,
    loc: &Localisation,
) -> Result<Rendered, String> {
    template_with(frame, handle, loc, &|_| Ok(()))
}

pub fn template_with(
    frame: &dyn replay_frame::frame::DecodedSource,
    handle: u64,
    loc: &Localisation,
    validate: &dyn Fn(&str) -> Result<(), String>,
) -> Result<Rendered, String> {
    let rows = frame
        .complete::<tables::DivisionTemplateIcons>()
        .map_err(|e| e.to_string())?;
    let row = rows
        .iter()
        .find(|row| row.template == handle)
        .ok_or_else(|| format!("template {handle} settings missing"))?;
    let mut out = Rendered::text(format!(
        "\nTemplate settings and layout\nTemplate {handle}\n"
    ));
    out.line("Name", checked(display_name(frame, handle, loc), validate));
    let tokens = tokens(frame);
    if let Some(settings) = out.value(
        "Template settings",
        row.settings
            .require("template settings")
            .and_then(|settings| {
                settings
                    .as_ref()
                    .ok_or_else(|| "template settings acquisition refused".to_owned())
            }),
    ) {
        writeln!(out.text,
            "Priority: {}\nHQ template: {}\nLocked: {}\nObsolete: {} (changed {})\nForce recruitment: {}\nNew equipment: {}\nForeign equipment: {}\nRoster position: {}",
            settings.priority, settings.is_army_hq != 0, settings.locked != 0, settings.obsolete != 0,
            settings.obsolete_change_date, settings.force_allow_recruiting != 0, settings.allow_new_equipment != 0,
            settings.allow_foreign_equipment != 0, settings.roster_ordinal).unwrap();
        let group = settings
            .names_group
            .as_deref()
            .map(|bytes| {
                std::str::from_utf8(bytes)
                    .map(|key| format!("\"{key}\""))
                    .map_err(|_| "template naming group is not UTF-8".to_owned())
            })
            .transpose()
            .map(|key| key.unwrap_or_else(|| "None".into()));
        out.line("Historical naming group", checked(group, validate));
        out.reference("Parent template", frame, settings.parent, loc, validate);
        writeln!(out.text, "Creator country: {}\nOrigin: {}\nIntelligence decoy: {}\nDivision cap: {:?}\nEquipment niche: {}",
            settings.creator_country, settings.origin_type, settings.fake_intel != 0,
            settings.division_cap, settings.equipment_niche as i8).unwrap();
        for (label, role) in [
            ("Role", settings.role),
            ("AI role", settings.ai_role),
            ("AI fallback role", Some(settings.ai_role_fallback)),
        ] {
            out.line(
                label,
                checked(
                    role.map(|role| definition(&tokens, role))
                        .transpose()
                        .map(|name| name.unwrap_or_else(|| "None".into())),
                    validate,
                ),
            );
        }
        writeln!(
            out.text,
            "Cached manpower: {}\nHQ command power: {}\nTraining time: {}",
            settings.total_manpower,
            content_reader_parser::fixed5::fixed5_to_string(settings.command_power_cost, -1),
            settings.training_time
        )
        .unwrap();
        if let Some(forbidden) = out.value(
            "Forbidden equipment",
            settings
                .forbidden_equipment
                .as_ref()
                .ok_or_else(|| "template equipment restrictions unreadable".to_owned()),
        ) {
            let mut names = Vec::new();
            for (index, &token) in forbidden.iter().enumerate() {
                names.push(out.spelling(
                    &format!("Forbidden equipment {index}"),
                    checked(definition(&tokens, token), validate),
                ));
            }
            out.line(
                "Forbidden equipment",
                Ok(if names.is_empty() {
                    "None".into()
                } else {
                    names.join(", ")
                }),
            );
        }
    }
    out.line(
        "Excluded variants",
        if row.model_unreadable == 0 {
            Ok(format!("{:?}", row.excluded_variants))
        } else {
            Err("template excluded variants unreadable".into())
        },
    );
    out.line(
        "Model choice",
        if row.model_unreadable == 0 {
            checked(
                std::str::from_utf8(&row.model_entity)
                    .map(str::to_owned)
                    .map_err(|_| "template model choice is not UTF-8".into()),
                validate,
            )
        } else {
            Err("template model choice unreadable".into())
        },
    );
    writeln!(
        out.text,
        "Model category: {:?}\nComposition country: {}\nForeign country: {}",
        row.model_category, row.country, row.foreign_country
    )
    .unwrap();
    let layout = row.layout.require("template layout").and_then(|layout| {
        layout
            .as_ref()
            .ok_or_else(|| "template layout unreadable".into())
    });
    if let Some(layout) = out.value("Template layout", layout) {
        for (label, grid) in [
            ("Regiments", &layout.regiments),
            ("Support", &layout.support),
            ("Regimental support", &layout.regimental_support),
        ] {
            writeln!(
                out.text,
                "{label}: {} columns \u{d7} {} rows",
                grid.columns, grid.rows
            )
            .unwrap();
            if !grid.cells.is_empty() && grid.rows == 0 {
                out.value::<()>(
                    &format!("{label} cells"),
                    Err("nonempty template grid has zero rows".into()),
                );
                continue;
            }
            for (index, cell) in grid.cells.iter().enumerate() {
                let field = format!(
                    "{label} ({}, {})",
                    index / grid.rows as usize,
                    index % grid.rows as usize
                );
                let name = if cell.occupied {
                    out.spelling(&field, checked(definition(&tokens, cell.token), validate))
                } else {
                    "Empty".into()
                };
                writeln!(
                    out.text,
                    "({}, {}): {name}{}",
                    index / grid.rows as usize,
                    index % grid.rows as usize,
                    if cell.local != 0 {
                        " (country local)"
                    } else {
                        ""
                    }
                )
                .unwrap();
            }
        }
    }
    Ok(out)
}

pub fn country(
    frame: &dyn replay_frame::frame::DecodedSource,
    slot: u32,
    loc: &Localisation,
    canonical: impl Fn(u32) -> Result<u32, String>,
) -> Result<Rendered, String> {
    country_with(frame, slot, loc, canonical, &|_| Ok(()))
}

pub fn country_with(
    frame: &dyn replay_frame::frame::DecodedSource,
    slot: u32,
    loc: &Localisation,
    canonical: impl Fn(u32) -> Result<u32, String>,
    validate: &dyn Fn(&str) -> Result<(), String>,
) -> Result<Rendered, String> {
    let rows = frame
        .complete::<tables::CountryTemplateState>()
        .map_err(|e| e.to_string())?;
    let mut selected = None;
    for row in rows {
        if canonical(row.country)? == slot {
            selected = Some(row);
            break;
        }
    }
    let row = selected.ok_or("country template configuration missing")?;
    let mut out = Rendered::text("\nGarrison and staff templates\n");
    out.reference(
        "Military-staff template",
        frame,
        row.selected_staff,
        loc,
        validate,
    );
    writeln!(
        out.text,
        "All templates locked: {}\nNext HQ deployment order: {}",
        row.lock_all != 0,
        row.next_hq_order
    )
    .unwrap();
    out.line(
        "Garrison reinforcement priority",
        row.garrison_priority
            .require("garrison reinforcement priority")
            .and_then(|value| {
                value.ok_or_else(|| "garrison reinforcement priority unreadable".into())
            })
            .map(|value| value.to_string()),
    );
    let scopes = row
        .garrisons
        .require("garrison templates")
        .and_then(|scopes| {
            scopes
                .as_ref()
                .ok_or_else(|| "garrison templates unreadable".into())
        });
    if let Some(scopes) = out.value("Garrison templates", scopes) {
        for scope in scopes.iter() {
            let place = match (scope.occupied, scope.state) {
                (None, None) => "Country default".into(),
                (Some(country), Some(state)) => {
                    format!("Occupied country {country}, state {state}")
                }
                (None, Some(state)) => format!("State {state}"),
                (Some(country), None) => format!("Occupied country {country}"),
            };
            if let Some(handle) = scope.template {
                out.reference(&place, frame, handle, loc, validate);
            } else {
                out.line(&place, Ok("Inherit".into()));
            }
        }
    }
    Ok(out)
}

pub fn hq(
    frame: &dyn replay_frame::frame::DecodedSource,
    group: u64,
    loc: &Localisation,
) -> Result<Rendered, String> {
    hq_with(frame, group, loc, &|_| Ok(()))
}

pub fn hq_with(
    frame: &dyn replay_frame::frame::DecodedSource,
    group: u64,
    loc: &Localisation,
    validate: &dyn Fn(&str) -> Result<(), String>,
) -> Result<Rendered, String> {
    let rows = frame
        .complete::<tables::OrdersGroupCounterInputs>()
        .map_err(|e| e.to_string())?;
    let row = rows
        .iter()
        .find(|row| row.group == group)
        .ok_or("HQ group inputs missing")?;
    let context = row
        .template_context
        .require("HQ template context")
        .and_then(|context| {
            context
                .as_ref()
                .ok_or_else(|| "HQ template context unreadable".into())
        });
    if context.as_ref().is_ok_and(|context| {
        context.target == 0
            && context.deployment.is_none()
            && context.deployed == 0
            && context.queued == 0
    }) {
        return Ok(Rendered::default());
    }
    let mut out = Rendered::text(format!("\nHQ deployment\nHQ group: {group}\n"));
    out.line(
        "Nearest front province",
        row.hq_nearest_front
            .require("HQ nearest front")
            .and_then(|front| front.ok_or_else(|| "HQ nearest front unreadable".into()))
            .map(|front| front.to_string()),
    );
    if let Some(context) = out.value("HQ template context", context) {
        out.reference("HQ target template", frame, context.target, loc, validate);
        writeln!(
            out.text,
            "Deployed: {}\nQueued: {}",
            context.deployed != 0,
            context.queued != 0
        )
        .unwrap();
        if let Some(deployment) = &context.deployment {
            writeln!(out.text, "Deployment country: {}\nPriority: {}\nAllow zero equipment: {}\nManpower: {} / {}\nOrder: {}\nRequisitioned: {}",
                deployment.country, deployment.priority, deployment.allow_zero != 0,
                deployment.assembled_manpower, deployment.requested_manpower, deployment.order, deployment.requisitioned != 0).unwrap();
        } else {
            out.text.push_str("No deployment holder\n");
        }
        for equipment in &context.equipment {
            writeln!(
                out.text,
                "Assembled variant {}: {}",
                equipment.variant,
                content_reader_parser::fixed5::fixed5_to_string(equipment.amount, -1)
            )
            .unwrap();
        }
        let tokens = tokens(frame);
        for (index, required) in context.required.iter().enumerate() {
            let name = out.spelling(
                &format!("HQ requirement {index}"),
                checked(definition(&tokens, required.token), validate),
            );
            writeln!(
                out.text,
                "Required {name}: {}",
                content_reader_parser::fixed5::fixed5_to_string(required.amount, -1)
            )
            .unwrap();
        }
    }
    Ok(out)
}
