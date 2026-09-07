use super::*;
use compact_contract::country_capital_slot::Outcome;
use sha2::{Digest, Sha256};

struct Image {
    module: Vec<u64>,
    roster: RosterFixture,
    politics: PoliticsWindowFixture,
    state_object: Box<AlignedBytes<0x60>>,
    template: Box<AlignedBytes<0xa8>>,
    state_slots: Box<AlignedBytes<0x20>>,
    database: Box<AlignedBytes<0x40>>,
    template_slots: Box<AlignedBytes<0x100>>,
}

impl Image {
    fn new(hour: i32) -> Self {
        let mut image = Self {
            module: vec![0; (0x3316f00 + 7) / 8], roster: RosterFixture::new(),
            politics: politics_window_fixture(), state_object: AlignedBytes::new(),
            template: AlignedBytes::new(), state_slots: AlignedBytes::new(),
            database: AlignedBytes::new(), template_slots: AlignedBytes::new(),
        };
        let base = image.module.as_ptr() as usize;
        image.politics.country_vtable = base + COUNTRY_VTABLE_RVA;
        image.politics.country.write_u64(0, image.politics.country_vtable as u64);
        image.politics.country.write_u32(OBJ_TAG_DB_ID, 1);
        image.politics.country.write_i32(OBJ_CAPITAL, 1);
        image.politics.country.write_i32(OBJ_ORIGINAL_CAPITAL, 0);
        image.politics._status.write_u64(0, (base + POLITICAL_STATUS_VTABLE_RVA) as u64);
        for party in [&mut image.politics._democratic_party, &mut image.politics._neutral_party] {
            party.write_u64(0, (base + POLITICAL_PARTY_VTABLE_RVA) as u64);
        }
        for group in [&mut image.politics._democratic_group, &mut image.politics._neutral_group] {
            group.write_u64(0, (base + IDEOLOGY_GROUP_VTABLE_RVA) as u64);
        }
        image.politics._idea.write_u64(0, (base + IDEA_VTABLE_RVA_PRIMARY) as u64);
        let gs = image.roster.at(ROSTER_FIXTURE_GS);
        image.module[GAME_STATE_SINGLETON_RVA / 8] = gs as u64;
        image.module[0x3316ca0 / 8] = image.database.addr() as u64;
        image.roster.arena.write_u64(image.roster.pad + 0x2c8, image.state_slots.addr() as u64);
        image.roster.arena.write_u32(image.roster.pad + 0x2d4, 4);
        image.state_slots.write_u64(8, image.state_object.addr() as u64);
        image.state_object.write_u64(0, (base + STATE_VTABLE_RVA) as u64);
        image.state_object.write_u64(0x30, image.template.addr() as u64);
        image.state_object.write_i32(0x58, 73);
        image.template.write_i32(0xa0, 17);
        image.database.write_u64(0x28, image.template_slots.addr() as u64);
        image.database.write_u32(0x34, 18);
        image.template_slots.write_u64(17 * 8, image.template.addr() as u64);
        image.set_hour(hour);
        image
    }
    fn set_hour(&mut self, hour: i32) {
        self.roster.arena.write_i32(self.roster.pad + GS_CURRENT_DATE_OFFSET, hour);
    }
    fn collector(&self) -> CollectorState {
        let mut state = empty_collector_state();
        state.base = self.module.as_ptr() as usize;
        state.daily_match = true; state.long_string_match = true;
        state.date_ptr = self.roster.at(ROSTER_FIXTURE_GS) as u64;
        state.country_database_ptr = self.roster.at(ROSTER_FIXTURE_DB) as u64;
        state.country_identity = CountryIdentityCache {
            status: "gamestate_country_table_verified", scan_ms: 0,
            country_vtable: self.politics.country_vtable, bridge_rows_supplied: 0,
            record_rows_supplied: 0, rows: vec![test_country_row("GER", 1, self.politics.country.addr())],
            rejected_objects: 0, duplicate_tags: 0, completeness: CountryRosterCompleteness::Complete,
        };
        state
    }
    fn read(&self, tracker: &mut CompactCountryFrameTracker) -> (Vec<u8>, DomainClockBracket) {
        page_cache_new_frame();
        let state = self.collector(); let mut control = raw_test_control();
        control.idea_group_filter = politics_fixture_idea_groups();
        let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), state.base,
            state.date_ptr as usize + GS_CURRENT_DATE_OFFSET);
        let read = {
            let _guard = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_POLITICS);
            read_compact_country_domain_bundle(&state, &control,
                vec![*compact_country_domain_specs().iter().find(|s| s.id == COMPACT_DOMAIN_POLITICS).unwrap()], 0, tracker)
        };
        let brackets = lanes.take_clock_brackets(); assert_eq!(brackets.len(), 1);
        let bracket = brackets.into_iter().next().unwrap();
        let opening = bracket.opening.unwrap();
        assert_eq!(bracket.closing.unwrap().hour, opening.hour);
        let records = country_bundle_publication_records(&read, 0, None);
        let data = records.into_iter().find(|r| r.is_data_record).unwrap().payload;
        let decoded = compact_contract::country_name_inputs::decode_politics_capital_slots(&data, Some(opening.hour)).unwrap();
        assert_eq!(decoded.slots.observation_ordinal, opening.ordinal);
        assert_eq!(decoded.slots.groups.len(), 1);
        assert_eq!(decoded.slots.groups[0].operands.base_raw, Some(1));
        assert_eq!(decoded.slots.groups[0].operands.override_raw, Some(0));
        assert_eq!(decoded.slots.groups[0].outcome, Outcome::Present { slot_count: 4, state_id: 73, template_id: 17 });
        (data, bracket)
    }
}

