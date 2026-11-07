use super::*;
use crate::recording_name::RecordingName;
use crate::test_support::{
    ALLOCATION_CAMPAIGN, allocation_records, allocation_test_directory, named_session_records,
    write_campaign_spool, write_capture,
};

fn hour(year: i32) -> Hour {
    weather_period::EPOCH + year * weather_period::HOURS_PER_YEAR
}

/// One recorder session of the campaign: `(hour, ruling group, players)` per frame.
fn session_records(
    session: SessionId,
    branch: BranchId,
    hours: &[(Hour, &str, usize)],
) -> Vec<Record> {
    named_session_records(session, branch, hours)
}

/// The mod's names for Z01 by ruling group.
fn localisation() -> Localisation {
    [
        ("Z01_democratic", "Republic of Nowhere"),
        ("Z01_fascism", "Nowhere: The Reich"),
        ("Z01", "Nowhere"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

struct Fixture {
    directory: PathBuf,
    output: PathBuf,
    settings: PathBuf,
    localisation: Localisation,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let directory = allocation_test_directory(name);
        Self {
            output: directory.join("recordings"),
            settings: directory.join("settings.json"),
            directory,
            localisation: localisation(),
        }
    }

    /// A session recording into `output`, its recordings noted first as a capture does.
    fn start(&self, name: &str, output: &Path, records: &[Record]) -> PathBuf {
        let session = self.directory.join(name);
        write_campaign_spool(&session, records);
        select_recordings(&session, output).unwrap();
        session
    }

    fn naming(&self, localised: bool) -> Naming<'_> {
        Naming {
            localisation: localised.then_some(&self.localisation),
            settings: &self.settings,
            report: None,
        }
    }

    fn save_into(&self, session: &Path, output: &Path, localised: bool) -> Finalized {
        let finalized = finalize_session_ahead(
            session,
            output,
            &|| Ok(()),
            Ahead::default(),
            self.naming(localised),
            &|_| Destination::Recording,
        )
        .unwrap();
        assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
        finalized
    }

    fn save(&self, session: &Path) -> Finalized {
        self.save_into(session, &self.output, true)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn hours_of(path: &Path) -> Vec<Hour> {
    let replay = open(path).unwrap();
    replay
        .index
        .iter()
        .flat_map(|block| replay.read_block(block).unwrap())
        .map(|frame| frame.hour)
        .collect()
}

fn file_names(folder: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".battleplan"))
        .collect();
    names.sort();
    names
}

#[test]
fn a_name_is_the_country_then_the_years_then_multiplayer() {
    let name = |country: Option<&str>, years, multiplayer| {
        RecordingName {
            country: country.map(str::to_owned),
            years,
            multiplayer,
            by_tag: None,
        }
        .stem()
    };
    assert_eq!(
        name(Some("Germany"), Some((1936, 1941)), false),
        "Germany 1936–1941"
    );
    assert_eq!(
        name(Some("Germany"), Some((1936, 1936)), false),
        "Germany 1936"
    );
    assert_eq!(
        name(Some("Germany"), Some((1936, 1941)), true),
        "Germany 1936–1941 (Multiplayer)"
    );
    assert_eq!(name(None, Some((1936, 1941)), false), "Campaign 1936–1941");
    assert_eq!(name(Some("GER"), None, false), "GER");
    assert_eq!(name(None, None, true), "Campaign (Multiplayer)");
    // A name Windows refuses is made safe; one with nothing left names the campaign.
    assert_eq!(
        name(Some("Nowhere: The \"Reich\"?"), Some((1936, 1941)), false),
        "Nowhere The Reich 1936–1941"
    );
    assert_eq!(
        name(Some("???"), Some((1936, 1936)), false),
        "Campaign 1936"
    );
    assert_eq!(name(Some("CON"), None, false), "CON_");
    // A long name keeps its years and players, shortening only the country.
    let stem = name(Some(&"Ж".repeat(400)), Some((1936, 1941)), true);
    assert!(stem.ends_with("Ж 1936–1941 (Multiplayer)"), "{stem}");
    assert_eq!(recording_files::utf16_len(&stem), recording_files::MAX_STEM);
    assert_eq!(crate::recording_name::year(hour(1936)), Some(1936));
    assert_eq!(crate::recording_name::year(hour(1937) - 1), Some(1936));
    assert_eq!(crate::recording_name::year(weather_period::EPOCH - 1), None);
}

#[test]
fn a_recording_is_named_for_its_contents_and_renamed_as_they_grow() {
    let fixture = Fixture::new("named-recording");
    let first = fixture.start(
        "first",
        &fixture.output,
        &session_records(
            7,
            9,
            &[
                (hour(1936), "democratic", 1),
                (hour(1936) + 1, "democratic", 1),
            ],
        ),
    );
    let named = fixture.output.join("Republic of Nowhere 1936.battleplan");
    assert_eq!(fixture.save(&first).replays, [named.clone()]);
    // Continued: the span grew and the ideology renamed the country, so the file is renamed.
    let second = fixture.start(
        "second",
        &fixture.output,
        &session_records(
            8,
            12,
            &[(hour(1940), "fascism", 1), (hour(1941), "fascism", 1)],
        ),
    );
    let renamed = fixture
        .output
        .join("Nowhere The Reich 1936–1941.battleplan");
    let saved = fixture.save(&second);
    assert_eq!(saved.replays, [renamed.clone()]);
    assert!(saved.unrenamed.is_empty());
    assert_eq!(
        file_names(&fixture.output),
        ["Nowhere The Reich 1936–1941.battleplan"]
    );
    assert_eq!(
        hours_of(&renamed),
        [hour(1936), hour(1936) + 1, hour(1940), hour(1941)]
    );
    // Saving the same session again changes nothing, name included.
    assert_eq!(fixture.save(&second).replays, [renamed.clone()]);
    assert_eq!(file_names(&fixture.output).len(), 1);
    // Without the game's localisation the country is its recorded tag.
    let tagged = fixture.directory.join("tagged");
    let third = fixture.start(
        "third",
        &tagged,
        &session_records(8, 12, &[(hour(1940), "fascism", 1)]),
    );
    assert_eq!(
        fixture.save_into(&third, &tagged, false).replays,
        [tagged.join("Z01 1940.battleplan")]
    );
    // A new file of the campaign starts in another folder; the earlier one is untouched.
    let before = fs::read(&renamed).unwrap();
    let new_file = fixture.output.join("Republic of Nowhere 1936 (2)");
    let fourth = fixture.start(
        "fourth",
        &new_file,
        &session_records(9, 14, &[(hour(1942), "fascism", 1)]),
    );
    assert_eq!(
        fixture.save_into(&fourth, &new_file, true).replays,
        [new_file.join("Nowhere The Reich 1942.battleplan")]
    );
    assert_eq!(fs::read(&renamed).unwrap(), before);
}

#[test]
fn a_multiplayer_recording_says_so_and_a_taken_name_gets_a_number() {
    let fixture = Fixture::new("multiplayer-recording");
    fs::create_dir_all(&fixture.output).unwrap();
    // Another campaign's recording already has the name.
    let taken = fixture
        .output
        .join("Republic of Nowhere 1936 (Multiplayer).battleplan");
    let other = Replay::from_records(
        b"00000000-0000-4000-8000-000000000009".to_vec(),
        &[allocation_records()[..1].to_vec()],
    )
    .unwrap();
    other.write_atomic(&taken).unwrap();
    // A second human joined for one hour; the recording player's country names it.
    let first = fixture.start(
        "first",
        &fixture.output,
        &session_records(
            7,
            9,
            &[
                (hour(1936), "democratic", 1),
                (hour(1936) + 5, "democratic", 2),
            ],
        ),
    );
    let saved = fixture.save(&first);
    let numbered = fixture
        .output
        .join("Republic of Nowhere 1936 (Multiplayer) (2).battleplan");
    assert_eq!(saved.replays, [numbered.clone()]);
    assert_eq!(fs::read(&taken).unwrap(), other.bytes());
    // A later single-player session keeps the recording multiplayer.
    let second = fixture.start(
        "second",
        &fixture.output,
        &session_records(8, 12, &[(hour(1938), "democratic", 1)]),
    );
    assert_eq!(
        fixture.save(&second).replays,
        [fixture
            .output
            .join("Republic of Nowhere 1936–1938 (Multiplayer).battleplan")]
    );
}

#[test]
fn a_failed_rename_keeps_the_saved_recording_and_the_next_save_retries() {
    let fixture = Fixture::new("failed-rename");
    let first = fixture.start(
        "first",
        &fixture.output,
        &session_records(7, 9, &[(hour(1936), "democratic", 1)]),
    );
    let named = fixture.save(&first).replays[0].clone();
    let second = fixture.start(
        "second",
        &fixture.output,
        &session_records(8, 12, &[(hour(1938), "democratic", 1)]),
    );
    let campaign = second.join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap());
    let outcome = finalize_campaign_with_publisher(
        &campaign,
        &segment_paths(&campaign).unwrap(),
        &fixture.output,
        false,
        &mut None,
        None,
        None,
        &|| Ok(()),
        None,
        fixture.naming(true),
        false,
        session_ended(&second),
        place_built,
        |_, _| Err(std::io::Error::other("held open by another program")),
        &mut |_, _, _| {},
    )
    .unwrap();
    // Saved in full under the old name, reported, never duplicated.
    assert_eq!(outcome.replay.as_ref(), Some(&named));
    let reason = outcome.unrenamed.unwrap();
    assert!(
        reason.contains("Republic of Nowhere 1936–1938") && reason.contains("held open"),
        "{reason}"
    );
    assert_eq!(hours_of(&named), [hour(1936), hour(1938)]);
    assert_eq!(
        file_names(&fixture.output),
        ["Republic of Nowhere 1936.battleplan"]
    );
    // The next save of the same contents renames it.
    let renamed = fixture
        .output
        .join("Republic of Nowhere 1936–1938.battleplan");
    assert_eq!(fixture.save(&second).replays, [renamed.clone()]);
    assert_eq!(hours_of(&renamed), [hour(1936), hour(1938)]);
    assert_eq!(
        file_names(&fixture.output),
        ["Republic of Nowhere 1936–1938.battleplan"]
    );
}

