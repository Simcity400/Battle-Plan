"""Offline receipt checks for the bounded automatic capture driver.

This never opens a process. Journal/hash consistency is not authentication of
the collector protocol; retain the original worker proofs for that authority.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import stat

import capture_scorecard


def identity(path):
    path = Path(path)
    for part in (path, *path.parents):
        info = part.lstat()
        if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
            raise ValueError('reparse point in artifact path: ' + str(part))
    digest = hashlib.sha256()
    size = 0
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
            size += len(block)
    return {'path': str(path.resolve()), 'bytes': size, 'sha256': digest.hexdigest()}


def bounded_bytes(path, limit):
    with Path(path).open('rb') as stream:
        raw = stream.read(limit + 1)
    if len(raw) > limit:
        raise ValueError('receipt exceeds read limit: ' + str(path))
    return raw


def check_prefix(root):
    rows = read_journal(bounded_bytes(root / 'JOURNAL_BEFORE.jsonl', 128 * 1024**2))
    check_anchors(bounded_bytes(root / 'ANCHOR_BEFORE.jsonl', 128 * 1024**2), rows)
    latest = {key(r): r for r in rows}
    if any(r['state_to'] not in ('Complete', 'Failed', 'ProcessExitedBeforeAttach') for r in latest.values()):
        raise ValueError('prior journal has unfinished sessions')
    return len(rows)


def member_path(root, relative):
    if not isinstance(relative, str) or not relative or len(relative.encode()) > 1024 or '\\' in relative or ':' in relative:
        raise ValueError('unsafe artifact relative path')
    if any(p in ('', '.', '..') for p in relative.split('/')):
        raise ValueError('unsafe artifact relative path')
    path = root / relative
    if not path.resolve().is_relative_to(root.resolve()):
        raise ValueError('artifact escapes run directory')
    return path


def replay_references(run, row, artifact):
    """Inventory producer-declared packages; product container admission remains separate."""
    replay = Path(row.get('replay_path', ''))
    if same_path(replay, run / 'logical-stream-package-set.json'):
        descriptor = json.loads(bounded_bytes(replay, 4 * 1024**2))
        if descriptor.get('schema') != 'cid1-rollover-package-set-v1':
            raise ValueError('unsupported package-set schema')
        refs = [identity(replay)]
        source = member_path(run, descriptor['source_descriptor'])
        source_pin = identity(source)
        if source_pin['sha256'] != descriptor['source_descriptor_sha256']:
            raise ValueError('package-set source descriptor hash differs')
        refs.append(source_pin)
        streams = descriptor['ordered_streams']
        if not isinstance(streams, list) or not 1 <= len(streams) <= 4096:
            raise ValueError('package-set has invalid stream count')
        first = descriptor['first_recording_logical_stream_id']
        if type(first) is not int or not 0 <= first < len(streams):
            raise ValueError('package-set lacks an actual recording stream')
        seen = set()
        for expected_id, stream in enumerate(streams):
            stream_id = stream['logical_stream_id']
            if type(stream_id) is not int or not 0 <= stream_id < 2**64 or stream_id in seen or stream.get('recovery') is not False:
                raise ValueError('duplicate/invalid/recovery logical stream')
            if stream_id != expected_id:
                raise ValueError('package-set logical stream ordering differs')
            seen.add(stream_id)
            transient = stream_id < first
            if stream.get('pre_world_transient') is not transient or stream.get('recording_admitted') is not (not transient):
                raise ValueError('package-set pre-world/recording boundary disagrees')
            raw_pin = identity(member_path(run, stream['raw_relative_path']))
            if raw_pin['sha256'] != stream['raw_sha256'] or raw_pin['bytes'] != stream['raw_bytes']:
                raise ValueError('package-set raw stream identity differs')
            refs.append(dict(raw_pin, logical_stream_id=stream_id, pre_world_transient=transient))
            if transient:
                if any(stream.get(field, 'missing') is not None for field in ['manifest_relative_path', 'manifest_sha256', 'package_relative_path', 'package_sha256']):
                    raise ValueError('pre-world stream incorrectly declares a package')
                continue
            for field, hash_field in [('manifest_relative_path', 'manifest_sha256'), ('package_relative_path', 'package_sha256')]:
                path = member_path(run, stream[field])
                pin = identity(path)
                if pin['sha256'] != stream[hash_field]:
                    raise ValueError('package-set member hash differs: ' + field)
                refs.append(dict(pin, logical_stream_id=stream_id))
            package = member_path(run, stream['package_relative_path'])
            manifest = json.loads(bounded_bytes(member_path(run, stream['manifest_relative_path']), 8 * 1024**2))
            publication = manifest.get('replay_artifact') or {}
            if package.suffix != '.battleplan' or publication.get('written_file') != package.name or publication.get('pack_failed') or publication.get('recovery'):
                raise ValueError('package-set member manifest does not bind normal publication')
        return refs
    expected = member_path(run, artifact.get('written_file'))
    if not same_path(replay, expected) or replay.suffix != '.battleplan':
        raise ValueError('coordinator replay does not match this attempt publication')
    return [identity(replay)]


def read_journal(raw):
    """Verify the producer's v2 sequence and canonical hash chain, including history."""
    if raw and not raw.endswith(b'\n'):
        raise ValueError('journal has an incomplete final record')
    previous = '0' * 64
    records = []
    for line in raw.splitlines():
        row = json.loads(line)
        if row.get('schema') != 'auto_attach_journal.v2':
            raise ValueError('unsupported automatic journal schema')
        if json.dumps(row, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode() != line:
            raise ValueError('journal is not canonical JSON')
        if type(row.get('event_seq')) is not int or row['event_seq'] != len(records):
            raise ValueError('journal sequence is missing, repeated or out of order')
        if row.get('prev_record_hash') != previous:
            raise ValueError('journal previous hash mismatch')
        claimed = row.get('record_hash')
        canonical = {k: v for k, v in row.items() if k not in ('record_hash', 'prev_record_hash')}
        encoded = json.dumps(canonical, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()
        actual = hashlib.sha256(previous.encode() + encoded).hexdigest()
        if claimed != actual:
            raise ValueError('journal record hash mismatch')
        records.append(row)
        previous = actual
    return records


def check_anchors(raw, rows):
    if raw and not raw.endswith(b'\n'):
        raise ValueError('journal anchor has an incomplete final record')
    anchors = [json.loads(line) for line in raw.splitlines()]
    if len(anchors) != len(rows):
        raise ValueError('journal/anchor record counts differ')
    for anchor, row in zip(anchors, rows):
        if anchor != {'schema': 'auto_attach_journal_anchor.v1',
                      'event_seq': row['event_seq'], 'head_hash': row['record_hash']}:
            raise ValueError('journal/anchor custody mismatch')


def key(row):
    pid = row.get('pid')
    creation = row.get('creation_filetime', row.get('process_creation_filetime'))
    if isinstance(creation, str) and re.fullmatch(r'[1-9][0-9]*', creation):
        creation = int(creation)
    if type(pid) is not int or not 0 < pid <= 0xffffffff:
        raise ValueError('invalid process PID')
    if type(creation) is not int or not 0 < creation <= 0xffffffffffffffff:
        raise ValueError('invalid process creation FILETIME')
    return pid, creation


def same_path(left, right):
    return str(left).replace('\\', '/').rstrip('/').casefold() == str(right).replace('\\', '/').rstrip('/').casefold()


def assess_observations(observed, rows):
    issues = []
    scenario = observed['scenario']
    count = 2 if scenario == 'simultaneous' else 1
    if scenario not in ('app-first', 'game-first', 'simultaneous'):
        raise ValueError('unknown scenario')
    recorder = observed['recorder']
    recorder_key = key(recorder)
    initial = observed['initial_games']
    expected_initial = 0 if scenario == 'app-first' else count
    if len(initial) != expected_initial:
        issues.append('initial game census does not establish requested startup order')
    for game in initial:
        if key(game)[1] >= recorder_key[1]:
            issues.append('game-first creation time does not precede recorder')
    historical = observed['journal_prefix_records']
    if type(historical) is not int or not 0 <= historical <= len(rows):
        raise ValueError('invalid journal prefix record count')
    fresh = rows[historical:]
    if len({r['coordinator_instance_id'] for r in fresh}) != 1:
        issues.append('new journal does not belong to exactly one coordinator instance')
    old_instances = {r['coordinator_instance_id'] for r in rows[:historical]}
    if any(r['coordinator_instance_id'] in old_instances for r in fresh):
        issues.append('new attempt reused a historical coordinator identity')
    selected = {key(g) for g in observed['selected_games']}
    if len(selected) != count:
        issues.append('wrong number of distinct target process incarnations')
    if scenario != 'app-first' and selected != {key(g) for g in initial}:
        issues.append('initial target process incarnations were replaced')
    if scenario == 'app-first' and any(k[1] <= recorder_key[1] for k in selected):
        issues.append('app-first game creation did not follow recorder creation')
    for game in initial + observed['selected_games']:
        if game['user_sid'] != recorder['user_sid'] or game['session_id'] != recorder['session_id']:
            issues.append('game belongs to another user or Windows session')
        if not same_path(game['exe_path'], observed['game_exe']):
            issues.append('game executable path differs from explicit selection')
    if {key(r) for r in fresh} != selected:
        issues.append('journal contains missing or unexpected target incarnations')
    attempts, sessions = {}, {}
    for row in fresh:
        target = key(row)
        for name, mapping in [('attempt_id', attempts), ('session_id', sessions)]:
            value = row.get(name)
            if value is not None:
                mapping.setdefault(target, set()).add(value)
        if row.get('process_session_id') != recorder['session_id']:
            issues.append('journal Windows session does not match observed recorder')
        if not same_path(row.get('exe_path', ''), observed['game_exe']):
            issues.append('journal image path differs from observed game')
    for name, mapping in [('attempt', attempts), ('recording session', sessions)]:
        if set(mapping) != selected or any(len(v) != 1 for v in mapping.values()):
            issues.append(f'{name} identity is missing or changed within a scenario')
        values = [next(iter(v)) for v in mapping.values() if len(v) == 1]
        if len(values) != len(set(values)):
            issues.append(f'{name} identity is shared across target processes')
    start = observed.get('capture_window_started_ms')
    end = observed.get('capture_window_ended_ms')
    if type(start) is not int or type(end) is not int or end - start < 60000:
        issues.append('no observed 60-second capture window')
    else:
        samples = [s for s in observed['samples'] if start <= s['elapsed_ms'] <= end]
        if not samples or samples[0]['elapsed_ms'] != start or samples[-1]['elapsed_ms'] != end:
            issues.append('capture window boundaries lack observations')
        previous_time, previous_sequence = -1, historical
        for sample in observed['samples']:
            elapsed, seq = sample['elapsed_ms'], sample['journal_records']
            if type(elapsed) is not int or elapsed <= previous_time or not previous_sequence <= seq <= len(rows):
                issues.append('sample time or journal sequence regressed')
            previous_time, previous_sequence = elapsed, seq
        for sample in samples:
            latest = {key(r): r['state_to'] for r in rows[historical:sample['journal_records']]}
            if {key(g) for g in sample['games']} != selected or any(latest.get(k) != 'Recording' for k in selected):
                issues.append('targets were not jointly alive and Recording at every capture sample')
    terminal = {key(r): r for r in fresh}
    if any(r.get('state_to') != 'Complete' or r.get('artifact_status') != 'verified' for r in terminal.values()):
        issues.append('coordinator did not verify every target recording')
    if observed.get('recorder_exit_code') != 0:
        issues.append('owned recorder did not exit successfully after graceful stop')
    if observed.get('stop_requested') is not True:
        issues.append('owned recorder graceful stop was not requested')
    if observed.get('runtime_error'):
        issues.append(observed['runtime_error'])
    return sorted(set(issues)), terminal


def audit_run(root, row, game_exe, user_data):
    """Check producer receipt consistency. This does not independently authenticate IPC."""
    issues, refs = [], []
    attempt = row.get('attempt_id', '')
    if not re.fullmatch(r'[A-Za-z0-9_-]+', attempt):
        raise ValueError('unsafe or absent journal attempt id')
    run = root / ('run_' + attempt)
    if run.is_symlink() or not run.resolve().is_relative_to(root.resolve()):
        raise ValueError('run directory escapes explicit product root')
    manifest, _, pin = capture_scorecard.load_manifest(str(run))
    refs.append(pin)
    if manifest.get('pid') != row['pid'] or manifest.get('session_id') != row.get('session_id') or manifest.get('capture_id') != attempt:
        issues.append('manifest does not bind the journal process/attempt/session')
    modset = manifest.get('modset') or {}
    if modset.get('resolved') is not True or modset.get('enabled_mods') != []:
        issues.append('manifest does not establish resolved vanilla content')
    if not same_path(modset.get('pdx_user_dir', ''), user_data) or not same_path((modset.get('game') or {}).get('exe_path', ''), game_exe):
        issues.append('recorded game/profile selection differs from explicit inputs')
    campaign = manifest.get('campaign_identity') or {}
    raw_id, seed = campaign.get('game_unique_id'), campaign.get('game_unique_seed')
    if campaign.get('state') != 'known' or not isinstance(raw_id, str) or not raw_id or not isinstance(seed, str) or not re.fullmatch(r'-?(0|[1-9][0-9]*)', seed) or not -(2**63) <= int(seed) < 2**63:
        issues.append('raw campaign id and signed seed are not known')
    artifact = manifest.get('replay_artifact') or {}
    if artifact.get('pack_failed') or artifact.get('recovery'):
        issues.append('manifest has no normal published replay')
    refs.extend(replay_references(run, row, artifact))
    for name in ['worker_status.h4ct', 'HOOK_INSTALL_OUTCOMES_V1.json', 'HOOK_RESTORATION_OUTCOMES_V1.json']:
        path = run / name
        if not path.is_file():
            issues.append(f'missing {name}')
        else:
            refs.append(identity(path))
    restore = run / 'HOOK_RESTORATION_OUTCOMES_V1.json'
    if restore.is_file():
        data = json.loads(restore.read_bytes())
        if data.get('schema') != 'hoi4-hook-restoration-outcomes-v1' or data.get('outcome') != 'complete' or data.get('collector_module_disposition') != 'unloaded':
            issues.append('producer does not report complete hook restoration and unload')
        baseline = data.get('pinned_hook_site_baseline_sha256', '')
        if not re.fullmatch('[0-9a-f]{64}', baseline) or data.get('pinned_hook_site_observed_sha256') != baseline:
            issues.append('producer hook baseline and restored bytes disagree')
        sites = data.get('sites')
        if not isinstance(sites, list) or len(sites) != data.get('pinned_hook_site_count') or not sites or any(s.get('status') != 'restored' for s in sites):
            issues.append('producer hook restoration roster is incomplete')
        slots = data.get('import_slots')
        if not isinstance(slots, list) or len(slots) != data.get('import_slot_count') or any(s.get('status') != 'restored' for s in slots) or data.get('unrestored_hook_site_count') != 0 or data.get('unrestored_import_slot_count') != 0:
            issues.append('producer reports incomplete hook/import restoration')
    axes = capture_scorecard.score(manifest)
    return {'run_dir': str(run), 'issues': issues, 'campaign_identity': campaign,
            'references': refs, 'capture_complete': not any(n for _, n, _ in axes),
            'capture_axes': [{'axis': a, 'count': n, 'detail': d} for a, n, d in axes],
            'authority': 'producer_receipt_consistency_only; worker protocol not independently authenticated'}


def assess(root):
    observed = json.loads((root / 'OBSERVATIONS.json').read_bytes())
    product = Path(observed['product_data_root'])
    journal = bounded_bytes(product / 'auto_attach_journal.jsonl', 128 * 1024**2)
    before = (root / 'JOURNAL_BEFORE.jsonl').read_bytes()
    if not journal.startswith(before):
        raise ValueError('preexisting journal prefix was replaced')
    (root / 'JOURNAL_AFTER.jsonl').write_bytes(journal)
    rows = read_journal(journal)
    anchor = (product / 'auto_attach_journal.jsonl.anchor').read_bytes()
    anchor_before = (root / 'ANCHOR_BEFORE.jsonl').read_bytes()
    if not anchor.startswith(anchor_before):
        raise ValueError('preexisting anchor prefix was replaced')
    check_anchors(anchor, rows)
    (root / 'ANCHOR_AFTER.jsonl').write_bytes(anchor)
    issues, terminal = assess_observations(observed, rows)
    if check_prefix(root) != observed['journal_prefix_records']:
        issues.append('observed prefix count differs from retained journal')
    admission = json.loads((root / 'ARTIFACT_ADMISSION.json').read_bytes())
    retained = identity(root / 'CANDIDATE_ATTESTATION.json')
    if retained['sha256'] != admission['receipt_sha256']:
        issues.append('retained candidate attestation differs from admission')
    for role, path in admission['artifacts'].items():
        if identity(path)['sha256'] != admission['artifact_sha256'][role]:
            issues.append('candidate artifact changed during scenario: ' + role)
    if not same_path(observed['recorder']['exe_path'], admission['artifacts']['recorder_host']):
        issues.append('observed recorder differs from admitted host')
    for pin in observed['preserved_files']:
        if identity(pin['path']) != pin:
            issues.append('preexisting artifact changed: ' + pin['path'])
    runs = []
    for row in terminal.values():
        try:
            run = audit_run(product, row, observed['game_exe'], observed['user_data'])
        except (OSError, ValueError, SystemExit, TypeError, KeyError) as error:
            run = {'issues': [str(error)], 'attempt_id': row.get('attempt_id')}
        runs.append(run)
    return {'schema': 'hoi4.automatic_capture_scenario.v1',
            'scenario': observed['scenario'], 'scenario_observed': not issues,
            'issues': issues, 'runs': runs,
            'producer_receipts_consistent': bool(runs) and not any(r['issues'] for r in runs),
            'qualification': 'sampled automatic lifecycle evidence, not a release acceptance verdict',
            'max_sample_interval_ms': max((b['elapsed_ms'] - a['elapsed_ms'] for a, b in zip(observed['samples'], observed['samples'][1:])), default=None),
            'not_evaluated': ['independent_worker_protocol_authentication', 'container_and_content_admission', 'zero_loss_passive_speed5_capture', 'rendering', 'performance', 'clean_machine'],
            'references': [identity(root / name) for name in ['PLAN.json', 'OBSERVATIONS.json', 'JOURNAL_BEFORE.jsonl', 'JOURNAL_AFTER.jsonl', 'ANCHOR_BEFORE.jsonl', 'ANCHOR_AFTER.jsonl', 'ARTIFACT_ADMISSION.json', 'CANDIDATE_ATTESTATION.json']]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out-dir', type=Path, required=True)
    parser.add_argument('--check-prefix', action='store_true')
    args = parser.parse_args()
    if args.check_prefix:
        try:
            print(json.dumps({'prefix_records': check_prefix(args.out_dir)}))
            return 0
        except Exception as error:
            print(json.dumps({'error': str(error)}))
            return 1
    try:
        result = assess(args.out_dir)
    except Exception as error:
        result = {'schema': 'hoi4.automatic_capture_scenario.v1', 'scenario_observed': False,
                  'producer_receipts_consistent': False, 'issues': [str(error)]}
    with (args.out_dir / 'SCENARIO_RECEIPT.json').open('x', encoding='utf-8') as output:
        json.dump(result, output, indent=2)
        output.write('\n')
    return 0 if result['scenario_observed'] and result['producer_receipts_consistent'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
