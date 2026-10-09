//! Native operative presentation shared by roster and placement views.
//! Skill thresholds, captured duration, operation metadata and artwork come
//! from the recording's resolved content stack.

use super::{LoadContext, character_portraits, operative_rows, politics_content};
use crate::{
    Source,
    catalogue::{self, Catalogue, View},
};
use installed_content::{
    localisation::Argument,
    unit_leader::{Leaders, Skills},
};
use replay::rows::{CharacterPortraitRow, OperativeMembershipRow, PORTRAIT_LARGE};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};

type IconCache = BTreeMap<(bool, String, u32), Result<String, String>>;

pub struct OperativeContent {
    pub catalogue: Arc<Catalogue>,
    politics: Arc<politics_content::PoliticsContent>,
    skills: Skills,
    operations: installed_content::intelligence::Intelligence,
    capture_duration_days: Result<i32, String>,
    pub warnings: Vec<String>,
    preparation_findings: Vec<crate::content_check::Finding>,
    icons: Mutex<IconCache>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Assignment {
    pub name: Option<String>,
    pub missing: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Experience {
    pub text: Option<String>,
    /// Native Fixed5 fraction: 100000 is a whole level; no clamping.
    pub progress_raw: Option<i64>,
    pub missing: Option<String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct CapturedStatus {
    pub name: Option<String>,
    pub duration: Option<String>,
    /// The native f32 fraction, without clamping.
    pub progress: Option<f32>,
    pub missing: Option<String>,
}

fn capture_duration_days(layers: &installed_content::ResolvedLayers) -> Result<i32, String> {
    // RemainingCaptureDays@140C01EA0 reads this native Int32 define.
    installed_content::defines::read(layers)
        .and_then(|defines| defines.int32("NOperatives", "OPERATIVE_CAPTURE_DURATION_IN_DAYS"))
        .map_err(|error| {
            format!(
                "NDefines.NOperatives.OPERATIVE_CAPTURE_DURATION_IN_DAYS is unavailable: {error}"
            )
        })
}

pub fn content(
    source: &Source,
    context: &LoadContext,
) -> io::Result<Option<Arc<OperativeContent>>> {
    let Some(options) = context.content.as_ref() else {
        return Ok(None);
    };
    let politics = politics_content::politics_content(source, options)?;
    type Cached = Option<Arc<OperativeContent>>;
    let state = source.state::<Cached>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(cached) = state.as_ref()
        && Arc::ptr_eq(&cached.catalogue, &politics.catalogue)
    {
        return Ok(Some(Arc::clone(cached)));
    }
    let loaded = Arc::new(OperativeContent::load(politics)?);
    *state = Some(Arc::clone(&loaded));
    Ok(Some(loaded))
}

impl OperativeContent {
    pub(crate) fn load(politics: Arc<politics_content::PoliticsContent>) -> io::Result<Self> {
        let catalogue = Arc::clone(&politics.catalogue);
        let leaders = Leaders::<String>::load(&catalogue.content.layers, &|_| None)?;
        let operations = installed_content::intelligence::read(&catalogue.content.layers)?;
        let capture_duration_days = capture_duration_days(&catalogue.content.layers);
        use crate::content_check::{Finding, Kind};
        let mut preparation_findings: Vec<_> = leaders
            .skills
            .warnings()
            .iter()
            .map(|warning| Finding::new(Kind::Warning, "operative skills", warning))
            .collect();
        let mut warnings = politics.warnings.clone();
        warnings.extend(leaders.misreads.iter().cloned());
        warnings.extend(leaders.skills.warnings().iter().cloned());
        warnings.extend(operations.warnings.iter().cloned());
        for condition in leaders.skills.conditions() {
            let reason = format!(
                "Operative skill progress depends on recorded modifier {} at {}; its registry is unavailable here.",
                condition.modifier, condition.place
            );
            warnings.push(reason.clone());
            preparation_findings.push(Finding::new(Kind::Offline, "operative skills", reason));
        }
        Ok(Self {
            catalogue,
            politics,
            skills: leaders.skills,
            operations,
            capture_duration_days,
            warnings,
            preparation_findings,
            icons: Mutex::default(),
        })
    }

    pub(crate) fn check_content(&self) -> Vec<crate::content_check::Finding> {
        use crate::content_check::{Finding, Kind};
        let mut findings = self.preparation_findings.clone();
        if let Err(reason) = &self.capture_duration_days {
            findings.push(Finding::new(
                Kind::Refused,
                "OPERATIVE_CAPTURE_DURATION_IN_DAYS",
                reason,
            ));
        }
        let mut assignments: Vec<_> = if self.operations.operation_refusals.is_empty() {
            self.operations
                .operations
                .keys()
                .map(|token| (token.clone(), self.operation(token)))
                .collect()
        } else {
            Vec::new()
        };
        // The native mission/status branches are the same ones the roster uses
        // (140FADF90 / 141E2DC80); no recorded assignment is invented here.
        assignments
            .extend((0..=8).map(|variant| (format!("mission {variant}"), self.mission(variant))));
        assignments.extend(
            [2, 3]
                .into_iter()
                .map(|kind| (format!("cooldown {kind}"), self.cooldown_name(kind))),
        );
        for (subject, assignment) in assignments {
            for reason in [assignment.missing, assignment.icon_missing]
                .into_iter()
                .flatten()
            {
                findings.push(Finding::new(Kind::Refused, &subject, reason));
            }
        }
        findings
    }

    /// LookupCharacter@140C0EEF0 must have proved the operative's full
    /// reference. ResolveLeaderPortrait@140B3A100 selects character role4,
    /// large size1 for native category3. Without that link it uses the
    /// leader's own +0x100 portrait, which this recording does not hold.
    pub fn portrait(
        &self,
        character: u32,
        holders: Option<&[CharacterPortraitRow]>,
    ) -> Result<String, String> {
        let selected =
            character_portraits::recorded_portrait(holders, character, 4, PORTRAIT_LARGE);
        character_portraits::portrait_png(&self.politics, &selected)
    }

    pub fn experience(&self, skill: Option<i32>, raw: i64) -> Experience {
        experience(&self.catalogue.localisation, &self.skills, skill, raw)
    }

    /// Captured agency status@141E2D760 returns two strings and a fraction.
    /// +0xFB8 is its inline CGameDate (getter140C06020), so +8 is exactly
    /// the recorded capture raw hour +0xFC0, not another date.
    pub fn captured_status(
        &self,
        captured_hour: i32,
        now: i32,
        capturer_name: Option<&str>,
    ) -> CapturedStatus {
        let assignment = self.captured_name(capturer_name);
        let mut status = CapturedStatus {
            name: assignment.name,
            missing: assignment.missing,
            ..CapturedStatus::default()
        };
        let duration = if captured_hour == engine::cgame_date::NULL_RAW_HOUR {
            Err("Native captured duration and progress have no recorded capture date (+0xFC0; 141E2D760).".to_owned())
        } else {
            self.capture_duration_days.clone()
        };
        let duration = duration.and_then(|duration| {
            let (remaining, progress) = capture_progress(captured_hour, now, duration);
            status.progress = Some(progress);
            // MOV@141E2D9F0 copies RemainingCaptureDays' ESI into NUM as
            // localiser type5 (Int32), then calls DURATION_DAYS@141E2DA08.
            catalogue::localised_with(
                &self.catalogue.localisation,
                "DURATION_DAYS",
                &[("NUM", Argument::Int32(remaining))],
                View::Plain,
            )
        });
        match duration {
            Ok(duration) => status.duration = Some(duration),
            Err(reason) => {
                status.missing = Some(match status.missing {
                    Some(missing) => format!("{missing} {reason}"),
                    None => reason,
                });
            }
        }
        status
    }

    /// Cooldown status@141E2DC80 localises two independent strings and an f32 fraction.
    pub fn cooldown_status(&self, kind: u32, start: i32, end: i32, now: i32) -> CapturedStatus {
        let (remaining, progress) = cooldown_progress(kind, start, end, now);
        let mut status = CapturedStatus {
            progress: Some(progress),
            ..CapturedStatus::default()
        };
        let assignment = self.cooldown_name(kind);
        status.name = assignment.name;
        status.missing = assignment.missing;
        // MOV@141E2DE0B passes GetModifierCooldownDays' R15D as Int32 NUM.
        match catalogue::localised_with(
            &self.catalogue.localisation,
            "DURATION_DAYS",
            &[("NUM", Argument::Int32(remaining))],
            View::Plain,
        ) {
            Ok(duration) => status.duration = Some(duration),
            Err(reason) => {
                status.missing = Some(match status.missing {
                    Some(previous) => format!("{previous} {reason}"),
                    None => reason,
                })
            }
        }
        status
    }

    fn cooldown_name(&self, kind: u32) -> Assignment {
        // Executable literals in141E2DC80, resolved through recorded localisation.
        let key = match kind {
            2 => "OPERATIVE_HARMED_IN_MISSION",
            3 => "OPERATIVE_FORCED_INTO_HIDING",
            _ => {
                return Assignment {
                    name: Some(String::new()),
                    ..Assignment::default()
                };
            }
        };
        named(catalogue::localised_with(
            &self.catalogue.localisation,
            key,
            &[],
            View::Plain,
        ))
    }

    fn captured_name(&self, capturer_name: Option<&str>) -> Assignment {
        match capturer_name {
            Some(name) => named(catalogue::localised_with(
                &self.catalogue.localisation,
                "OPERATIVE_CAPTURED_BY",
                &[("CAPTURER", Argument::Text(name))],
                View::Plain,
            )), //141E2D760
            None => Assignment {
                missing: Some("The capturer's name at this hour is unknown.".into()),
                ..Assignment::default()
            },
        }
    }

    pub fn assignment(
        &self,
        row: &OperativeMembershipRow,
        operation_token: Option<&str>,
        capturer_name: Option<&str>,
    ) -> Assignment {
        if operative_rows::captured(row.assignment) {
            return self.captured_name(capturer_name);
        }
        // 140C17DF0's state dispatch: cooldown1, disbanded2,
        // mission3, operation4, killed5.
        // Agency status141E2F4C0 only presents 0/1/3/4; terminal2/5 have
        // no agency status text.
        match row.assignment {
            1 => match operative_rows::cooldown(row) {
                Ok(Some((kind, _, _))) => self.cooldown_name(kind),
                Err(reason) => Assignment {
                    missing: Some(reason),
                    ..Assignment::default()
                },
                Ok(None) => Assignment::default(),
            },
            2 => Assignment {
                missing: Some(
                    "Native disbanded assignment has no active agency status text (141E2F4C0)."
                        .into(),
                ),
                ..Assignment::default()
            },
            // The collector stores discriminator0 as None; this is a known
            // native no-mission value (+0x18 getter140FADB40), not a gap.
            3 => self.mission(row.action_variant.unwrap_or(0)),
            4 => match operation_token {
                Some(token) => self.operation(token),
                None => Assignment {
                    missing: Some("The operation definition was not recorded at this read.".into()),
                    ..Assignment::default()
                },
            },
            5 => Assignment {
                missing: Some(
                    "Native killed assignment has no active agency status text (141E2F4C0).".into(),
                ),
                ..Assignment::default()
            },
            other => Assignment {
                missing: Some(format!("Unsupported native operative assignment {other}.")),
                ..Assignment::default()
            },
        }
    }

    fn mission(&self, variant: u32) -> Assignment {
        if let Err(missing) = operative_rows::mission_variant(Some(variant)) {
            return Assignment {
                missing: Some(missing),
                ..Assignment::default()
            };
        }
        let Some(key) = mission_key(variant) else {
            return Assignment {
                missing: Some(format!(
                    "Unsupported native operative mission discriminator {variant}."
                )),
                ..Assignment::default()
            };
        };
        // 140FADF90 appends the executable's _TITLE then localises.
        let mut result = named(catalogue::localised_with(
            &self.catalogue.localisation,
            &format!("{key}_TITLE"),
            &[],
            View::Plain,
        ));
        if variant != 0 {
            // 140FAE190 creates operative_mission_icons_small|<enum>.
            // CGameBitmapFont::text_icon_lookup@14162E880 first searches
            // the executable's GFX_ prefix, then bare alias, and subtracts
            // one from the decimal frame. No GUI fallback on this path.
            const ALIAS: &str = "operative_mission_icons_small";
            let mut icons = self.icons.lock().unwrap_or_else(|error| error.into_inner());
            let icon = icons
                .entry((true, ALIAS.to_owned(), variant - 1))
                .or_insert_with(|| {
                    let sprite = catalogue::text_icon_sprite(&self.catalogue.sprites, ALIAS)
                        .map_err(|error| format!("Mission text icon {ALIAS}: {error}"))?;
                    let decode = *self
                        .catalogue
                        .gui_decode
                        .as_ref()
                        .map_err(|why| format!("Mission text icon {ALIAS}: {why}"))?;
                    catalogue::sprite_frame_png(
                        &self.catalogue.content.layers,
                        sprite,
                        decode,
                        variant - 1,
                        96,
                    )
                    .map_err(|error| format!("Mission text icon {ALIAS} frame{variant}: {error}"))
                })
                .clone();
            match icon {
                Ok(icon) => result.icon = Some(icon),
                Err(reason) => result.icon_missing = Some(reason),
            }
        }
        result
    }

    fn operation(&self, token: &str) -> Assignment {
        let metadata = match self.operations.operation(token) {
            Ok(Some(metadata)) => metadata,
            Ok(None) => {
                return Assignment {
                    missing: Some(format!(
                        "Recorded content does not define operation {token}."
                    )),
                    ..Assignment::default()
                };
            }
            Err(reason) => {
                return Assignment {
                    missing: Some(reason),
                    ..Assignment::default()
                };
            }
        };
        // LocalizeOperationName@140A6DF00 localises definition+0x4D0;
        // COperation's constructor140A6C9F0 leaves it empty, no token fallback.
        let mut result = named(catalogue::localised_with(
            &self.catalogue.localisation,
            &metadata.name,
            &[],
            View::Plain,
        ));
        // Agency portrait141E21710 uses definition+0x4B0 (map_icon),
        // rather than the operation card's +0x490 icon.
        if !metadata.map_icon.is_empty() {
            match self.icon(&metadata.map_icon, 0) {
                Ok(icon) => result.icon = Some(icon),
                Err(reason) => result.icon_missing = Some(reason),
            }
        }
        result
    }

    fn icon(&self, name: &str, frame: u32) -> Result<String, String> {
        let mut icons = self.icons.lock().unwrap_or_else(|error| error.into_inner());
        icons
            .entry((false, name.to_owned(), frame))
            .or_insert_with(|| {
                let sprite = catalogue::created_gui_sprite(&self.catalogue.sprites, name)
                    .map_err(|error| format!("Operative icon {name}: {error}"))?;
                let decode = *self
                    .catalogue
                    .gui_decode
                    .as_ref()
                    .map_err(|why| format!("Operative icon {name}: {why}"))?;
                catalogue::sprite_frame_png(
                    &self.catalogue.content.layers,
                    sprite,
                    decode,
                    frame,
                    96,
                )
                .map_err(|error| format!("Operative icon {name}: {error}"))
            })
            .clone()
    }
}

fn named(name: Result<String, String>) -> Assignment {
    match name {
        Ok(name) => Assignment {
            name: Some(name),
            ..Assignment::default()
        },
        Err(missing) => Assignment {
            missing: Some(missing),
            ..Assignment::default()
        },
    }
}

/// 140C01EA0 day-rounds both raw hours before subtracting; 141E2D760
/// clamps elapsed to zero and elapsed+remaining to one, then divides as f32.
/// Subtraction/addition are native signed dword wrapping operations.
fn capture_progress(captured_hour: i32, now: i32, duration: i32) -> (i32, f32) {
    let captured_day = captured_hour.wrapping_sub(engine::cdate::HOUR_EPOCH) / 24;
    let now_day = now.wrapping_sub(engine::cdate::HOUR_EPOCH) / 24;
    let remaining = captured_day.wrapping_sub(now_day).wrapping_add(duration);
    let elapsed = now_day.wrapping_sub(captured_day).max(0);
    let denominator = elapsed.wrapping_add(remaining).max(1);
    (remaining, elapsed as f32 / denominator as f32)
}

/// GetModifierCooldownDays@140C0A130 uses signed hour arithmetic and truncation.
pub(super) fn cooldown_remaining(kind: u32, end: i32, now: i32) -> i32 {
    if kind == 0 {
        return 0;
    }
    let end = end.wrapping_sub(engine::cdate::HOUR_EPOCH);
    let now = now.wrapping_sub(engine::cdate::HOUR_EPOCH);
    end.wrapping_add(23_i32.wrapping_sub(now)) / 24
}
fn cooldown_progress(kind: u32, start: i32, end: i32, now: i32) -> (i32, f32) {
    let start_day = start.wrapping_sub(engine::cdate::HOUR_EPOCH) / 24;
    let end_day = end.wrapping_sub(engine::cdate::HOUR_EPOCH) / 24;
    let duration = end_day.wrapping_sub(start_day).max(1);
    let remaining = cooldown_remaining(kind, end, now);
    let elapsed = duration.wrapping_sub(remaining).clamp(0, duration);
    (remaining, elapsed as f32 / duration as f32)
}

// GetOperativeMissionLocalizationKey@140FADB60: these are executable
// literal keys, not a catalogue of authored mission types.
fn mission_key(variant: u32) -> Option<&'static str> {
    Some(match variant {
        0 => "OPERATIVE_MISSION_NO_MISSION",
        1 => "OPERATIVE_MISSION_BUILD_INTEL_NETWORK",
        2 => "OPERATIVE_MISSION_QUIET_INTEL_NETWORK",
        3 => "OPERATIVE_MISSION_COUNTER_INTELLIGENCE",
        4 => "OPERATIVE_MISSION_ROOT_OUT_RESISTANCE",
        5 => "OPERATIVE_MISSION_BOOST_IDEOLOGY",
        6 => "OPERATIVE_MISSION_CONTROL_TRADE",
        7 => "OPERATIVE_MISSION_DIPLOMATIC_PRESSURE",
        8 => "OPERATIVE_MISSION_PROPAGANDA",
        _ => return None,
    })
}

/// experience_ratio@140C0A4B0: next skill/category3, signed cost*100000,
/// native signed multiply wraps, signed division truncates toward zero.
/// The sentinel is present, not missing; negative/over-full ratios stand.
fn native_progress(raw: i64, cost: i32) -> i64 {
    let denominator = i64::from(cost) * 100_000;
    if denominator != 0 && (raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
    {
        raw.wrapping_mul(100_000) / denominator
    } else {
        0xffff_ffff
    }
}

fn experience(
    localisation: &installed_content::localisation::Localisation,
    skills: &Skills,
    skill: Option<i32>,
    raw: i64,
) -> Experience {
    let Some(skill) = skill else {
        return Experience {
            missing: Some("The operative's native skill definition was not recorded.".into()),
            ..Experience::default()
        };
    };
    if !skills.conditions().is_empty() {
        return Experience {
            missing: Some(format!(
                "Operative skill levels depend on an unrecorded modifier registry: {}.",
                skills
                    .conditions()
                    .iter()
                    .map(|condition| format!("{} at {}", condition.modifier, condition.place))
                    .collect::<Vec<_>>()
                    .join("; ")
            )),
            ..Experience::default()
        };
    }
    let Some(cost) = skills.next_required_experience(skill, 3) else {
        return Experience {
            missing: Some(format!(
                "Operative skill levels are unsupported: {}",
                skills.warnings().join("; ")
            )),
            ..Experience::default()
        };
    };
    let progress_raw = cost.map(|cost| native_progress(raw, cost));
    // COperativeLeaderItem skill tooltip141AB3910 (also140C03FC0):
    // SKILL is Int32, PROGRESS is Fixed5, native-selected progress/cap key.
    let text = match progress_raw {
        Some(progress) => catalogue::localised_with(
            localisation,
            "LEADER_SKILL_WITH_PROGRESS",
            &[
                ("SKILL", Argument::Int32(skill)),
                ("PROGRESS", Argument::Fixed5(progress)),
            ],
            View::Plain,
        ),
        None => catalogue::localised_with(
            localisation,
            "LEADER_SKILL_AT_CAP",
            &[("SKILL", Argument::Int32(skill))],
            View::Plain,
        ),
    };
    match text {
        Ok(text) => Experience {
            text: Some(text),
            progress_raw,
            missing: None,
        },
        Err(missing) => Experience {
            text: None,
            progress_raw,
            missing: Some(missing),
        },
    }
}

#[cfg(test)]
pub(crate) fn test_content(
    files: &[(&str, &str)],
) -> (Arc<OperativeContent>, crate::catalogue::tests::Folder) {
    let mut files = files.to_vec();
    files.push(("common/defines/00_fixture.lua", "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"fixture_advisor\" } } }"));
    let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&files);
    (test_content_from_catalogue(Arc::new(catalogue)), folder)
}

#[cfg(test)]
pub(crate) fn test_content_from_catalogue(catalogue: Arc<Catalogue>) -> Arc<OperativeContent> {
    let politics =
        Arc::new(politics_content::PoliticsContent::load(Arc::clone(&catalogue)).unwrap());
    let skills = Leaders::<String>::load(&catalogue.content.layers, &|_| None)
        .unwrap()
        .skills;
    let operations = installed_content::intelligence::read(&catalogue.content.layers).unwrap();
    let capture_duration_days = capture_duration_days(&catalogue.content.layers);
    Arc::new(OperativeContent {
        catalogue,
        politics,
        skills,
        operations,
        capture_duration_days,
        warnings: Vec::new(),
        preparation_findings: Vec::new(),
        icons: Mutex::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_signed_progress_keeps_cap_sentinels_and_over_full_values() {
        assert_eq!(native_progress(25_000_000, 100), 250_000);
        assert_eq!(native_progress(-25_000_000, 100), -250_000);
        assert_eq!(native_progress(25_000_000, -100), -250_000);
        assert_eq!(native_progress(1, 0), 0xffff_ffff);
        assert_eq!(native_progress(i64::MAX, 1), 0xffff_ffff);
        assert_eq!(native_progress(i64::MIN, 1), 0xffff_ffff);
        assert_eq!(
            native_progress(100_000_000_000_000, 1),
            100_000_000_000_000i64.wrapping_mul(100_000) / 100_000
        );
    }

    #[test]
    fn operations_reuse_native_metadata_variables_and_nested_reached_forms() {
        let (operations, _) = installed_content::intelligence::parse_operations(
            br#"@name = MOD_OPERATION_NAME
            operation_test = {
              name = @name map_icon = MOD_SPRITE
              equipment = { authored_equipment = 2 civilian_factories = { amount = 1 days = 8 } }
              phases = { authored_phase = { base = 1 modifier = { factor = 0.5 always = yes } } }
              selection_target = { targets = { ROOT FROM } }
              outcome_execute = { set_country_flag = synthetic_flag }
            }"#,
            &installed_content::key_names::KeyNames::default(),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].name, "MOD_OPERATION_NAME");
        assert_eq!(operations[0].map_icon, "MOD_SPRITE");
    }

    #[test]
    fn operations_keep_named_refusals_for_unsupported_targets_and_bad_text() {
        for (bytes, reason) in [
            (
                b"operation_test = { name = label operation_target = { target_array = scope } }"
                    .as_slice(),
                "target_array",
            ),
            (b"operation_test = { name = \xfe }".as_slice(), "UTF-8"),
        ] {
            let refusal = installed_content::intelligence::parse_operations(
                bytes,
                &installed_content::key_names::KeyNames::default(),
                &Default::default(),
            )
            .unwrap_err();
            assert!(refusal.to_string().contains(reason), "{refusal}");
        }
    }

    #[test]
    fn duplicate_native_operation_definitions_keep_first_and_unrelated_names() {
        let (content, _folder) = test_content(&[
            (
                "common/operations/fixture.txt",
                "operation_dup = { name = FIRST } OPERATION_DUP = { name = SECOND } operation_other = { name = OTHER }",
            ),
            (
                "localisation/english/fixture_l_english.yml",
                "\u{feff}l_english:\n FIRST:0 \"First name\"\n SECOND:0 \"Wrong name\"\n OTHER:0 \"Unrelated name\"\n",
            ),
        ]);
        assert_eq!(
            content.operation("operation_dup").name.as_deref(),
            Some("First name")
        );
        assert_eq!(
            content.operation("operation_other").name.as_deref(),
            Some("Unrelated name")
        );
        assert!(content.operations.operation_refusals.is_empty());
    }

    #[test]
    fn mission_discriminators_are_executable_keys_and_unknown_is_refused() {
        for variant in 0..=8 {
            assert!(mission_key(variant).is_some());
        }
        assert!(mission_key(9).is_none());
    }

    #[test]
    fn native_capture_day_rounding_remaining_argument_and_unclamped_progress() {
        let epoch = engine::cdate::HOUR_EPOCH;
        // The capture happened late in a day; the next hour changes the day.
        assert_eq!(
            capture_progress(epoch + 23, epoch + 24, 17),
            (16, 1.0 / 17.0)
        );
        assert_eq!(capture_progress(epoch + 1, epoch + 23, 17), (17, 0.0));
        assert_eq!(capture_progress(epoch + 48, epoch + 24, 17), (18, 0.0));
        assert_eq!(
            capture_progress(epoch, epoch + 24 * 20, 17),
            (-3, 20.0 / 17.0)
        );
        assert_eq!(capture_progress(epoch, epoch + 48, -1), (-3, 2.0));
        // The native day getter truncates toward zero, including before epoch.
        assert_eq!(capture_progress(epoch - 23, epoch, 17), (17, 0.0));
    }

    #[test]
    fn captured_duration_follows_replaced_files_and_later_content_layers() {
        let (_, base) = crate::catalogue::tests::folder_catalogue(&[(
            "common/defines/fixture.lua",
            "NDefines = { NOperatives = { OPERATIVE_CAPTURE_DURATION_IN_DAYS = 17 } }",
        )]);
        let (_, replacement) = crate::catalogue::tests::folder_catalogue(&[(
            "common/defines/fixture.lua",
            "NDefines = { NOperatives = { OPERATIVE_CAPTURE_DURATION_IN_DAYS = 31 } }",
        )]);
        let (_, last) = crate::catalogue::tests::folder_catalogue(&[(
            "common/defines/zz_override.lua",
            "NDefines.NOperatives.OPERATIVE_CAPTURE_DURATION_IN_DAYS = 11 * 3",
        )]);
        let layers = installed_content::ResolvedLayers::from_folders(&[
            (&base.0, &[]),
            (&replacement.0, &[]),
            (&last.0, &[]),
        ])
        .unwrap();
        assert_eq!(capture_duration_days(&layers), Ok(33));
    }

    #[test]
    fn captured_status_uses_content_define_and_separate_native_strings() {
        let (content, _folder) = test_content(&[
            (
                "common/defines/zz_capture.lua",
                "NDefines.NOperatives = { OPERATIVE_CAPTURE_DURATION_IN_DAYS = 5 + 12 }",
            ),
            (
                "localisation/english/fixture_l_english.yml",
                "\u{feff}l_english:\n OPERATIVE_CAPTURED_BY:0 \"Held by $CAPTURER$\"\n DURATION_DAYS:0 \"$NUM|.0$ authored days\"\n",
            ),
        ]);
        let epoch = engine::cdate::HOUR_EPOCH;
        let status = content.captured_status(epoch + 23, epoch + 24, Some("Current name"));
        assert_eq!(status.name.as_deref(), Some("Held by Current name"));
        assert_eq!(status.duration.as_deref(), Some("16 authored days"));
        assert_eq!(status.progress, Some(1.0 / 17.0));
        assert!(status.missing.is_none());
        let unknown_name = content.captured_status(epoch + 23, epoch + 24, None);
        assert!(unknown_name.name.is_none());
        assert_eq!(unknown_name.duration, status.duration);
        assert_eq!(unknown_name.progress, status.progress);
        assert!(unknown_name.missing.unwrap().contains("capturer's name"));
        let null_date =
            content.captured_status(engine::cgame_date::NULL_RAW_HOUR, epoch, Some("Name"));
        assert!(null_date.duration.is_none());
        assert!(null_date.progress.is_none());
        assert!(null_date.missing.unwrap().contains("capture date (+0xFC0"));
    }

    #[test]
    fn captured_status_names_an_unavailable_define_and_keeps_empty_assignment_text() {
        let (content, _folder) = test_content(&[
            (
                "localisation/english/fixture_l_english.yml",
                "\u{feff}l_english:\n OPERATIVE_CAPTURED_BY:0 \"\"\n OPERATIVE_MISSION_NO_MISSION_TITLE:0 \"\"\n",
            ),
            ("common/operations/fixture.txt", "operation_unnamed = { }"),
        ]);
        let epoch = engine::cdate::HOUR_EPOCH;
        let status = content.captured_status(epoch, epoch, Some("Name"));
        assert_eq!(status.name.as_deref(), Some(""));
        assert!(status.duration.is_none());
        assert!(status.progress.is_none());
        assert!(
            status
                .missing
                .unwrap()
                .contains("NDefines.NOperatives.OPERATIVE_CAPTURE_DURATION_IN_DAYS")
        );
        let row = OperativeMembershipRow {
            assignment: 3,
            ..Default::default()
        };
        let assignment = content.assignment(&row, None, None);
        assert_eq!(assignment.name.as_deref(), Some(""));
        assert!(assignment.missing.is_none());
        let row = OperativeMembershipRow {
            assignment: 4,
            ..Default::default()
        };
        let assignment = content.assignment(&row, Some("operation_unnamed"), None);
        assert_eq!(assignment.name.as_deref(), Some(""));
        assert!(assignment.missing.is_none());
    }

    #[test]
    fn native_experience_uses_mod_skill_cost_and_localisation_format_at_cap() {
        let localisation = crate::catalogue::tests::loaded(
            "l_english:\n LEADER_SKILL_WITH_PROGRESS:0 \"Level $SKILL|0$, $PROGRESS|%1$\"\n LEADER_SKILL_AT_CAP:0 \"Level $SKILL|0$ at cap\"\n",
        );
        let skills = Leaders::<String>::from_files(
            vec![(
                "synthetic.txt".into(),
                b"leader_skills = { 2 = { type = operative cost = 80 } }".to_vec(),
            )],
            &|_| None,
        )
        .skills;
        let result = experience(&localisation, &skills, Some(1), 4_000_000);
        assert_eq!(result.progress_raw, Some(50_000));
        assert_eq!(result.text, Some("Level 1, 50.0%".into()));
        assert!(result.missing.is_none());
        let result = experience(&localisation, &skills, Some(2), 9_000_000);
        assert_eq!(result.progress_raw, None);
        assert_eq!(result.text, Some("Level 2 at cap".into()));
        assert!(
            experience(&localisation, &skills, None, 0)
                .missing
                .is_some()
        );
    }

    #[test]
    fn assignment_uses_native_keys_and_refuses_details_the_recording_lacks() {
        let (content, _folder) = test_content(&[
            (
                "localisation/english/fixture_l_english.yml",
                "\u{feff}l_english:\n OPERATIVE_CAPTURED_BY:0 \"Captured by $CAPTURER$\"\n OPERATIVE_MISSION_NO_MISSION_TITLE:0 \"No task\"\n",
            ),
            (
                "common/operations/fixture.txt",
                "operation_synthetic = { name = authored_name }",
            ),
        ]);
        let mut row = OperativeMembershipRow::default();
        assert_eq!(
            content
                .assignment(&row, None, Some("Renamed capturer"))
                .name,
            Some("Captured by Renamed capturer".into())
        );
        for assignment in [1, 2, 5, 6] {
            row.assignment = assignment;
            let result = content.assignment(&row, None, None);
            assert!(result.name.is_none());
            assert!(result.missing.is_some());
        }
        row.assignment = 3;
        assert_eq!(
            content.assignment(&row, None, None).name,
            Some("No task".into())
        );
        row.assignment = 4;
        assert_eq!(
            content
                .assignment(&row, Some("operation_synthetic"), None)
                .name,
            Some("authored_name".into())
        );
        assert!(content.assignment(&row, None, None).missing.is_some());
    }
    #[test]
    fn cooldown_status_uses_remaining_hours_native_clamping_and_modded_text() {
        let (content, _folder) = test_content(&[(
            "localisation/english/fixture_l_english.yml",
            "\u{feff}l_english:\n OPERATIVE_HARMED_IN_MISSION:0 \"Authored injury\"\n OPERATIVE_FORCED_INTO_HIDING:0 \"Authored hiding\"\n DURATION_DAYS:0 \"$NUM|.0$ authored days\"\n",
        )]);
        let e = engine::cdate::HOUR_EPOCH;
        let before = content.cooldown_status(3, e + 2, e + 50, e + 1);
        assert_eq!(before.name.as_deref(), Some("Authored hiding"));
        assert_eq!(before.duration.as_deref(), Some("3 authored days"));
        assert_eq!(before.progress, Some(0.0));
        let held = content.cooldown_status(2, e + 2, e + 50, e + 27);
        assert_eq!(held.name.as_deref(), Some("Authored injury"));
        assert_eq!(held.duration.as_deref(), Some("1 authored days"));
        assert_eq!(held.progress, Some(0.5));
        let after = content.cooldown_status(3, e + 2, e + 50, e + 99);
        assert_eq!(after.duration.as_deref(), Some("-1 authored days"));
        assert_eq!(after.progress, Some(1.0));
        assert_eq!(
            content
                .cooldown_status(0, e + 2, e + 50, e + 27)
                .name
                .as_deref(),
            Some("")
        );
        assert_eq!(cooldown_progress(3, e + 50, e + 2, e), (1, 0.0));
    }
}

#[cfg(test)]
mod corpus;