#[test]
fn a_name_the_user_chose_sticks_and_later_saves_append_into_it() {
    let fixture = Fixture::new("chosen-name");
    let first = fixture.start(
        "first",
        &fixture.output,
        &session_records(7, 9, &[(hour(1936), "democratic", 1)]),
    );
    let named = fixture.save(&first).replays[0].clone();
    // The user renames it, as the replay app does.
    let folder = fs::canonicalize(&fixture.output).unwrap();
    RecorderSettings::update_at(&fixture.settings, |settings| {
        settings.choose_name(
            &folder,
            std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap(),
            true,
        )
    })
    .unwrap();
    let chosen = recording_files::rename(&named, "Operation Nowhere").unwrap();
    let second = fixture.start(
        "second",
        &fixture.output,
        &session_records(8, 12, &[(hour(1941), "fascism", 1)]),
    );
    let saved = fixture.save(&second);
    assert_eq!(saved.replays, [chosen.clone()]);
    assert_eq!(hours_of(&chosen), [hour(1936), hour(1941)]);
    // Renamed again within the folder, it is still the campaign's and the user's.
    let moved = recording_files::rename(&chosen, "Nowhere, the long war").unwrap();
    let third = fixture.start(
        "third",
        &fixture.output,
        &session_records(9, 14, &[(hour(1942), "fascism", 1)]),
    );
    assert_eq!(fixture.save(&third).replays, [moved.clone()]);
    assert_eq!(hours_of(&moved), [hour(1936), hour(1941), hour(1942)]);
    assert_eq!(
        file_names(&fixture.output),
        ["Nowhere, the long war.battleplan"]
    );
    assert!(!fixture.output.join("chosen-names.json").exists());
}

#[test]
fn a_session_saves_into_the_recording_it_started_with() {
    let fixture = Fixture::new("selected-recording");
    let first = fixture.start(
        "first",
        &fixture.output,
        &session_records(7, 9, &[(hour(1936), "democratic", 1)]),
    );
    let named = fixture.save(&first).replays[0].clone();
    // A copy made while the next session records does not stop its save.
    let second = fixture.start(
        "second",
        &fixture.output,
        &session_records(8, 12, &[(hour(1937), "democratic", 1)]),
    );
    let copy = fixture.output.join("Copy of the campaign.battleplan");
    fs::copy(&named, &copy).unwrap();
    let continued = fixture
        .output
        .join("Republic of Nowhere 1936–1937.battleplan");
    assert_eq!(fixture.save(&second).replays, [continued.clone()]);
    assert_eq!(hours_of(&continued), [hour(1936), hour(1937)]);
    assert_eq!(hours_of(&copy), [hour(1936)]);
    // With the copy there at the start, nothing is saved and the spool stays.
    let third = fixture.start(
        "third",
        &fixture.output,
        &session_records(9, 14, &[(hour(1938), "democratic", 1)]),
    );
    let refused = finalize_session_ahead(
        &third,
        &fixture.output,
        &|| Ok(()),
        Ahead::default(),
        fixture.naming(true),
        &|_| Destination::Recording,
    )
    .unwrap();
    assert!(refused.replays.is_empty());
    assert!(
        refused.errors[0].contains("2 recordings"),
        "{:?}",
        refused.errors
    );
    fs::remove_file(&copy).unwrap();
    // Renamed between the session's start and its save, it is found again.
    let fourth = fixture.start(
        "fourth",
        &fixture.output,
        &session_records(10, 16, &[(hour(1939), "democratic", 1)]),
    );
    let moved = recording_files::rename(&continued, "Moved").unwrap();
    assert_eq!(
        fixture.save(&fourth).replays,
        [fixture
            .output
            .join("Republic of Nowhere 1936–1939.battleplan")]
    );
    assert!(!moved.exists());
}

