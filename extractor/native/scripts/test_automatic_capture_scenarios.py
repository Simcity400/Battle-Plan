"""Offline scenario refusals and safe Plan/owned-process boundary checks."""
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import automatic_capture_scenarios as scenario

SCRIPTS = Path(__file__).resolve().parent
ROOT = SCRIPTS.parents[2]


def encode(rows):
    previous = '0' * 64
    result = []
    for seq, original in enumerate(rows):
        row = dict(original, event_seq=seq)
        row.pop('record_hash', None)
        row.pop('prev_record_hash', None)
        payload = json.dumps(row, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()
        row['record_hash'] = hashlib.sha256(previous.encode() + payload).hexdigest()
        row['prev_record_hash'] = previous
        previous = row['record_hash']
        result.append(json.dumps(row, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode())
    return b'\n'.join(result) + (b'\n' if result else b'')


def fixture(mode='game-first'):
    count = 2 if mode == 'simultaneous' else 1
    games = [{'pid': 10+i, 'creation_filetime': str((300 if mode == 'app-first' else 100)+i),
              'session_id': 3, 'user_sid': 'owner', 'exe_path': 'C:/game/hoi4.exe'} for i in range(count)]
    recorder = dict(games[0], pid=90, creation_filetime='200', exe_path='C:/release/collector_record.exe')
    rows = []
    for state in ['Recording', 'Complete']:
        for game in games:
            rows.append({'schema': 'auto_attach_journal.v2', 'pid': game['pid'],
                         'process_creation_filetime': int(game['creation_filetime']),
                         'process_session_id': 3, 'exe_path': game['exe_path'],
                         'coordinator_instance_id': 'fresh', 'state_to': state,
                         'attempt_id': 'attempt-'+str(game['pid']),
                         'session_id': 'session-'+str(game['pid']),
                         'artifact_status': 'verified' if state == 'Complete' else 'pending'})
    rows = scenario.read_journal(encode(rows))
    observed = {'scenario': mode, 'recorder': recorder, 'initial_games': [] if mode == 'app-first' else games,
                'selected_games': games, 'game_exe': games[0]['exe_path'], 'journal_prefix_records': 0,
                'capture_window_started_ms': 0, 'capture_window_ended_ms': 60000,
                'samples': [{'elapsed_ms': ms, 'games': copy.deepcopy(games), 'journal_records': count} for ms in [0, 30000, 60000]],
                'stop_requested': True, 'recorder_exit_code': 0, 'runtime_error': None}
    return copy.deepcopy(observed), rows


class JournalTests(unittest.TestCase):
    def test_prefix_refuses_unfinished_history_and_truncated_anchor(self):
        _, rows = fixture()
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            def save(selected):
                (root/'JOURNAL_BEFORE.jsonl').write_bytes(encode(selected))
                checked = scenario.read_journal(encode(selected))
                (root/'ANCHOR_BEFORE.jsonl').write_bytes(b''.join(json.dumps({'schema': 'auto_attach_journal_anchor.v1', 'event_seq': r['event_seq'], 'head_hash': r['record_hash']}).encode()+b'\n' for r in checked))
            save(rows)
            self.assertEqual(scenario.check_prefix(root), 2)
            save(rows[:1])
            with self.assertRaisesRegex(ValueError, 'unfinished'):
                scenario.check_prefix(root)
            save(rows)
            (root/'ANCHOR_BEFORE.jsonl').write_bytes(b'')
            with self.assertRaisesRegex(ValueError, 'counts'):
                scenario.check_prefix(root)

    def test_producer_sequence_begins_at_zero_and_roundtrips(self):
        _, rows = fixture()
        self.assertEqual(rows[0]['event_seq'], 0)
        self.assertEqual(scenario.read_journal(encode(rows)), rows)

    def test_truncated_or_modified_chain_is_refused(self):
        _, rows = fixture()
        raw = encode(rows)
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            scenario.read_journal(raw[:-1])
        with self.assertRaisesRegex(ValueError, 'hash'):
            scenario.read_journal(raw.replace(b'Recording', b'Finalized'))

    def test_noncanonical_or_duplicate_json_is_refused(self):
        _, rows = fixture()
        with self.assertRaisesRegex(ValueError, 'canonical'):
            scenario.read_journal(json.dumps(rows[0]).encode() + b'\n')
        raw = encode(rows).replace(b'"pid":10', b'"pid":10,"pid":10', 1)
        with self.assertRaisesRegex(ValueError, 'canonical'):
            scenario.read_journal(raw)

    def test_anchor_mismatch_and_truncation_cannot_hide_lost_records(self):
        _, rows = fixture()
        anchors = [{'schema': 'auto_attach_journal_anchor.v1', 'event_seq': r['event_seq'], 'head_hash': r['record_hash']} for r in rows]
        raw = b''.join(json.dumps(a).encode()+b'\n' for a in anchors)
        scenario.check_anchors(raw, rows)
        with self.assertRaises(ValueError):
            scenario.check_anchors(raw, rows[:-1])
        with self.assertRaises(ValueError):
            scenario.check_anchors(raw[:-1], rows)


class ObservationTests(unittest.TestCase):
    def test_each_actual_startup_order_and_overlap_shape_is_accepted(self):
        for mode in ['app-first', 'game-first', 'simultaneous']:
            with self.subTest(mode=mode):
                observed, rows = fixture(mode)
                self.assertEqual(scenario.assess_observations(observed, rows)[0], [])

    def test_existing_game_cannot_be_relabelled_app_first(self):
        observed, rows = fixture('game-first')
        observed['scenario'] = 'app-first'
        issues, _ = scenario.assess_observations(observed, rows)
        self.assertTrue(any('startup order' in x for x in issues))
        self.assertTrue(any('creation' in x for x in issues))

    def test_same_pid_new_creation_is_a_different_game(self):
        observed, rows = fixture()
        observed['samples'][1]['games'][0]['creation_filetime'] = '101'
        self.assertTrue(scenario.assess_observations(observed, rows)[0])

    def test_another_user_or_windows_session_is_refused(self):
        for field, value in [('user_sid', 'another'), ('session_id', 7)]:
            observed, rows = fixture()
            observed['selected_games'][0][field] = value
            self.assertIn('game belongs to another user or Windows session', scenario.assess_observations(observed, rows)[0])

    def test_simultaneous_requires_isolated_sessions_and_attempts(self):
        for field in ['session_id', 'attempt_id']:
            observed, rows = fixture('simultaneous')
            for row in rows:
                row[field] = 'shared'
            self.assertTrue(any('shared' in x for x in scenario.assess_observations(observed, rows)[0]))

    def test_simultaneous_requires_joint_recording_not_sequential_runs(self):
        observed, rows = fixture('simultaneous')
        observed['samples'][1]['journal_records'] = 3
        observed['samples'][2]['journal_records'] = 3
        self.assertTrue(any('jointly' in x for x in scenario.assess_observations(observed, rows)[0]))

    def test_waiting_for_world_is_not_campaign_readiness(self):
        observed, rows = fixture()
        rows[0]['state_to'] = 'WaitingForWorld'
        self.assertTrue(any('jointly' in x for x in scenario.assess_observations(observed, rows)[0]))

    def test_short_window_or_missing_stop_or_finalize_never_passes(self):
        for field, value in [('capture_window_ended_ms', 59000), ('stop_requested', False), ('recorder_exit_code', None)]:
            observed, rows = fixture()
            observed[field] = value
            self.assertTrue(scenario.assess_observations(observed, rows)[0])

    def test_extra_target_or_reused_historical_coordinator_is_refused(self):
        observed, rows = fixture()
        observed['journal_prefix_records'] = 1
        self.assertTrue(any('historical' in x for x in scenario.assess_observations(observed, rows)[0]))
        observed, rows = fixture()
        rows.append(dict(rows[0], pid=15))
        self.assertTrue(any('unexpected' in x for x in scenario.assess_observations(observed, rows)[0]))

    def test_failed_container_or_worker_is_not_complete(self):
        observed, rows = fixture()
        rows[-1]['state_to'] = 'Failed'
        rows[-1]['artifact_status'] = 'failed'
        self.assertTrue(any('verify' in x for x in scenario.assess_observations(observed, rows)[0]))


class ArtifactTests(unittest.TestCase):
    def test_full_offline_receipt_retains_gaps_and_refuses_changed_candidate(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            product = root/'product'
            product.mkdir()
            run = product/'run_attempt-10'
            run.mkdir()
            observed, rows = fixture()
            host = root/'collector_record.exe'
            host.write_bytes(b'candidate fixture; not an attested PE')
            observed['recorder']['exe_path'] = str(host)
            observed.update(product_data_root=str(product), user_data='C:/explicit/profile', preserved_files=[])
            replay = run/'native_session.battleplan'
            replay.write_bytes(b'package fixture; never admitted content')
            rows[-1]['replay_path'] = str(replay)
            raw = encode(rows)
            rows = scenario.read_journal(raw)
            (product/'auto_attach_journal.jsonl').write_bytes(raw)
            anchors = b''.join(json.dumps({'schema': 'auto_attach_journal_anchor.v1', 'event_seq': r['event_seq'], 'head_hash': r['record_hash']}).encode()+b'\n' for r in rows)
            (product/'auto_attach_journal.jsonl.anchor').write_bytes(anchors)
            for name in ['JOURNAL_BEFORE.jsonl', 'ANCHOR_BEFORE.jsonl']:
                (root/name).write_bytes(b'')
            (root/'OBSERVATIONS.json').write_text(json.dumps(observed))
            (root/'PLAN.json').write_text('{}')
            attestation = root/'CANDIDATE_ATTESTATION.json'
            attestation.write_bytes(b'explicit fixture, no central release authority')
            admission = {'receipt_sha256': scenario.identity(attestation)['sha256'], 'artifacts': {'recorder_host': str(host)}, 'artifact_sha256': {'recorder_host': scenario.identity(host)['sha256']}}
            (root/'ARTIFACT_ADMISSION.json').write_text(json.dumps(admission))
            manifest = {'pid': 10, 'session_id': 'session-10', 'capture_id': 'attempt-10',
                        'modset': {'resolved': True, 'enabled_mods': [], 'pdx_user_dir': observed['user_data'], 'game': {'exe_path': observed['game_exe']}},
                        'campaign_identity': {'state': 'known', 'game_unique_id': 'a4b17fed-ed12-4361-9064-fb6576638110', 'game_unique_seed': '-1'},
                        'replay_artifact': {'written_file': replay.name, 'recovery': False}}
            (run/'native_session_manifest.json').write_text(json.dumps(manifest))
            (run/'worker_status.h4ct').write_bytes(b'fixture; not authenticated IPC')
            (run/'HOOK_INSTALL_OUTCOMES_V1.json').write_text('{}')
            restore = {'schema': 'hoi4-hook-restoration-outcomes-v1', 'outcome': 'complete', 'collector_module_disposition': 'unloaded',
                       'pinned_hook_site_baseline_sha256': 'a'*64, 'pinned_hook_site_observed_sha256': 'a'*64,
                       'sites': [{'status': 'restored'}], 'pinned_hook_site_count': 1, 'unrestored_hook_site_count': 0,
                       'import_slots': [], 'import_slot_count': 0, 'unrestored_import_slot_count': 0}
            (run/'HOOK_RESTORATION_OUTCOMES_V1.json').write_text(json.dumps(restore))
            result = scenario.assess(root)
            self.assertTrue(result['scenario_observed'], result)
            self.assertTrue(result['producer_receipts_consistent'], result)
            self.assertFalse(result['runs'][0]['capture_complete'])
            self.assertIn('container_and_content_admission', result['not_evaluated'])
            self.assertIn('independent_worker_protocol_authentication', result['not_evaluated'])
            host.write_bytes(b'changed candidate')
            result = scenario.assess(root)
            self.assertFalse(result['scenario_observed'])
            self.assertIn('candidate artifact changed during scenario: recorder_host', result['issues'])

    def test_journal_cannot_name_another_attempt_recording(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            replay = root/'native_session.battleplan'
            replay.write_bytes(b'retained package bytes; not a container admission fixture')
            refs = scenario.replay_references(root, {'replay_path': str(replay)}, {'written_file': replay.name})
            self.assertEqual(refs, [scenario.identity(replay)])
            with self.assertRaisesRegex(ValueError, 'does not match'):
                scenario.replay_references(root, {'replay_path': str(root.parent/replay.name)}, {'written_file': replay.name})
            for relative in ['../old.battleplan', '/old.battleplan', 'C:/old.battleplan', 'a\\old.battleplan']:
                with self.assertRaises(ValueError):
                    scenario.member_path(root, relative)

    def test_package_set_retains_declared_stream_bytes_without_container_admission(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            source = root/'logical-streams/logical-stream-set.json'
            source.parent.mkdir()
            source.write_bytes(b'{}')
            replay = root/'logical-streams/packages/00000000000000000000/native_session.battleplan'
            replay.parent.mkdir(parents=True)
            replay.write_bytes(b'not an admitted container')
            manifest = replay.parent/'portable_session_manifest.json'
            manifest.write_text(json.dumps({'replay_artifact': {'written_file': replay.name, 'recovery': False}}))
            descriptor = root/'logical-stream-package-set.json'
            stream = {'logical_stream_id': 0, 'recovery': False,
                      'pre_world_transient': False, 'recording_admitted': True,
                      'raw_relative_path': source.relative_to(root).as_posix(), 'raw_bytes': 2, 'raw_sha256': scenario.identity(source)['sha256'],
                      'manifest_relative_path': manifest.relative_to(root).as_posix(), 'manifest_sha256': scenario.identity(manifest)['sha256'],
                      'package_relative_path': replay.relative_to(root).as_posix(), 'package_sha256': scenario.identity(replay)['sha256']}
            data = {'schema': 'cid1-rollover-package-set-v1', 'first_recording_logical_stream_id': 0, 'source_descriptor': source.relative_to(root).as_posix(),
                    'source_descriptor_sha256': scenario.identity(source)['sha256'], 'ordered_streams': [stream]}
            descriptor.write_text(json.dumps(data))
            row = {'replay_path': str(descriptor)}
            refs = scenario.replay_references(root, row, {})
            self.assertEqual(refs[-1]['logical_stream_id'], 0)
            self.assertEqual(refs[-1]['sha256'], scenario.identity(replay)['sha256'])
            replay.write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError, 'member hash'):
                scenario.replay_references(root, row, {})
            replay.write_bytes(b'not an admitted container')
            data['ordered_streams'].append(stream)
            descriptor.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, 'duplicate'):
                scenario.replay_references(root, row, {})
            transient = dict(stream, pre_world_transient=True, recording_admitted=False,
                             manifest_relative_path=None, manifest_sha256=None,
                             package_relative_path=None, package_sha256=None)
            data['ordered_streams'] = [transient, dict(stream, logical_stream_id=1)]
            data['first_recording_logical_stream_id'] = 1
            descriptor.write_text(json.dumps(data))
            refs = scenario.replay_references(root, row, {})
            self.assertTrue(refs[2]['pre_world_transient'])
            self.assertEqual(refs[-1]['logical_stream_id'], 1)
            transient['package_relative_path'] = replay.name
            descriptor.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, 'pre-world'):
                scenario.replay_references(root, row, {})


@unittest.skipUnless(os.name == 'nt', 'PowerShell/Windows process boundary')
class WindowsBoundaries(unittest.TestCase):
    def test_journal_poll_shares_producer_writer_and_ignores_partial_record(self):
        source = str(SCRIPTS/'Invoke-HeadlessRawCaptureUntilClean.ps1').replace("'", "''")
        with tempfile.TemporaryDirectory() as scratch:
            journal = str(Path(scratch)/'journal.jsonl').replace("'", "''")
            command = f"""$ErrorActionPreference='Stop'
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile('{source}',[ref]$tokens,[ref]$errors)
if ($errors.Count) {{ throw 'driver parse failed' }}
$fn=$ast.Find({{ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Read-ScenarioJournal' }},$true)
Invoke-Expression $fn.Extent.Text
$writer=[IO.File]::Open('{journal}',[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::Read)
try {{
  $bytes=[Text.Encoding]::UTF8.GetBytes("{{`"seq`":0}}`n{{`"partial`":")
  $writer.Write($bytes,0,$bytes.Length); $writer.WriteByte(0xc3); $writer.Flush()
  $rows=@(Read-ScenarioJournal '{journal}')
  if ($rows.Count -ne 1 -or $rows[0].seq -ne 0) {{ throw 'in-flight journal sample differs' }}
}} finally {{ $writer.Dispose() }}
Write-Output 'SHARED_JOURNAL_PASS'
"""
            result = subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', command], capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
            self.assertIn('SHARED_JOURNAL_PASS', result.stdout)

    def test_plan_with_missing_attestation_names_refusal_without_process_launch(self):
        with tempfile.TemporaryDirectory() as scratch:
            output = Path(scratch)/'fresh'
            argv = ['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                    str(SCRIPTS/'Invoke-HeadlessRawCaptureUntilClean.ps1'), '-Scenario', 'app-first',
                    '-RepoRoot', str(ROOT), '-ArtifactAttestationPath', str(Path(scratch)/'missing.json'),
                    '-GameExe', 'C:/not-selected/hoi4.exe', '-GameExeSha256', '0'*64,
                    '-UserData', 'C:/not-selected/profile', '-ProductDataRoot', 'C:/not-selected/product',
                    '-OutDir', str(output), '-ReadyTimeoutSeconds', '1', '-FinalizeTimeoutSeconds', '1']
            result = subprocess.run(argv, capture_output=True, timeout=30)
            self.assertNotEqual(result.returncode, 0)
            plan = json.loads((output/'PLAN.json').read_bytes())
            self.assertEqual(plan['mode'], 'Plan')
            self.assertEqual(plan['status'], 'blocked')
            self.assertIn('attestation is missing', plan['error'])
            self.assertFalse((output/'RECORDER_OWNERSHIP.json').exists())
            preserved = (output/'PLAN.json').read_bytes()
            result = subprocess.run(argv, capture_output=True, timeout=30)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual((output/'PLAN.json').read_bytes(), preserved)

    def test_native_stop_boundary_rejects_wrong_creation_and_path_before_messaging(self):
        source = str(SCRIPTS/'AutomaticCapture.Process.ps1').replace("'", "''")
        command = f"""$ErrorActionPreference='Stop'
. '{source}'
Initialize-ScenarioProcessAccess
$owned=[AutomaticScenarioProcess]::Read($PID)
if ($owned.user_sid -cne [Security.Principal.WindowsIdentity]::GetCurrent().User.Value) {{ throw 'owner SID mismatch' }}
$refused=0
try {{ [void][AutomaticScenarioProcess]::RequestRecorderStop($PID,'1',$owned.exe_path) }} catch {{ if ($_.Exception.Message -match 'ownership changed') {{ $refused++ }} else {{ throw }} }}
try {{ [void][AutomaticScenarioProcess]::RequestRecorderStop($PID,$owned.creation_filetime,'C:\\different.exe') }} catch {{ if ($_.Exception.Message -match 'ownership changed') {{ $refused++ }} else {{ throw }} }}
if($refused -ne 2) {{ throw 'ownership guard did not refuse' }}
Write-Output 'OWNERSHIP_BOUNDARY_PASS'
"""
        result = subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', command], capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        self.assertIn('OWNERSHIP_BOUNDARY_PASS', result.stdout)


if __name__ == '__main__':
    unittest.main()