fn append(stream: &mut Vec<u8>, sequence: &mut u64, hour: i32, domain: u16, kind: u16, bytes: &[u8]) {
    append_compact_record(stream, *sequence, hour, domain, kind, 0, bytes); *sequence += 1;
}
fn seal(stream: &[u8], sequence: u64, hour: i32) -> (Vec<u8>, serde_json::Value) {
    let mut raw = stream.to_vec(); let mut sequence = sequence;
    append(&mut raw, &mut sequence, hour, compact_contract::COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END, &[0; 28]);
    let mut fold = recorder_host::world_fold::WorldFold::new("ccs1_joint");
    fold.ingest_stream(&raw);
    let snapshots = fold.finish();
    let snapshot = serde_json::from_slice(&snapshots.last().unwrap().2).unwrap();
    (raw, snapshot)
}
fn artifact(name: &str, raw: &[u8], snapshot: serde_json::Value) -> serde_json::Value {
    let snapshot_bytes = serde_json::to_vec(&snapshot).unwrap();
    serde_json::json!({"stage":name,"snapshot_sha256":format!("{:x}", Sha256::digest(&snapshot_bytes)),
        "raw_h4cr_sha256":format!("{:x}", Sha256::digest(raw)),
        "raw_h4cr_hex":raw.iter().map(|b| format!("{b:02x}")).collect::<String>(),"snapshot":snapshot})
}