#[test]
fn a_first_save_interrupted_before_noting_its_recording_continues_it() {
    let fixture = Fixture::new("interrupted-first-save");
    let session = fixture.start(
        "session",
        &fixture.output,
        &session_records(7, 9, &[(hour(1936), "democratic", 1)]),
    );
    let noted = fs::read(session.join(RECORDINGS_FILE)).unwrap();
    let published = fixture.save(&session).replays[0].clone();
    let bytes = fs::read(&published).unwrap();
    // Published, then stopped before the session noted it.
    fs::write(session.join(RECORDINGS_FILE), &noted).unwrap();
    assert_eq!(fixture.save(&session).replays, [published.clone()]);
    assert_eq!(
        file_names(&fixture.output),
        ["Republic of Nowhere 1936.battleplan"]
    );
    assert_eq!(fs::read(&published).unwrap(), bytes);
    // Another session's recording of the campaign appearing meanwhile is refused, not continued.
    let other = fixture.start(
        "other",
        &fixture.output.join("elsewhere"),
        &session_records(8, 12, &[(hour(1937), "democratic", 1)]),
    );
    let foreign = fixture
        .save_into(&other, &fixture.output.join("elsewhere"), true)
        .replays[0]
        .clone();
    let output = fixture.directory.join("later recordings");
    let later = fixture.start(
        "later",
        &output,
        &session_records(11, 15, &[(hour(1938), "democratic", 1)]),
    );
    fs::copy(&foreign, output.join("Appeared.battleplan")).unwrap();
    let refused = finalize_session_ahead(
        &later,
        &output,
        &|| Ok(()),
        Ahead::default(),
        fixture.naming(true),
        &|_| Destination::Recording,
    )
    .unwrap();
    assert!(refused.replays.is_empty());
    assert!(
        refused.errors[0].contains("appeared after the session started"),
        "{:?}",
        refused.errors
    );
    assert_eq!(file_names(&output), ["Appeared.battleplan"]);
}

#[test]
fn a_trimmed_spool_continues_only_its_saved_ancestry_in_the_selected_folder() {
    let fixture = Fixture::new("trimmed-saved-ancestry");
    let session = fixture.start(
        "session",
        &fixture.output,
        &session_records(7, 9, &[(hour(1936), "democratic", 1)]),
    );
    let selection = fs::read(session.join(RECORDINGS_FILE)).unwrap();
    let published = fixture.save(&session).replays[0].clone();
    let bytes = fs::read(&published).unwrap();
    write_session_end(&session, &fixture.output, "completed.json", &json!({})).unwrap();
    let campaign = session.join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap());
    for (_, path) in segment_paths(&campaign).unwrap() {
        fs::remove_file(path).unwrap();
    }
    assert!(spool_session(&segment_paths(&campaign).unwrap()).is_empty());
    fs::write(session.join(RECORDINGS_FILE), &selection).unwrap();
    let moved = recording_files::rename(&published, "Moved").unwrap();
    assert_eq!(fixture.save(&session).replays, [published.clone()]);
    assert!(!moved.exists());
    assert_eq!(fs::read(&published).unwrap(), bytes);

    let saved_path = campaign.join(SAVED_FILE);
    let saved_bytes = fs::read(&saved_path).unwrap();
    let elsewhere = fixture.directory.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let mut saved: Value = serde_json::from_slice(&saved_bytes).unwrap();
    saved["replay"] = json!(elsewhere.join(published.file_name().unwrap()));
    fs::write(&saved_path, serde_json::to_vec(&saved).unwrap()).unwrap();
    fs::write(session.join(RECORDINGS_FILE), &selection).unwrap();
    let refused = finalize_session(&session, &fixture.output).unwrap_err();
    assert!(
        refused.contains("appeared after the session started"),
        "{refused}"
    );
    assert_eq!(fs::read(&published).unwrap(), bytes);

    fs::write(&saved_path, &saved_bytes).unwrap();
    let other = fixture.start(
        "other",
        &elsewhere,
        &session_records(8, 12, &[(hour(1937), "democratic", 1)]),
    );
    let foreign = fixture.save_into(&other, &elsewhere, true).replays[0].clone();
    let foreign_bytes = fs::read(foreign).unwrap();
    fs::write(&published, &foreign_bytes).unwrap();
    let refused = finalize_session(&session, &fixture.output).unwrap_err();
    assert!(
        refused.contains("appeared after the session started"),
        "{refused}"
    );
    assert_eq!(fs::read(&published).unwrap(), foreign_bytes);
    assert_eq!(fs::read(&saved_path).unwrap(), saved_bytes);
}

#[test]
fn a_save_into_another_folder_leaves_the_noted_recording_alone() {
    let fixture = Fixture::new("other-output-folder");
    let first = fixture.start(
        "first",
        &fixture.output,
        &session_records(7, 9, &[(hour(1936), "democratic", 1)]),
    );
    let original = fixture.save(&first).replays[0].clone();
    let bytes = fs::read(&original).unwrap();
    let second = fixture.start(
        "second",
        &fixture.output,
        &session_records(8, 12, &[(hour(1937), "democratic", 1)]),
    );
    let elsewhere = fixture.directory.join("elsewhere");
    let saved = fixture.save_into(&second, &elsewhere, true).replays;
    assert_eq!(
        saved,
        [elsewhere.join("Republic of Nowhere 1937.battleplan")]
    );
    assert_eq!(fs::read(&original).unwrap(), bytes);
}

#[test]
fn a_spool_names_the_recording_it_will_become() {
    let fixture = Fixture::new("spool-name");
    let session = fixture.start(
        "session",
        &fixture.output,
        &session_records(
            7,
            9,
            &[(hour(1936), "democratic", 1), (hour(1939), "fascism", 2)],
        ),
    );
    let name = fixture.predicted(&session).unwrap();
    assert_eq!(name, "Nowhere The Reich 1936–1939 (Multiplayer)");
    let saved = fixture.save(&session);
    assert_eq!(saved.replays[0].file_stem().unwrap(), name.as_str());
}

const CAMPAIGN: &str = match std::str::from_utf8(ALLOCATION_CAMPAIGN) {
    Ok(campaign) => campaign,
    Err(_) => panic!("the campaign id is text"),
};

impl Fixture {
    /// The name a running session's spool says Save gives its recording in the output folder.
    fn predicted(&self, session: &Path) -> Option<String> {
        let mut ahead = Ahead::default();
        ahead.read(session, &self.output).unwrap();
        ahead
            .names(session, &self.output, self.naming(true))
            .remove(CAMPAIGN)
    }

    /// A session that ended without Auto save: its hours wait in the unsaved recording.
    fn stage(&self, name: &str, records: &[Record]) -> PathBuf {
        let session = self.start(name, &self.output, records);
        let staged = finalize_session_ahead(
            &session,
            &self.output,
            &|| Ok(()),
            Ahead::default(),
            self.naming(true),
            &|_| Destination::Unsaved,
        )
        .unwrap();
        assert!(staged.errors.is_empty(), "{:?}", staged.errors);
        write_session_end(
            &session,
            &self.output,
            "waiting.json",
            &json!({"waiting": [{"campaign": CAMPAIGN}]}),
        )
        .unwrap();
        session
    }
}

