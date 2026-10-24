use super::*;
use crate::recording_name::RecordingName;
use crate::test_support::{
    ALLOCATION_CAMPAIGN, allocation_frame, allocation_records, allocation_test_directory,
    write_campaign_spool,
};
use replay_data::rows::{
    CountryAppearanceRow, CountryLinkRow, CountryRow, LocalPlayerRow, PlayerRow, PoliticalPartyRow,
    PoliticsRow,
};
use replay_frame::frame::TableUpdate;

fn hour(year: i32) -> Hour {
    weather_period::EPOCH + year * weather_period::HOURS_PER_YEAR
}

/// An hour of the modded country Z01 ruled by `group`, with `players` humans.
fn named_frame(branch: BranchId, hour: Hour, group: &str, players: usize) -> Frame {
    let mut frame = allocation_frame(branch, hour, 19);
    for rows in [
        TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 400,
            observer_country: 0,
        }]),
        TableRows::CountryLinks(vec![CountryLinkRow {
            id: 400,
            canonical_slot: 30,
        }]),
        TableRows::Countries(vec![CountryRow {
            id: 400,
            canonical_slot: 30,
            tag: b"Z01".to_vec(),
            ..Default::default()
        }]),
        TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 400,
            ..Default::default()
        }]),
        TableRows::Politics(vec![PoliticsRow {
            country: 400,
            ruling_party: Some(7),
            ..Default::default()
        }]),
        TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 400,
            id: 7,
            token: group.as_bytes().to_vec(),
            ..Default::default()
        }]),
        TableRows::Players(
            (0..players)
                .map(|id| PlayerRow {
                    id: id as i32,
                    country: 400,
                    ..Default::default()
                })
                .collect(),
        ),
    ] {
        let index = rows.table().index();
        frame.tables[index] = TableUpdate::Snapshot(rows);
    }
    frame
}

/// One recorder session of the campaign: `(hour, ruling group, players)` per frame.
fn session_records(
    session: SessionId,
    branch: BranchId,
    hours: &[(Hour, &str, usize)],
) -> Vec<Record> {
    let mut records = allocation_records();
    records.truncate(2);
    if let Record::SessionStart(start) = &mut records[0] {
        start.id = session;
    }
    if let Record::BranchStart(start) = &mut records[1] {
        start.id = branch;
        start.session = session;
        start.first_hour = hours[0].0;
        start.loaded_hour = Some(hours[0].0 - 1);
    }
    for (at, &(hour, group, players)) in hours.iter().enumerate() {
        records.push(Record::Frame(named_frame(branch, hour, group, players)));
        records.push(Record::Control(ControlRecord::ClockWitness {
            session,
            qpc: 100 * (at as u64 + 1),
            hour,
            load_signal: Some(19),
        }));
    }
    records.push(Record::Control(ControlRecord::Detach {
        session,
        last_hour: Some(hours.last().unwrap().0),
    }));
    records
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
        |replay, destination| replay.write_atomic(destination),
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
    let mut ahead = Ahead::default();
    ahead.read(&session, &fixture.output).unwrap();
    let names = ahead.names(Some(&fixture.localisation));
    let name = &names[std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap()];
    assert_eq!(name.stem(), "Nowhere The Reich 1936–1939 (Multiplayer)");
    let saved = fixture.save(&session);
    assert_eq!(saved.replays[0].file_stem().unwrap(), name.stem().as_str());
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