#[test]
fn actual_bracketed_cpni3_publisher_fold_fixture_keeps_historical_refusal_until_next_hour() {
    let hour = DATE_EPOCH_CONST + 16_959_360;
    let mut image = Image::new(hour); let mut tracker = CompactCountryFrameTracker::default();
    let (first, first_bracket) = image.read(&mut tracker);
    assert!(tracker.country_politics.is_empty()); assert!(tracker.has_staged_advance());
    tracker.roll_back_staged_advance();
    assert!(tracker.country_politics.is_empty()); assert!(!tracker.has_staged_advance());
    let (accepted, accepted_bracket) = image.read(&mut tracker);
    assert!(accepted_bracket.opening.unwrap().ordinal > first_bracket.opening.unwrap().ordinal);
    assert_eq!(&first[56..compact_contract::country_name_inputs::decode_politics_capital_slots(&first,None).unwrap().tail_offset],
        &accepted[56..compact_contract::country_name_inputs::decode_politics_capital_slots(&accepted,None).unwrap().tail_offset]);
    tracker.commit_staged_advance();
    let baseline = tracker.country_politics[&1].0.clone();
    let identity = compact_country_identity_data_payload(
        &compact_contract::country_name_inputs::CountryLinkIndices::Unavailable(NameInputRefusal::Unreadable),
        &[CompactCountryIdentityRow {tag_db_id:1,tag:"GER".into(),cosmetic_tag:NameToken::Absent,
            effective_name_tag:Err(NameInputRefusal::Unreadable),graphical_culture:NameToken::Unavailable(NameInputRefusal::Unreadable)}],
        0,0,&CompactCountryIdentityExtension::unavailable(0));
    let mut stream=Vec::new();let mut seq=1;
    append(&mut stream,&mut seq,hour,18,COMPACT_RECORD_TYPE_DOMAIN_DATA,&identity);
    append(&mut stream,&mut seq,hour,14,COMPACT_RECORD_TYPE_DOMAIN_DATA,&accepted);
    let (raw, snapshot)=seal(&stream,seq,hour);
    assert!(snapshot["provenance"]["collector"]["domain_problems"].as_array().unwrap().iter().all(|p|p["domain_id"]!=14));
    let mut stages=vec![artifact("H_accept",&raw,snapshot)];
    let (mut rejected, _) = image.read(&mut tracker);
    rejected[48] ^= 1;
    assert!(compact_contract::country_name_inputs::decode_politics_capital_slots(&rejected,Some(hour)).is_err());
    tracker.roll_back_staged_advance(); assert!(tracker.country_politics[&1].0 == baseline, "rollback retains the committed politics fingerprint");
    append(&mut stream,&mut seq,hour,14,COMPACT_RECORD_TYPE_DOMAIN_DATA,&rejected);
    let (raw,snapshot)=seal(&stream,seq,hour);
    assert!(snapshot["countries"]["GER"]["politics"]["capital_slot"].is_null());
    stages.push(artifact("H_refusal",&raw,snapshot));
    let (recovered, recovered_bracket)=image.read(&mut tracker);
    append(&mut stream,&mut seq,hour,14,COMPACT_RECORD_TYPE_DOMAIN_DATA,&recovered);tracker.commit_staged_advance();
    let (raw,snapshot)=seal(&stream,seq,hour);
    assert_eq!(snapshot["countries"]["GER"]["politics"]["capital_slot"]["outcome"]["template_id"],17);
    assert!(snapshot["provenance"]["collector"]["domain_problems"].as_array().unwrap().iter().any(|p|p["domain_id"]==14));
    stages.push(artifact("H_raw_recovery_historical_refusal",&raw,snapshot));
    append(&mut stream,&mut seq,hour,compact_contract::COMPACT_DOMAIN_FRAME,COMPACT_RECORD_TYPE_FRAME_END,&[0;28]);
    image.set_hour(hour+1); let (next,next_bracket)=image.read(&mut tracker);
    assert!(next_bracket.opening.unwrap().ordinal>recovered_bracket.opening.unwrap().ordinal);
    append(&mut stream,&mut seq,hour+1,18,COMPACT_RECORD_TYPE_DOMAIN_DATA,&identity);
    append(&mut stream,&mut seq,hour+1,14,COMPACT_RECORD_TYPE_DOMAIN_DATA,&next);tracker.commit_staged_advance();
    let (raw,snapshot)=seal(&stream,seq,hour+1);
    assert_eq!(snapshot["countries"]["GER"]["politics"]["capital_slot"]["game_hour_raw"],hour+1);
    assert!(snapshot["provenance"]["collector"]["domain_problems"].as_array().unwrap().iter().all(|p|p["domain_id"]!=14));
    stages.push(artifact("H_plus_1_clean",&raw,snapshot));
    let output=serde_json::json!({"schema":1,"producer":"actual_bracketed_cpni3_publisher_fold_fixture_keeps_historical_refusal_until_next_hour",
        "acquisition":"ReaderClockBracketGuard/read_compact_country_domain_bundle/country_bundle_publication_records",
        "stages":stages});
    if let Some(path)=std::env::var_os("BATTLE_PLAN_CCS1_JOINT_OUTPUT").or_else(|| std::env::var_os("HOI4_CCS1_JOINT_OUTPUT")) {
        use std::io::Write;
        let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
        file.write_all(&serde_json::to_vec_pretty(&output).unwrap()).unwrap();
    }
}