#[test]
fn checkpointed_spools_keep_the_full_prediction_after_reading_resumes_or_restarts() {
    use crate::test_support::RELOAD;
    for auto_save in [false, true] {
        for first_year in [1936, 1942] {
            let fixture = Fixture::new("checkpointed-spool-name");
            let mut first = session_records(7, 9, &[(hour(first_year), "democratic", 2)]);
            first.pop();
            let session = fixture.start("running", &fixture.output, &first);
            let campaign = session.join(CAMPAIGN);
            let mut later = session_records(7, 12, &[(hour(1939), "fascism", 1)]);
            for record in &mut later {
                match record {
                    Record::BranchStart(branch) => {
                        branch.reason = replay_frame::branches::BranchReason::LoadSignal;
                    }
                    Record::Frame(frame) => {
                        let TableUpdate::Snapshot(TableRows::Clock(rows)) =
                            &mut frame.tables[TableId::Clock.index()]
                        else {
                            unreachable!()
                        };
                        rows[0].play_session_started_at = RELOAD;
                    }
                    Record::Control(ControlRecord::ClockWitness {
                        qpc, load_signal, ..
                    }) => {
                        *qpc += 100;
                        *load_signal = Some(RELOAD);
                    }
                    _ => {}
                }
            }
            fs::write(
                campaign.join("spool.0001.h4s"),
                later
                    .iter()
                    .flat_map(|record| replay::spool::encode_record(record).unwrap())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            write_capture(&campaign, 1, &later);
            let mut ahead = Ahead::default();
            ahead.read(&session, &fixture.output).unwrap();
            let expected =
                ahead.names(&session, &fixture.output, fixture.naming(true))[CAMPAIGN].clone();
            let checkpoint = checkpoint_campaign(
                &session,
                &fixture.output,
                CAMPAIGN,
                0,
                auto_save,
                &mut ahead,
                fixture.naming(true),
                &|| Ok(()),
            )
            .unwrap()
            .unwrap();
            assert_eq!(checkpoint.deleted, 1);
            assert!(!campaign.join("spool.0000.h4s").exists());
            for restarted in [false, true] {
                if restarted {
                    ahead = Ahead::default();
                }
                ahead.read(&session, &fixture.output).unwrap();
                assert_eq!(
                    ahead.names(&session, &fixture.output, fixture.naming(true))[CAMPAIGN],
                    expected,
                    "auto_save={auto_save}, first_year={first_year}, restarted={restarted}"
                );
            }
            assert_eq!(
                fixture.save(&session).replays[0].file_stem().unwrap(),
                expected.as_str()
            );
        }
    }
}

#[test]
fn a_spool_adding_to_an_unsaved_recording_is_named_for_both() {
    let fixture = Fixture::new("unsaved-spool-name");
    fixture.stage(
        "ended",
        &session_records(7, 9, &[(hour(1936), "democratic", 1)]),
    );
    assert!(replay::publication::unsaved_path(&fixture.output, CAMPAIGN).exists());
    // The running session adds later hours, a second human among them.
    let running = fixture.start(
        "running",
        &fixture.output,
        &session_records(8, 12, &[(hour(1939), "fascism", 2)]),
    );
    let name = fixture.predicted(&running).unwrap();
    assert_eq!(name, "Nowhere The Reich 1936–1939 (Multiplayer)");
    let saved = fixture.save(&running).replays[0].clone();
    assert_eq!(saved.file_stem().unwrap(), name.as_str());
    assert_eq!(hours_of(&saved), [hour(1936), hour(1939)]);
}

#[test]
fn a_spool_continuing_a_saved_recording_is_named_as_its_save_renames_it() {
    let fixture = Fixture::new("continued-spool-name");
    let first = fixture.start(
        "first",
        &fixture.output,
        &session_records(
            7,
            9,
            &[(hour(1936), "democratic", 1), (hour(1941), "fascism", 1)],
        ),
    );
    let recording = fixture.save(&first).replays[0].clone();
    // An earlier save loaded again: the latest hour, already saved, still names its country.
    let second = fixture.start(
        "second",
        &fixture.output,
        &session_records(8, 12, &[(hour(1938), "democratic", 1)]),
    );
    assert_eq!(
        fixture.predicted(&second).as_deref(),
        Some("Nowhere The Reich 1936–1941")
    );
    assert_eq!(fixture.save(&second).replays, [recording.clone()]);
    // Later hours rename it, past a file another campaign holds under that name.
    let taken = fixture
        .output
        .join("Republic of Nowhere 1936–1943.battleplan");
    Replay::from_records(
        b"00000000-0000-4000-8000-000000000009".to_vec(),
        &[allocation_records()[..1].to_vec()],
    )
    .unwrap()
    .write_atomic(&taken)
    .unwrap();
    let third = fixture.start(
        "third",
        &fixture.output,
        &session_records(9, 14, &[(hour(1943), "democratic", 1)]),
    );
    let name = fixture.predicted(&third).unwrap();
    assert_eq!(name, "Republic of Nowhere 1936–1943 (2)");
    let renamed = fixture.save(&third).replays[0].clone();
    assert_eq!(renamed.file_stem().unwrap(), name.as_str());
    // A name the user chose is the one Save keeps.
    let folder = fs::canonicalize(&fixture.output).unwrap();
    RecorderSettings::update_at(&fixture.settings, |settings| {
        settings.choose_name(&folder, CAMPAIGN, true)
    })
    .unwrap();
    let chosen = recording_files::rename(&renamed, "Operation Nowhere").unwrap();
    let fourth = fixture.start(
        "fourth",
        &fixture.output,
        &session_records(10, 16, &[(hour(1944), "fascism", 1)]),
    );
    assert_eq!(
        fixture.predicted(&fourth).as_deref(),
        Some("Operation Nowhere")
    );
    assert_eq!(fixture.save(&fourth).replays, [chosen.clone()]);
    // With a copy there when the session started, Save refuses, so nothing is named.
    fs::copy(&chosen, fixture.output.join("Copy.battleplan")).unwrap();
    let fifth = fixture.start(
        "fifth",
        &fixture.output,
        &session_records(11, 18, &[(hour(1945), "fascism", 1)]),
    );
    assert_eq!(fixture.predicted(&fifth), None);
}

#[test]
fn hours_waiting_from_ended_sessions_are_named_as_saving_them_names_them() {
    let fixture = Fixture::new("waiting-name");
    let first = fixture.stage(
        "first",
        &session_records(7, 9, &[(hour(1936), "democratic", 1)]),
    );
    let second = fixture.stage(
        "second",
        &session_records(8, 12, &[(hour(1940), "fascism", 2)]),
    );
    let sessions = [first, second];
    let name = waiting_name(&sessions, &fixture.output, CAMPAIGN, fixture.naming(true)).unwrap();
    assert_eq!(name, "Nowhere The Reich 1936–1940 (Multiplayer)");
    // Hours a session did not stage would be added from its spool: not known here.
    let unstaged = fixture.start(
        "unstaged",
        &fixture.output,
        &session_records(9, 14, &[(hour(1942), "fascism", 1)]),
    );
    let with_unstaged = [sessions[0].clone(), sessions[1].clone(), unstaged];
    assert_eq!(
        waiting_name(
            &with_unstaged,
            &fixture.output,
            CAMPAIGN,
            fixture.naming(true)
        ),
        None
    );
    // Saved as the Recordings view saves them: each session in turn, then what is still staged.
    for session in &sessions {
        fixture.save(session);
    }
    assert_eq!(
        publish_unsaved(&fixture.output, CAMPAIGN, fixture.naming(true)).unwrap(),
        None
    );
    assert_eq!(file_names(&fixture.output), [format!("{name}.battleplan")]);
}

#[test]
fn unavailable_or_ambiguous_naming_inputs_never_predict_a_name() {
    let fixture = Fixture::new("unknown-spool-name");
    for table in [
        TableId::Players,
        TableId::LocalPlayer,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryAppearance,
        TableId::Politics,
        TableId::PoliticalParties,
        TableId::Autonomy,
        TableId::ReservedCountryNaming,
    ] {
        for partial in [false, true] {
            let hours = if partial {
                vec![
                    (hour(1936), "democratic", 1),
                    (hour(1936) + 1, "democratic", 1),
                ]
            } else {
                vec![(hour(1936), "democratic", 1)]
            };
            let mut records = session_records(7, 9, &hours);
            let Record::Frame(frame) = &mut records[if partial { 4 } else { 2 }] else {
                unreachable!()
            };
            frame.keyframe = !partial;
            frame.tables[table.index()] = if partial {
                let TableUpdate::Snapshot(rows) = &frame.tables[table.index()] else {
                    unreachable!()
                };
                TableUpdate::Truncated {
                    rows: rows.clone(),
                    missing: 1,
                }
            } else {
                TableUpdate::Unavailable {
                    reason: "synthetic missing input".into(),
                }
            };
            let session = fixture.start(&format!("{table:?}-{partial}"), &fixture.output, &records);
            assert_eq!(fixture.predicted(&session), None, "{table:?}-{partial}");
            let replay = Replay::from_records(ALLOCATION_CAMPAIGN.to_vec(), &[records]).unwrap();
            assert!(
                Contents::for_prediction(&replay)
                    .unwrap()
                    .predicted(Some(&fixture.localisation))
                    .is_none()
            );
        }
    }
    for players in [
        vec![],
        vec![replay_data::rows::LocalPlayerRow::default(); 2],
    ] {
        let mut frame = crate::test_support::named_frame(9, hour(1936), "democratic", 1);
        frame.tables[TableId::LocalPlayer.index()] =
            TableUpdate::Snapshot(TableRows::LocalPlayer(players));
        let mut name = SpoolName::default();
        name.observe(0, Placement::default(), &frame);
        assert!(name.contents(|_, _| Ok(frame.clone())).is_none());
    }
    let mut records = session_records(7, 9, &[(hour(1936), "democratic", 1)]);
    let Record::Frame(frame) = &mut records[2] else {
        unreachable!()
    };
    frame.tables[TableId::LocalPlayer.index()] = TableUpdate::Unavailable {
        reason: "synthetic".into(),
    };
    let unknown = fixture.start("unknown-player", &fixture.output, &records);
    assert_eq!(fixture.predicted(&unknown), None);
    assert_eq!(
        fixture.save(&unknown).replays[0].file_stem().unwrap(),
        "Campaign 1936"
    );
    let Record::Frame(frame) = &mut records[2] else {
        unreachable!()
    };
    frame.tables[TableId::LocalPlayer.index()] =
        TableUpdate::Snapshot(TableRows::LocalPlayer(vec![
            replay_data::rows::LocalPlayerRow::default(),
        ]));
    let observer = fixture.start(
        "observer",
        &fixture.directory.join("observer-output"),
        &records,
    );
    let mut ahead = Ahead::default();
    ahead
        .read(&observer, &fixture.directory.join("observer-output"))
        .unwrap();
    assert_eq!(
        ahead
            .names(
                &observer,
                &fixture.directory.join("observer-output"),
                fixture.naming(true)
            )
            .get(CAMPAIGN)
            .map(String::as_str),
        Some("Campaign 1936")
    );
}

#[test]
fn unreadable_country_and_overlord_operands_withhold_live_saved_and_waiting_names() {
    use replay_data::rows::{CountryAppearanceRow, CountryLinkRow, CountryRow};
    for missing in ["party", "cosmetic", "overlord-party", "overlord-country"] {
        let fixture = Fixture::new(&format!("unknown-operands-{missing}"));
        let mut records = session_records(7, 9, &[(hour(1936), "democratic", 1)]);
        let Record::Frame(frame) = &mut records[2] else {
            unreachable!()
        };
        if missing == "party" {
            frame.tables[TableId::PoliticalParties.index()] =
                TableUpdate::Snapshot(TableRows::PoliticalParties(vec![]));
        } else {
            let TableUpdate::Snapshot(TableRows::CountryAppearance(rows)) =
                &mut frame.tables[TableId::CountryAppearance.index()]
            else {
                unreachable!()
            };
            if missing == "cosmetic" {
                rows[0].cosmetic_tag = vec![0xff];
            } else {
                rows[0].overlord = 401;
                if missing == "overlord-party" {
                    rows.push(CountryAppearanceRow {
                        country: 401,
                        ..Default::default()
                    });
                }
                let TableUpdate::Snapshot(TableRows::CountryLinks(rows)) =
                    &mut frame.tables[TableId::CountryLinks.index()]
                else {
                    unreachable!()
                };
                rows.push(CountryLinkRow {
                    id: 401,
                    canonical_slot: 31,
                });
                if missing == "overlord-party" {
                    let TableUpdate::Snapshot(TableRows::Countries(rows)) =
                        &mut frame.tables[TableId::Countries.index()]
                    else {
                        unreachable!()
                    };
                    rows.push(CountryRow {
                        id: 401,
                        canonical_slot: 31,
                        tag: b"Z02".to_vec(),
                        ..Default::default()
                    });
                    let TableUpdate::Snapshot(TableRows::Politics(rows)) =
                        &mut frame.tables[TableId::Politics.index()]
                    else {
                        unreachable!()
                    };
                    rows.push(replay_data::rows::PoliticsRow {
                        country: 401,
                        ruling_party: Some(8),
                        ..Default::default()
                    });
                }
            }
        }
        let session = fixture.start("live", &fixture.output, &records);
        assert_eq!(fixture.predicted(&session), None, "{missing}");
        let replay =
            Replay::from_records(ALLOCATION_CAMPAIGN.to_vec(), &[records.clone()]).unwrap();
        assert_eq!(
            Contents::for_prediction(&replay)
                .unwrap()
                .predicted(Some(&fixture.localisation)),
            None,
            "{missing}"
        );
        let staged = fixture.stage("waiting", &records);
        assert_eq!(
            waiting_name(&[staged], &fixture.output, CAMPAIGN, fixture.naming(true)),
            None,
            "{missing}"
        );
    }
}

#[test]
fn predictions_wait_for_decisions_and_exclude_refused_loads_exactly_as_save_does() {
    use crate::test_support::{FIRST_LOAD, RELOAD};
    let fixture = Fixture::new("admitted-spool-name");
    let mut records = session_records(7, 9, &[(hour(1936), "democratic", 1)]);
    records.pop();
    let mut later = session_records(7, 10, &[(hour(1942), "fascism", 2)]);
    for record in &mut later {
        match record {
            Record::BranchStart(branch) => {
                branch.reason = replay_frame::branches::BranchReason::LoadSignal
            }
            Record::Control(ControlRecord::ClockWitness {
                qpc, load_signal, ..
            }) => {
                *qpc = 200;
                *load_signal = Some(RELOAD);
            }
            Record::Frame(frame) => {
                let TableUpdate::Snapshot(TableRows::Clock(rows)) =
                    &mut frame.tables[TableId::Clock.index()]
                else {
                    unreachable!()
                };
                rows[0].play_session_started_at = RELOAD;
            }
            _ => {}
        }
    }
    records.extend(later.into_iter().skip(1));
    let session = fixture.start("session", &fixture.output, &records);
    begin_admissions(&session, &fixture.output).unwrap();
    let mut admissions = Admissions::default();
    admissions
        .admit(&session, &fixture.output, CAMPAIGN, FIRST_LOAD)
        .unwrap();
    let mut ahead = Ahead::default();
    ahead.read(&session, &fixture.output).unwrap();
    assert!(
        !ahead
            .names(&session, &fixture.output, fixture.naming(true))
            .contains_key(CAMPAIGN)
    );
    admissions
        .refuse(&session, &fixture.output, CAMPAIGN, RELOAD, Refusal::Off)
        .unwrap();
    let names = ahead.names(&session, &fixture.output, fixture.naming(true));
    assert_eq!(
        names.get(CAMPAIGN).map(String::as_str),
        Some("Republic of Nowhere 1936")
    );
    admissions
        .admit(&session, &fixture.output, CAMPAIGN, RELOAD)
        .unwrap();
    assert_eq!(
        ahead
            .names(&session, &fixture.output, fixture.naming(true))
            .get(CAMPAIGN)
            .map(String::as_str),
        Some("Nowhere The Reich 1936–1942 (Multiplayer)")
    );
    admissions.loads.get_mut(CAMPAIGN).unwrap().remove(&RELOAD);
    admissions
        .refuse(&session, &fixture.output, CAMPAIGN, RELOAD, Refusal::Off)
        .unwrap();
    assert_eq!(
        ahead.names(&session, &fixture.output, fixture.naming(true)),
        names
    );
    // The session's native capture of both loads saves only the admitted one's branch.
    let native = super::tests::native_recording(&[(9, hour(1936), 1), (10, hour(1942), 1)]);
    fs::write(session.join(CAMPAIGN).join("columnar.0.h4col"), &native).unwrap();
    let saved = fixture.save(&session);
    assert_eq!(
        saved.replays[0].file_stem().unwrap(),
        names[CAMPAIGN].as_str()
    );
    assert_eq!(hours_of(&saved.replays[0]), [hour(1936)]);
    let replay = Replay::open_path(&saved.replays[0]).unwrap();
    assert!(replay.damaged_parts == 0, "{:?}", replay.warnings);
    assert_eq!(replay.sources.len(), 1);
    assert_eq!(replay.recording().run(replay.sources[0].run).unwrap().branch, 9);
}

#[test]
fn branch_naming_combines_only_admitted_summaries_and_decodes_one_country() {
    use crate::test_support::named_frame;
    let mut frames = vec![
        named_frame(90, hour(1942) - 1, "democratic", 1),
        named_frame(90, hour(1942), "democratic", 1),
        named_frame(2, hour(1936), "fascism", 2),
        named_frame(3, hour(1942), "fascism", 1),
    ];
    frames[1].keyframe = false;
    frames[1].tables[TableId::Players.index()] = TableUpdate::Unchanged {
        baseline: hour(1942) - 1,
    };
    frames[2].tables[TableId::LocalPlayer.index()] = TableUpdate::Unavailable {
        reason: "earlier country not needed".into(),
    };
    let mut naming = CampaignNaming::default();
    for (offset, frame) in frames.iter().enumerate() {
        naming.observe(
            0,
            Placement {
                offset,
                ..Placement::default()
            },
            frame,
        );
    }
    let mut read = Vec::new();
    let contents = naming
        .contents(&BTreeSet::new(), |_, placement| {
            read.push(placement.offset);
            Ok(frames[placement.offset].clone())
        })
        .unwrap();
    assert_eq!(read, [3]);
    assert_eq!(
        contents.predicted(Some(&localisation())).unwrap().stem(),
        "Nowhere The Reich 1936\u{2013}1942 (Multiplayer)"
    );
    let contents = naming
        .contents(&BTreeSet::from([3]), |_, placement| {
            read.push(placement.offset);
            Ok(frames[placement.offset].clone())
        })
        .unwrap();
    assert_eq!(read, [3, 0, 1]);
    assert_eq!(
        contents.predicted(Some(&localisation())).unwrap().stem(),
        "Republic of Nowhere 1936\u{2013}1942 (Multiplayer)"
    );
    for dropped in [BTreeSet::new(), BTreeSet::from([3]), BTreeSet::from([2, 3])] {
        assert!(
            naming
                .contents(&dropped, |_, _| panic!("cached branch country read again"))
                .is_some()
        );
    }
    frames[2].tables[TableId::Players.index()] = TableUpdate::Unavailable {
        reason: "unknown roster".into(),
    };
    frames[3] = named_frame(3, hour(1942), "fascism", 2);
    let mut unknown = CampaignNaming::default();
    for (offset, frame) in frames.iter().enumerate() {
        unknown.observe(
            0,
            Placement {
                offset,
                ..Placement::default()
            },
            frame,
        );
    }
    assert!(
        unknown
            .contents(&BTreeSet::new(), |_, _| panic!(
                "unknown roster must withhold"
            ))
            .is_none()
    );
    assert!(
        unknown
            .contents(&BTreeSet::from([2]), |_, placement| Ok(frames
                [placement.offset]
                .clone()))
            .is_some()
    );
}

#[test]
fn a_spool_frame_unreadable_for_now_is_read_again_but_invalid_bytes_are_forgotten() {
    use crate::test_support::named_frame;
    use std::io::{Error, ErrorKind};
    // The hour after the keyframe keeps its countries, so it decodes only after the keyframe.
    let mut frames = vec![
        named_frame(9, hour(1936), "democratic", 1),
        named_frame(9, hour(1936) + 1, "democratic", 1),
    ];
    frames[1].keyframe = false;
    frames[1].tables[TableId::Countries.index()] = TableUpdate::Unchanged {
        baseline: hour(1936),
    };
    let observed = || {
        let mut name = SpoolName::default();
        for (offset, frame) in frames.iter().enumerate() {
            name.observe(
                0,
                Placement {
                    offset,
                    ..Placement::default()
                },
                frame,
            );
        }
        name
    };
    let mut name = observed();
    let mut read = Vec::new();
    // A sharing violation while reading the second hour.
    assert!(
        name.contents(|_, placement| {
            read.push(placement.offset);
            match placement.offset {
                1 => Err(Error::from(ErrorKind::PermissionDenied)),
                offset => Ok(frames[offset].clone()),
            }
        })
        .is_none()
    );
    let contents = name
        .contents(|_, placement| {
            read.push(placement.offset);
            Ok(frames[placement.offset].clone())
        })
        .unwrap();
    assert_eq!(read, [0, 1, 1]);
    assert_eq!(
        contents.predicted(Some(&localisation())).unwrap().stem(),
        "Republic of Nowhere 1936"
    );
    let mut name = observed();
    assert!(
        name.contents(|_, placement| match placement.offset {
            1 => Err(Error::from(ErrorKind::InvalidData)),
            offset => Ok(frames[offset].clone()),
        })
        .is_none()
    );
    assert!(
        name.contents(|_, _| panic!("frames that cannot be decoded are read again"))
            .is_none()
    );
    // The next keyframe names the recording again.
    let next = named_frame(9, hour(1936) + 2, "fascism", 1);
    name.observe(
        0,
        Placement {
            offset: 2,
            ..Placement::default()
        },
        &next,
    );
    assert_eq!(
        name.contents(|_, _| Ok(next.clone()))
            .unwrap()
            .predicted(Some(&localisation()))
            .unwrap()
            .stem(),
        "Nowhere The Reich 1936"
    );
}

#[test]
fn spool_branch_order_span_and_latest_hour_ties_match_save() {
    use crate::test_support::RELOAD;
    for last in [hour(1938), hour(1942), hour(1944)] {
        let fixture = Fixture::new("branch-name-order");
        let mut records = session_records(
            7,
            90,
            &[(hour(1941), "democratic", 1), (hour(1942), "democratic", 1)],
        );
        records.pop();
        let mut later = session_records(7, 2, &[(hour(1936), "fascism", 2), (last, "fascism", 1)]);
        for record in &mut later {
            match record {
                Record::BranchStart(branch) => {
                    branch.reason = replay_frame::branches::BranchReason::LoadSignal
                }
                Record::Frame(frame) => {
                    let TableUpdate::Snapshot(TableRows::Clock(rows)) =
                        &mut frame.tables[TableId::Clock.index()]
                    else {
                        unreachable!()
                    };
                    rows[0].play_session_started_at = RELOAD;
                }
                Record::Control(ControlRecord::ClockWitness {
                    qpc, load_signal, ..
                }) => {
                    *qpc += 200;
                    *load_signal = Some(RELOAD);
                }
                _ => {}
            }
        }
        records.extend(later.into_iter().skip(1));
        let session = fixture.start("session", &fixture.output, &records);
        let mut ahead = Ahead::default();
        ahead.read(&session, &fixture.output).unwrap();
        let name = ahead
            .names(&session, &fixture.output, fixture.naming(true))
            .remove(CAMPAIGN)
            .unwrap();
        let country = if last < hour(1942) {
            "Republic of Nowhere"
        } else {
            "Nowhere The Reich"
        };
        let expected = format!(
            "{country} 1936\u{2013}{} (Multiplayer)",
            crate::recording_name::year(last.max(hour(1942))).unwrap()
        );
        assert_eq!(name, expected);
        assert!(!ahead.read(&session, &fixture.output).unwrap());
        assert_eq!(
            ahead.names(&session, &fixture.output, fixture.naming(true))[CAMPAIGN],
            name
        );
        assert_eq!(
            fixture.save(&session).replays[0].file_stem().unwrap(),
            name.as_str()
        );
    }
}

#[test]
fn rejected_spool_frames_never_contribute_to_a_name() {
    let fixture = Fixture::new("rejected-naming-frame");
    let mut records = session_records(7, 9, &[(hour(1936), "democratic", 1)]);
    records.pop();
    records.push(Record::Frame(crate::test_support::named_frame(
        99,
        hour(1942),
        "fascism",
        2,
    )));
    let session = fixture.start("session", &fixture.output, &records);
    let name = fixture.predicted(&session).unwrap();
    assert_eq!(name, "Republic of Nowhere 1936");
    assert_eq!(
        fixture.save(&session).replays[0].file_stem().unwrap(),
        name.as_str()
    );
}

#[test]
fn exact_published_prefix_accepts_a_file_backed_recording() {
    let fixture = Fixture::new("file-backed-naming-prefix");
    let records = session_records(7, 9, &[(hour(1936), "democratic", 1)]);
    let session = fixture.start("session", &fixture.output, &records);
    let path = fixture.save(&session).replays[0].clone();
    let previous = Replay::open_path(&path).unwrap();
    assert!(previous.is_file_backed());
    let spool = session.join(CAMPAIGN).join("spool.0000.h4s");
    let bytes = fs::read(&spool).unwrap();
    let recovered = recover_reader(
        std::io::Cursor::new(&bytes),
        bytes.len(),
        Recovered::keep,
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(published_prefix(&previous, &[(0, recovered)]), 1);
}

#[test]
fn staged_prediction_checks_the_actual_revision_and_accepts_later_appends() {
    let fixture = Fixture::new("restored-staged-name");
    let first = fixture.stage(
        "first",
        &session_records(7, 9, &[(hour(1936), "democratic", 1)]),
    );
    let staged = replay::publication::unsaved_path(&fixture.output, CAMPAIGN);
    let backup = fs::read(&staged).unwrap();
    let second = fixture.stage(
        "second",
        &session_records(8, 12, &[(hour(1942), "fascism", 2)]),
    );
    let sessions = [first, second.clone()];
    let predicted =
        waiting_name(&sessions, &fixture.output, CAMPAIGN, fixture.naming(true)).unwrap();
    assert_eq!(predicted, "Nowhere The Reich 1936–1942 (Multiplayer)");
    assert!(staged_whole(&sessions[0].join(CAMPAIGN), &staged));
    fs::write(&staged, &backup).unwrap();
    assert_eq!(
        waiting_name(&sessions, &fixture.output, CAMPAIGN, fixture.naming(true)),
        None
    );
    let saved = fixture.save(&second);
    assert_eq!(saved.replays[0].file_stem().unwrap(), predicted.as_str());
    assert_eq!(hours_of(&saved.replays[0]), [hour(1936), hour(1942)]);
}

#[test]
fn staged_predictions_recheck_both_revisions_for_saves_containment_rule() {
    let fixture = Fixture::new("staged-existing-containment");
    let first = fixture.start(
        "first",
        &fixture.output,
        &session_records(7, 9, &[(hour(1936), "democratic", 1)]),
    );
    let existing = fixture.save(&first).replays[0].clone();
    let original = fs::read(&existing).unwrap();
    let waiting = fixture.stage(
        "waiting",
        &session_records(8, 12, &[(hour(1940), "fascism", 1)]),
    );
    let staged = replay::publication::unsaved_path(&fixture.output, CAMPAIGN);
    let staged_bytes = fs::read(&staged).unwrap();
    let name = || {
        waiting_name(
            std::slice::from_ref(&waiting),
            &fixture.output,
            CAMPAIGN,
            fixture.naming(true),
        )
    };
    let predicted = name().unwrap();
    assert!(staged_holds(&staged, &existing));
    let held = Replay::open_path(&staged).unwrap();
    assert!(held.is_file_backed());
    assert!(held.holds(&existing).unwrap());
    drop(held);
    let newer = Replay::open(original.clone())
        .unwrap()
        .append(&[session_records(10, 15, &[(hour(1942), "democratic", 2)])])
        .unwrap();
    newer.write_atomic(&existing).unwrap();
    assert!(!staged_holds(&staged, &existing));
    assert_eq!(name(), None);
    let refused = finalize_session_ahead(
        &waiting,
        &fixture.output,
        &|| Ok(()),
        Ahead::default(),
        fixture.naming(true),
        &|_| Destination::Recording,
    )
    .unwrap();
    assert!(
        refused
            .errors
            .iter()
            .any(|error| error.contains("does not hold all of this recording unchanged")),
        "{:?}",
        refused.errors
    );
    fs::write(&existing, &original).unwrap();
    assert_eq!(name().as_deref(), Some(predicted.as_str()));
    Replay::from_records(
        ALLOCATION_CAMPAIGN.to_vec(),
        &[session_records(20, 25, &[(hour(1944), "democratic", 1)])],
    )
    .unwrap()
    .write_atomic(&staged)
    .unwrap();
    assert!(!staged_holds(&staged, &existing));
    assert_eq!(name(), None);
    fs::write(&staged, staged_bytes).unwrap();
    assert!(staged_holds(&staged, &existing));
    assert_eq!(name().as_deref(), Some(predicted.as_str()));
    assert_eq!(
        fixture.save(&waiting).replays[0].file_stem().unwrap(),
        predicted.as_str()
    );
}

#[test]
fn staged_revisions_remain_verifiable_after_later_appends_without_spool_reads() {
    let fixture = Fixture::new("staged-revision-before-recovery");
    let waiting = fixture.stage(
        "waiting",
        &session_records(7, 9, &[(hour(1936), "democratic", 1)]),
    );
    let staged = replay::publication::unsaved_path(&fixture.output, CAMPAIGN);
    let campaign = waiting.join(CAMPAIGN);
    let stamp = staged_stamp(&campaign, &staged).unwrap();
    use std::os::windows::fs::OpenOptionsExt;
    let hold = || {
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(campaign.join("spool.0000.h4s"))
            .unwrap()
    };
    let held = hold();
    assert_eq!(verify_staged(&campaign, &staged, &stamp), Some(true));
    drop(held);
    fixture.stage(
        "later",
        &session_records(8, 12, &[(hour(1942), "fascism", 2)]),
    );
    let stamp = staged_stamp(&campaign, &staged).unwrap();
    let _held = hold();
    assert_eq!(verify_staged(&campaign, &staged, &stamp), Some(true));
}

#[test]
fn scripted_country_names_keep_the_named_refusal_with_the_fallback_tag() {
    let records = session_records(7, 9, &[(hour(1936), "democratic", 1)]);
    let replay = Replay::from_records(ALLOCATION_CAMPAIGN.to_vec(), &[records]).unwrap();
    let localisation = [(
        "Z01_democratic".to_owned(),
        "[ROOT.SyntheticName]".to_owned(),
    )]
    .into_iter()
    .collect();
    let name = crate::recording_name::recording_name(&replay, Some(&localisation), false).unwrap();
    assert_eq!(name.stem(), "Z01 1936");
    assert_eq!(
        Contents::for_prediction(&replay)
            .unwrap()
            .predicted(Some(&localisation)),
        Some(name.clone())
    );
    let why = name.by_tag.unwrap();
    assert!(
        why.contains("Z01_democratic") && why.contains("unsupported scripted"),
        "{why}"
    );
}

/// A single-player recording's roster scan against saves of hours appended to it.
#[test]
#[ignore = "benchmark"]
fn naming_a_large_recording_costs_little_of_its_save() {
    use replay_data::rows::{ClockRow, LexicalTokenRow};
    use replay_frame::frame::FrameEncoder;
    let fixture = Fixture::new("naming-benchmark");
    fs::create_dir_all(&fixture.output).unwrap();
    let hours = 8760;
    let mut records = session_records(7, 9, &[(hour(1936), "democratic", 1)]);
    let detach = records.pop().unwrap();
    let Record::Frame(first) = &records[2] else {
        unreachable!()
    };
    let template = FrameDecoder::new().apply(first).unwrap();
    let mut encoder = FrameEncoder::new();
    encoder.encode(template.clone(), true).unwrap();
    let mut state = 0x9e37_79b9_u32;
    for at in 1..hours {
        let mut decoded = template.clone();
        decoded.hour = hour(1936) + at;
        *decoded.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![ClockRow {
                hour: decoded.hour,
                ..Default::default()
            }]));
        let literal = (0..16 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        *decoded.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 0,
                literal,
            }]));
        records.push(Record::Frame(encoder.encode(decoded, false).unwrap()));
    }
    drop(detach);
    records.push(Record::Control(ControlRecord::Detach {
        session: 7,
        last_hour: Some(hour(1936) + hours - 1),
    }));
    let existing = fixture.output.join("Republic of Nowhere 1936.battleplan");
    Replay::from_records(ALLOCATION_CAMPAIGN.to_vec(), &[records])
        .unwrap()
        .write_atomic(&existing)
        .unwrap();
    let replay = open(&existing).unwrap();
    let started = Instant::now();
    assert!(!crate::recording_name::multiplayer(&replay, replay.index.len()).unwrap());
    let scan = started.elapsed();
    let session = fixture.start(
        "append",
        &fixture.output,
        &session_records(8, 12, &[(hour(1937) + 1, "democratic", 1)]),
    );
    let started = Instant::now();
    fixture.save(&session);
    let first = started.elapsed();
    // The next save knows the rosters this one read.
    let session = fixture.start(
        "again",
        &fixture.output,
        &session_records(9, 14, &[(hour(1937) + 2, "democratic", 1)]),
    );
    let started = Instant::now();
    fixture.save(&session);
    let next = started.elapsed();
    println!(
        "recording: {} MB, {hours} hours, {} blocks; roster scan {scan:?}; first save {first:?}; next save {next:?}",
        fs::metadata(
            fixture
                .output
                .join("Republic of Nowhere 1936–1937.battleplan")
        )
        .unwrap()
        .len()
            / (1 << 20),
        replay.index.len()
    );
}