fn full_panel_input(path_env: &str, hash_env: &str, limit: u64) -> Vec<u8> {
    let path = std::env::var_os(path_env).expect(path_env);
    let expected = std::env::var(hash_env).expect(hash_env);
    assert!(std::fs::metadata(&path).unwrap().len() <= limit);
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), expected, "{path_env} digest");
    bytes
}

fn seal_full_panel(stream: &[u8], sequence: u64, hour: i32) -> (Vec<u8>, serde_json::Value) {
    let mut raw = stream.to_vec();
    let mut sequence = sequence;
    append(&mut raw, &mut sequence, hour, compact_contract::COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END, &[0; compact_contract::record_semantics::COMPACT_FRAME_END_PAYLOAD_LEN]);
    let mut validator = compact_contract::stream_validator::StatefulStreamValidator::identity_v1();
    let mut offset = 0;
    while offset < raw.len() {
        let header = compact_contract::parse_compact_record_header(&raw[offset..]).unwrap();
        let end = offset.checked_add(header.record_len()).unwrap();
        assert!(end <= raw.len());
        validator.validate_record(&header, &raw[offset + usize::from(COMPACT_RECORD_HEADER_LEN)..end]).unwrap();
        offset = end;
    }
    assert_eq!(offset, raw.len());
    let mut fold = recorder_host::world_fold::WorldFold::new(validator.identity().unwrap().campaign_id_hex());
    fold.ingest_stream(&raw);
    let snapshots = fold.finish();
    (raw, serde_json::from_slice(&snapshots.last().unwrap().2).unwrap())
}

#[test]
#[ignore = "requires the actual native A27 and D53 generator artifacts and their hashes"]
fn full_panel_actual_generators_share_cid1_and_preserve_capital_gap_history() {
    use compact_contract::build_profile as profile;
    use recorder_host::native_design_catalog::custody::StreamBinding;
    let catalog_bytes = full_panel_input("BATTLE_PLAN_FULL_PANEL_A27_INPUT", "BATTLE_PLAN_FULL_PANEL_A27_SHA256", 4 * 1024 * 1024);
    let catalog: serde_json::Value = serde_json::from_slice(&catalog_bytes).unwrap();
    let cid1_record = full_panel_input("BATTLE_PLAN_FULL_PANEL_CID1_INPUT", "BATTLE_PLAN_FULL_PANEL_CID1_SHA256", 2048);
    let first = recorder_host::duplex_session::identity_control::ValidatedFirstCompactRecord::identity_v1(0, &cid1_record).unwrap();
    let cid1 = first.validated_cid1().unwrap();
    let binding = StreamBinding::from_cid1(0, cid1);
    assert_eq!(catalog["authority"]["stream_binding"], serde_json::to_value(&binding).unwrap());
    assert_eq!(cid1.game_unique_id(), "a4b17fed-ed12-4361-9064-fb6576638110");
    assert_eq!(cid1.game_unique_seed(), -1234);
    assert_eq!(cid1.game_hour_raw(), DATE_EPOCH_CONST + 16_959_360);
    let d53_bytes = full_panel_input("BATTLE_PLAN_FULL_PANEL_D53_INPUT", "BATTLE_PLAN_FULL_PANEL_D53_SHA256", 1024 * 1024);
    let d53: serde_json::Value = serde_json::from_slice(&d53_bytes).unwrap();
    assert_eq!(d53["schema"], 1);
    assert_eq!(d53["cid1_record_sha256"], format!("{:x}", Sha256::digest(&cid1_record)));
    assert_eq!(d53["cid1_payload_sha256"], binding.cid1_payload_sha256);
    assert_eq!(d53["logical_stream_id"], binding.logical_stream_id);
    assert_eq!(d53["game_unique_id"], binding.game_unique_id);
    assert_eq!(d53["game_unique_seed"], binding.game_unique_seed);
    let observations = d53["observations"].as_array().unwrap();
    assert_eq!(observations.len(), 2);
    let mut faction_payloads = Vec::new();
    for (offset, observation) in observations.iter().enumerate() {
        assert_eq!(observation["game_hour_raw"], cid1.game_hour_raw() + offset as i32);
        let payload: Vec<u8> = serde_json::from_value(observation["payload"].clone()).unwrap();
        assert!(payload.len() <= 64 * 1024);
        assert_eq!(observation["payload_sha256"], format!("{:x}", Sha256::digest(&payload)));
        let decoded = diplomacy_intel_operatives_live::compact::decode_snapshot_payload(0, &payload).unwrap();
        assert!(decoded.faction_roster_complete && decoded.subject_autonomy_status_complete);
        assert_eq!(decoded.factions.len(), 1);
        assert_eq!(decoded.factions[0].member_tag_db_ids, vec![1]);
        assert!(decoded.subjects.is_empty());
        faction_payloads.push(payload);
    }
    let hour = cid1.game_hour_raw();
    let mut image = Image::new(hour);
    let mut uuid = AlignedBytes::<40>::new();
    uuid.write_bytes(0, cid1.game_unique_id().as_bytes());
    image.roster.arena.write_i32(image.roster.pad + profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF, cid1.game_unique_seed());
    image.roster.arena.write_u64(image.roster.pad + profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF, uuid.addr() as u64);
    image.roster.arena.write_u64(image.roster.pad + profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF + 16, 36);
    image.roster.arena.write_u64(image.roster.pad + profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF + 24, 36);
    let context = CampaignIdentityReadContext::from(&image.collector());
    page_cache_new_frame();
    let (observed_cid1, observed_record) = build_campaign_identity_candidate(&context).unwrap();
    assert_eq!(observed_record, cid1_record, "actual collector campaign bytes must match A27 binding");
    assert_eq!(StreamBinding::from_cid1(0, &observed_cid1), binding);
    image.roster.arena.write_i32(image.roster.pad + profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF, -1233);
    page_cache_new_frame();
    let (wrong_campaign, _) = build_campaign_identity_candidate(&context).unwrap();
    assert_ne!(StreamBinding::from_cid1(0, &wrong_campaign), binding);
    image.roster.arena.write_i32(image.roster.pad + profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF, -1234);
    page_cache_new_frame();
    assert_eq!(build_campaign_identity_candidate(&context).unwrap().1, cid1_record);

    let identity = compact_country_identity_data_payload(
        &compact_contract::country_name_inputs::CountryLinkIndices::Unavailable(NameInputRefusal::Unreadable),
        &[CompactCountryIdentityRow { tag_db_id: 1, tag: "GER".into(), cosmetic_tag: NameToken::Absent,
            effective_name_tag: Err(NameInputRefusal::Unreadable), graphical_culture: NameToken::Unavailable(NameInputRefusal::Unreadable) }],
        0, 0, &CompactCountryIdentityExtension::unavailable(0));
    let mut tracker = CompactCountryFrameTracker::default();
    let (accepted, _) = image.read(&mut tracker);
    tracker.commit_staged_advance();
    let baseline = tracker.country_politics[&1].0.clone();
    let mut stream = observed_record;
    let mut seq = 1;
    append(&mut stream, &mut seq, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &identity);
    append(&mut stream, &mut seq, hour, 53, COMPACT_RECORD_TYPE_DOMAIN_DATA, &faction_payloads[0]);
    append(&mut stream, &mut seq, hour, 14, COMPACT_RECORD_TYPE_DOMAIN_DATA, &accepted);
    let (raw, snapshot) = seal_full_panel(&stream, seq, hour);
    assert_eq!(snapshot["diplomacy_world"]["domain53_extra"]["faction_roster_status"], "measured_nonempty");
    assert_eq!(snapshot["diplomacy_world"]["domain53_extra"]["faction_member_order_schema"], 1);
    let mut stages = vec![artifact("H_accept", &raw, snapshot)];
    let (mut rejected, _) = image.read(&mut tracker);
    rejected[48] ^= 1;
    assert!(compact_contract::country_name_inputs::decode_politics_capital_slots(&rejected, Some(hour)).is_err());
    tracker.roll_back_staged_advance();
    assert!(tracker.country_politics[&1].0 == baseline);
    append(&mut stream, &mut seq, hour, 14, COMPACT_RECORD_TYPE_DOMAIN_DATA, &rejected);
    let (raw, snapshot) = seal_full_panel(&stream, seq, hour);
    assert!(snapshot["countries"]["GER"]["politics"]["capital_slot"].is_null());
    stages.push(artifact("H_refusal", &raw, snapshot));
    let (recovered, recovered_bracket) = image.read(&mut tracker);
    append(&mut stream, &mut seq, hour, 14, COMPACT_RECORD_TYPE_DOMAIN_DATA, &recovered);
    tracker.commit_staged_advance();
    let (raw, snapshot) = seal_full_panel(&stream, seq, hour);
    assert_eq!(snapshot["countries"]["GER"]["politics"]["capital_slot"]["outcome"]["template_id"], 17);
    assert!(snapshot["provenance"]["collector"]["domain_problems"].as_array().unwrap().iter().any(|p| p["domain_id"] == 14));
    stages.push(artifact("H_raw_recovery_historical_refusal", &raw, snapshot));
    append(&mut stream, &mut seq, hour, compact_contract::COMPACT_DOMAIN_FRAME, COMPACT_RECORD_TYPE_FRAME_END, &[0; compact_contract::record_semantics::COMPACT_FRAME_END_PAYLOAD_LEN]);
    image.set_hour(hour + 1);
    page_cache_new_frame();
    let next_identity = read_campaign_identity_observation_from_context(&context).unwrap();
    assert_eq!(next_identity.game_unique_id, binding.game_unique_id);
    assert_eq!(next_identity.game_unique_seed, binding.game_unique_seed);
    assert_eq!(next_identity.game_hour_raw, hour + 1);
    let (next, next_bracket) = image.read(&mut tracker);
    assert!(next_bracket.opening.unwrap().ordinal > recovered_bracket.opening.unwrap().ordinal);
    append(&mut stream, &mut seq, hour + 1, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &identity);
    append(&mut stream, &mut seq, hour + 1, 53, COMPACT_RECORD_TYPE_DOMAIN_DATA, &faction_payloads[1]);
    append(&mut stream, &mut seq, hour + 1, 14, COMPACT_RECORD_TYPE_DOMAIN_DATA, &next);
    tracker.commit_staged_advance();
    let (raw, snapshot) = seal_full_panel(&stream, seq, hour + 1);
    assert_eq!(snapshot["countries"]["GER"]["politics"]["capital_slot"]["game_hour_raw"], hour + 1);
    assert!(snapshot["provenance"]["collector"]["domain_problems"].as_array().unwrap().iter().all(|p| p["domain_id"] != 14 && p["domain_id"] != 53));
    stages.push(artifact("H_plus_1_clean", &raw, snapshot));
    for stage in &stages {
        assert_eq!(stage["snapshot"]["diplomacy_world"]["domain53_extra"]["faction_member_order_schema"], 1);
    }
    let output = serde_json::json!({"schema":1,"producer":"full_panel_actual_generators_share_cid1_and_preserve_capital_gap_history",
        "stream_binding":binding,"cid1_h4cr_sha256":format!("{:x}", Sha256::digest(&cid1_record)),
        "a27_catalog_sha256":format!("{:x}", Sha256::digest(&catalog_bytes)),
        "d53_source_sha256":format!("{:x}", Sha256::digest(&d53_bytes)),"stages":stages});
    let bytes = serde_json::to_vec_pretty(&output).unwrap();
    assert!(bytes.len() <= 4 * 1024 * 1024);
    if let Some(path) = std::env::var_os("BATTLE_PLAN_FULL_PANEL_JOINT_OUTPUT").or_else(|| std::env::var_os("HOI4_FULL_PANEL_JOINT_OUTPUT")) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
        file.write_all(&bytes).unwrap(); file.sync_all().unwrap();
    }
}
