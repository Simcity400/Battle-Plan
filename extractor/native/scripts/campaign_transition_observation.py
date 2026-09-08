"""Passive transition observer's offline evidence checks and product CLI runner.

Only Invoke-CampaignTransitionObservation.ps1 acquires live observations. This
module never opens the game, synthesizes observations, stitches files, or promotes a
capture receipt. Product admission owns container/fold validation; ZIP inspection
below only selects evidence for exact comparisons after that admission.
"""
import argparse
import ctypes
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import time
import zipfile
import zlib

import automatic_capture_scenarios as startup
import capture_scorecard
import gap_ledger

LIMIT = 128 * 1024**2
SCHEMA = 'hoi4.campaign_transition_observation.v1'
SEGMENT_FIELDS = ('segment_id', 'parent_segment', 'cutover_hour', 'head_hour',
                  'start_reason', 'stop_reason', 'visible_spans', 'superseded_spans')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def strict_pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, 'duplicate JSON key: ' + key)
        result[key] = value
    return result


def decode(raw):
    return json.loads(raw, object_pairs_hook=strict_pairs,
                      parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))


def read(path):
    startup.identity(path)  # reject reparse ancestors before reading
    return decode(startup.bounded_bytes(path, LIMIT))


def write(path, value):
    with Path(path).open('x', encoding='utf-8', newline='\n') as out:
        json.dump(value, out, indent=2, ensure_ascii=False, allow_nan=False)
        out.write('\n')
        out.flush()
        os.fsync(out.fileno())


def retain(source, destination):
    before = startup.identity(source)
    with Path(source).open('rb') as src, Path(destination).open('xb') as dst:
        for block in iter(lambda: src.read(1024**2), b''):
            dst.write(block)
        dst.flush()
        os.fsync(dst.fileno())
    after = startup.identity(destination)
    require((before['bytes'], before['sha256']) == (after['bytes'], after['sha256'])
            and startup.identity(source) == before, 'input changed while retaining: ' + str(source))
    return dict(after, source_path=str(source))


def exact(actual, expected, label):
    # bools and floats must not compare equal to integer ticks/identifiers.
    require(json.dumps(actual, sort_keys=True) == json.dumps(expected, sort_keys=True),
            'exact expectation differs: ' + label)


def integer(value, label, minimum=-(2**63), maximum=2**63-1):
    require(type(value) is int and minimum <= value <= maximum, 'invalid ' + label)
    return value


def campaign(value, producer=False):
    raw, seed = value.get('game_unique_id'), value.get('game_unique_seed')
    require(isinstance(raw, str) and re.fullmatch(
        '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}', raw),
        'raw game_unique_id must be a canonical UUID; no header/name fallback')
    require(isinstance(seed, str) and re.fullmatch('0|-?[1-9][0-9]*', seed)
            and -(2**31) <= int(seed) < 2**31, 'missing or invalid signed game_unique_seed')
    preimage = b'hoi4-campaign-id-v1\0' + raw.encode() + b'\0' + seed.encode()
    key = hashlib.sha256(preimage).digest()
    if producer:
        require(value.get('schema') == 'hoi4.campaign_identity.v1'
                and value.get('state') == 'known' and value.get('source') == 'live_memory',
                'campaign lacks producer known live-memory identity')
        exact(value.get('campaign_id'), key.hex(), 'derived campaign_id')
    return key


def contract(value):
    require(value.get('schema') == 'hoi4.campaign_transition_expectations.v1', 'expectation schema')
    stages, recordings, transitions = value['checkpoints'], value['recordings'], value['transitions']
    require(isinstance(stages, list) and 2 <= len(stages) <= 64, 'need 2..64 checkpoints')
    require(isinstance(recordings, list) and 1 <= len(recordings) <= 64, 'need scoped recordings')
    names = [s['name'] for s in stages]
    rec_names = [r['name'] for r in recordings]
    for group in (names, rec_names):
        require(all(isinstance(n, str) and re.fullmatch('[A-Za-z0-9_-]{1,64}', n) for n in group)
                and len({n.casefold() for n in group}) == len(group), 'unique safe names required')
    require(len(transitions) == len(stages)-1, 'exact adjacent transition list required')
    for stage in stages:
        require(stage['recording'] in rec_names, 'unknown recording selection')
        integer(stage['segment_id'], 'segment id', 0, 2**32-1)
        integer(stage['logical_stream_id'], 'logical stream id', 0, 2**64-1)
        campaign(stage['campaign'])
        require(Path(stage['scope_file']).is_absolute(), 'scope_file must be absolute')
    by_campaign, by_recording = {}, {}
    for stage in stages:
        key = campaign(stage['campaign'])
        previous = by_campaign.setdefault(key, stage['recording'])
        require(previous == stage['recording'], 'one campaign cannot split across logical recordings')
        previous_key = by_recording.setdefault(stage['recording'], key)
        require(previous_key == key, 'different campaigns cannot share one logical recording')
    require(len({s['scope_file'].casefold() for s in stages}) == len(stages), 'reused checkpoint file')
    for number, transition in enumerate(transitions):
        exact([transition['from'], transition['to']], names[number:number+2], 'transition order')
        require(transition['kind'] in ('reload', 'relaunch', 'rollback', 'identity-change'), 'unknown transition kind')
        require(transition['attachment'] in ('same', 'replaced'), 'explicit attachment relation required')
        a, b = stages[number:number+2]
        changed = transition['kind'] == 'identity-change'
        require((campaign(a['campaign']) != campaign(b['campaign'])) == changed, 'campaign relation contradicts transition')
        require((a['recording'] != b['recording']) == changed, 'same campaign must select one logical recording')
        if changed and transition['attachment'] == 'same':
            require(a['campaign']['game_unique_id'] != b['campaign']['game_unique_id']
                    and a['campaign']['game_unique_seed'] != b['campaign']['game_unique_seed'],
                    'current producer refuses single-component identity changes after recorded activity')
        if not changed:
            require(b['segment_id'] > a['segment_id'], 'reload/relaunch requires a new retained segment')
        if transition['kind'] == 'relaunch':
            require(transition['attachment'] == 'replaced', 'relaunch requires replaced attachment')
        if transition['kind'] in ('reload', 'rollback'):
            require(transition['attachment'] == 'same', 'in-process reload/rollback requires same attachment')
    for recording in recordings:
        require(Path(recording['path']).is_absolute(), 'recording path must be absolute')
        require('logical_stream_id' in recording, 'explicit logical_stream_id (null for single file) required')
        if recording['logical_stream_id'] is not None:
            integer(recording['logical_stream_id'], 'logical stream', 0, 2**64-1)
        expected = recording['expected']
        for field in ('segments', 'main_slices', 'main_head', 'coverage', 'surviving_hours', 'playback_gaps'):
            require(field in expected, 'missing exact recording expectation: ' + field)
        require(expected['surviving_hours'], 'nonempty whole playback expected')
        for hour in expected['surviving_hours']:
            integer(hour, 'canonical hour')
        require(expected['surviving_hours'] == sorted(set(expected['surviving_hours'])), 'surviving hours order')
        for segment in expected['segments']:
            exact(sorted(segment), sorted(SEGMENT_FIELDS), 'segment expectation fields')
    require(set(rec_names) == {s['recording'] for s in stages}, 'unused recording expectation')
    return value


@contextmanager
def zip_stream(archive, info):
    """Metadata/extraction reader; product admission remains the ZIP authority."""
    if info.compress_type != 93:
        with archive.open(info) as stream:
            yield stream
    else:
        import zstandard
        # zipfile in Python 3.11 cannot decompress method 93. Its directory reader
        # locates the entry; product admission, not this reader, authenticates ZIP.
        with Path(archive.filename).open('rb') as stream:
            stream.seek(info.header_offset)
            header = stream.read(30)
            require(len(header) == 30 and header[:4] == b'PK\x03\x04', 'ZIP local header')
            name_len, extra_len = struct.unpack_from('<HH', header, 26)
            stream.seek(name_len + extra_len, 1)
            class BoundedReader:
                remaining = info.compress_size

                def read(self, size=-1):
                    size = self.remaining if size < 0 else min(size, self.remaining)
                    data = stream.read(size)
                    self.remaining -= len(data)
                    return data

            with zstandard.ZstdDecompressor().stream_reader(BoundedReader()) as decoded:
                yield decoded


def zip_bytes(archive, name):
    """Read bounded stored/deflated/Zstandard JSON, including production method 93."""
    info = archive.getinfo(name)
    require(info.file_size <= LIMIT and info.compress_size <= LIMIT, 'ZIP evidence entry too large')
    with zip_stream(archive, info) as stream:
        raw = stream.read(LIMIT+1)
    require(len(raw) == info.file_size and zlib.crc32(raw) == info.CRC, 'ZIP evidence size/CRC differs')
    return raw


def zip_json(archive, name):
    return decode(zip_bytes(archive, name))


def assert_pin(pin):
    actual = startup.identity(pin['path'])
    exact([actual['bytes'], actual['sha256']], [pin['bytes'], pin['sha256']], 'retained artifact changed: ' + pin['path'])


def assess_epoch(stage, scope, state, ticket):
    require(state.get('schema') == 'cid1-rollover-fs-v1' and state.get('phase') == 'streaming',
            'checkpoint lacks contemporaneous streaming rollover state')
    require(ticket.get('schema') == 'product-stitch-ticket-v1', 'checkpoint lacks producer append ticket')
    exact(state['logical_stream_id'], stage['logical_stream_id'], 'contemporaneous logical stream')
    exact(ticket['logical_stream_id'], stage['logical_stream_id'], 'ticket logical stream')
    exact(state['campaign_id_hex'], campaign(stage['campaign']).hex(), 'contemporaneous campaign')
    exact(ticket['campaign_key_hex'], state['campaign_id_hex'], 'ticket campaign')
    exact(ticket['identity_evidence_sha256'], state['cid1_payload_sha256_hex'], 'ticket CID1 epoch')
    require(re.fullmatch('[0-9a-f]{64}', ticket['identity_evidence_sha256']), 'invalid CID1 epoch digest')
    pid, creation = startup.key(scope)
    exact(ticket['process_start_identity'], f'pid:{pid}:creation-filetime:{creation}', 'ticket attachment')
    exact(ticket['session_nonce'], scope['session_id'], 'ticket session')


def admitted_member(root, product, stage, witness, viewer, viewer_hash, timeout):
    """Bind the assessed portable manifest to a producer-declared, admitted member."""
    name, scope = stage['name'], witness['scope']
    run = product/('run_'+scope['attempt_id'])
    descriptor_pin = retain(run/'logical-stream-package-set.json', root/(name+'.package-set.json'))
    descriptor = read(descriptor_pin['path'])
    require(descriptor.get('schema') == 'cid1-rollover-package-set-v1', 'missing producer package-set descriptor')
    source_path = startup.member_path(run, descriptor['source_descriptor'])
    source_pin = retain(source_path, root/(name+'.stream-set.json'))
    exact(source_pin['sha256'], descriptor['source_descriptor_sha256'], 'source descriptor custody')
    source = read(source_pin['path'])
    require(source.get('schema') == 'cid1-rollover-set-v1', 'raw source descriptor schema')
    exact(source['transitions'], descriptor['transitions'], 'published transition descriptor')
    selected = [m for m in descriptor['ordered_streams'] if m['logical_stream_id'] == stage['logical_stream_id']]
    require(len(selected) == 1, 'missing or duplicate producer member')
    member = selected[0]
    require(member.get('recording_admitted') is True and member.get('pre_world_transient') is False
            and member.get('recovery') is False, 'member is transient, recovery or unadmitted')
    raw = [m for m in source['ordered_streams'] if m['logical_stream_id'] == stage['logical_stream_id']]
    require(len(raw) == 1, 'missing or duplicate raw member descriptor')
    raw = raw[0]
    for published, original in [('campaign_id_hex','campaign_id_hex'), ('cid1_payload_sha256_hex','cid1_payload_sha256_hex'),
                                ('raw_sha256','file_sha256_hex'), ('raw_bytes','file_bytes')]:
        exact(member[published], raw[original], 'member/raw descriptor ' + published)
    exact(member['campaign_id_hex'], witness['ticket']['campaign_key_hex'], 'member campaign at checkpoint')
    exact(member['cid1_payload_sha256_hex'], witness['ticket']['identity_evidence_sha256'], 'member epoch at checkpoint')
    manifest_path = startup.member_path(run, member['manifest_relative_path'])
    require(startup.same_path(manifest_path, scope['manifest_path']), 'selected manifest is not the producer-declared member')
    manifest_pin = retain(manifest_path, root/(name+'.manifest.json'))
    exact(manifest_pin['sha256'], member['manifest_sha256'], 'published manifest hash')
    package_pin = retain(startup.member_path(run, member['package_relative_path']), root/(name+'.member.battleplan'))
    exact(package_pin['sha256'], member['package_sha256'], 'published member package hash')
    product_command(viewer, ['--validate-recording', package_pin['path'], '--non-interactive'], root,
                    name+'.member-admission', timeout, viewer_hash)
    assert_pin(package_pin)
    with zipfile.ZipFile(package_pin['path']) as archive:
        package_manifest = zip_json(archive, 'manifest.json')
        entries = [e for e in package_manifest['entries'] if e.get('role') == 'recorder_manifest']
        require(len(entries) == 1, 'member lacks unique embedded recorder manifest')
        embedded = zip_bytes(archive, entries[0]['path'])
    exact(hashlib.sha256(embedded).hexdigest(), manifest_pin['sha256'], 'external/embedded manifest authentication')
    assert_pin(package_pin)
    assert_pin(manifest_pin)
    manifest = decode(embedded)
    exact(manifest['logical_stream_id'], stage['logical_stream_id'], 'manifest logical stream')
    exact(manifest['campaign_identity']['stream_header_sha256'], member['cid1_payload_sha256_hex'], 'manifest CID1 epoch')
    exact(manifest['stream_storage']['decoded_sha256'], member['raw_decoded_sha256'], 'manifest decoded stream')
    witness['member'] = member
    return manifest, manifest_pin


def retain_embedded_source(replay, index, segment, destination):
    relative = segment['source_ref']['session_path']
    # Use the declared source locator only, never infer campaign identity from it.
    startup.member_path(destination.parent, relative)
    entry = 'payload/' + index['game_id'] + '/' + relative
    with zipfile.ZipFile(replay) as archive:
        info = archive.getinfo(entry)
        require(0 < info.file_size <= 64 * 1024**3, 'embedded raw source exceeds 64 GiB observation bound')
        digest, crc, length = hashlib.sha256(), 0, 0
        with zip_stream(archive, info) as stream, destination.open('xb') as out:
            while True:
                block = stream.read(min(1024**2, info.file_size-length+1))
                if not block:
                    break
                length += len(block)
                require(length <= info.file_size, 'embedded raw source exceeds declared length')
                digest.update(block); crc = zlib.crc32(block, crc); out.write(block)
            out.flush(); os.fsync(out.fileno())
        require(length == info.file_size and crc == info.CRC, 'embedded source size/CRC differs')
        exact(digest.hexdigest(), segment['source_ref']['stored_digest'], 'embedded raw source digest')
        return dict(path=str(destination), bytes=length, sha256=digest.hexdigest(),
                    container=str(replay), entry=entry)


def graph_evidence(path):
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        require(len(names) == len(set(names)), 'duplicate ZIP entries')
        indexes = [n for n in names if re.fullmatch('payload/[^/]+/index.json', n)]
        require(len(indexes) == 1, 'expected exactly one product index')
        index = zip_json(archive, indexes[0])
        require(index.get('index_schema') == 'stitched-main-v1' and index.get('segments_file') == 'segments.json',
                'product has no stitched branch evidence; cannot prove transition')
        graph = zip_json(archive, indexes[0].rsplit('/', 1)[0] + '/segments.json')
        return index, graph


def assess_graph(index, graph, expected):
    exact([{field: s[field] for field in SEGMENT_FIELDS} for s in graph['segments']],
          expected['segments'], 'retained branches/ticks/spans/reasons')
    for field in ('main_slices', 'main_head'):
        exact(graph[field], expected[field], field)
    exact(index['main_head'], graph['main_head'], 'index/graph head')
    exact(index['coverage'], expected['coverage'], 'typed surviving coverage/gaps')
    exact([s['hour_index'] for s in index['snapshots']], expected['surviving_hours'], 'index surviving hours')


def assess_playback(stdout, reach, expected, recording_path):
    rows = [decode(line) for line in stdout.splitlines() if line.startswith('{')]
    completed = [r for r in rows if r.get('schema') == 'replay.validate_playback.completed.v1']
    require(len(completed) == 1 and completed[0].get('result') == 'PASS', 'no unique product whole-playback PASS')
    exact(completed[0].get('hours_validated'), len(expected['surviving_hours']), 'whole playback count')
    require(reach.get('schema') == 'hoi4.replay.recording_reach.v4', 'unsupported product reach schema')
    require(startup.same_path(reach['recording'], recording_path), 'reach recording differs')
    exact(reach['hour_count'], len(expected['surviving_hours']), 'reach total hours')
    exact([h['hour_index'] for h in reach['hours']], expected['surviving_hours'], 'product surviving line')
    exact([dict(hour_index=h['hour_index'], gap_inputs=h['gap_inputs'],
                recorder_domain_problems=h['recorder_domain_problems'],
                coverage_warnings=h['coverage_warnings'], map_authority_error=h['map_authority_error'])
           for h in reach['hours']], expected['playback_gaps'], 'product gaps and warnings at every hour')


def assess_transitions(spec, witnesses, graphs, manifests):
    stages = {s['name']: s for s in spec['checkpoints']}
    selected = {}
    for name, stage in stages.items():
        witness = witnesses[name]
        scope = witness['scope']
        assess_epoch(stage, scope, witness['rollover_state'], witness['ticket'])
        exact(startup.key(witness['game']), startup.key(scope), 'observed attachment ' + name)
        manifest = manifests[name]
        require(manifest['pid'] == scope['pid'] and manifest['session_id'] == scope['session_id']
                and manifest['capture_id'] == scope['attempt_id'], 'manifest attachment/session/attempt differs')
        key = campaign(manifest['campaign_identity'], producer=True)
        exact(key.hex(), campaign(stage['campaign']).hex(), 'raw campaign ' + name)
        graph = graphs[stage['recording']]
        exact(graph['campaign']['campaign_key'], key.hex(), 'packaged raw campaign join')
        exact(graph['campaign']['campaign_key_digest'], hashlib.sha256(key).hexdigest(), 'packaged campaign digest')
        segments = [s for s in graph['segments'] if s['segment_id'] == stage['segment_id']]
        require(len(segments) == 1, 'missing or repeated selected segment')
        segment = segments[0]
        pid, creation = startup.key(scope)
        exact(segment['process_start_identity'], f'pid:{pid}:creation-filetime:{creation}', 'branch attachment')
        exact(segment['session_nonce'], scope['session_id'], 'branch session')
        exact(segment['in_session_ordinal'], stage['logical_stream_id'], 'branch logical stream')
        exact(segment['epoch_token'], witness['ticket']['identity_evidence_sha256'], 'branch checkpoint epoch')
        exact(segment['play_order'], witness['ticket']['play_order'], 'branch checkpoint play order')
        if 'member' in witness:
            exact(segment['source_ref']['stored_digest'], witness['member']['raw_sha256'], 'branch authenticated member source')
            exact(segment['source_ref']['decoded_digest'], witness['member']['raw_decoded_sha256'], 'branch authenticated decoded source')
        exact(segment['source_ref']['stored_digest'], witness['raw_identity']['sha256'], 'branch raw source')
        require(segment.get('empty_recovery') is False, 'empty recovery cannot establish a transition')
        selected[name] = segment
    for transition in spec['transitions']:
        a, b = transition['from'], transition['to']
        before, after = selected[a], selected[b]
        same = startup.key(witnesses[a]['game']) == startup.key(witnesses[b]['game'])
        require(same == (transition['attachment'] == 'same'), 'actual attachment relation differs')
        if transition['attachment'] == 'replaced':
            require(startup.key(witnesses[b]['game'])[1] > startup.key(witnesses[a]['game'])[1],
                    'replacement process did not follow original incarnation')
            require(witnesses[b]['previous_attachment_absent'] is True, 'relaunch lacks observed old-process exit')
            require(witnesses[a]['scope']['session_id'] != witnesses[b]['scope']['session_id'], 'replacement attachment reused a recording session')
        kind = transition['kind']
        if kind == 'rollback':
            require(after['cutover_hour'] < before['head_hour'] and after['start_reason'] == 'time_rollback',
                    'no actual producer rollback boundary')
            require(any(s['superseded_by'] == after['segment_id'] for s in before['superseded_spans']),
                    'rollback did not retain superseded earlier branch')
        if kind == 'reload':
            require(after['start_reason'] in ('epoch_change', 'time_rollback'), 'no producer reload boundary')
    return selected


def child_creation_filetime(process):
    require(os.name == 'nt', 'live product commands require Windows process custody')
    created, exited, kernel, user = (ctypes.c_uint64() for _ in range(4))
    get_times = ctypes.WinDLL('kernel32', use_last_error=True).GetProcessTimes
    get_times.argtypes = [ctypes.c_void_p] + [ctypes.POINTER(ctypes.c_uint64)]*4
    get_times.restype = ctypes.c_int
    require(get_times(int(process._handle), ctypes.byref(created), ctypes.byref(exited),
                      ctypes.byref(kernel), ctypes.byref(user)), 'cannot bind owned product process creation')
    return str(created.value)


def product_command(exe, args, root, label, timeout, exe_hash):
    require(startup.identity(exe)['sha256'] == exe_hash, 'viewer bytes changed before invocation')
    command = dict(executable=str(exe), argv=args, executable_sha256=exe_hash,
                   started_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()))
    write(root / (label + '.argv.json'), command)
    stdout, stderr = root/(label+'.stdout.txt'), root/(label+'.stderr.txt')
    # No shell, no arbitrary command supplied by parent, no inherited development
    # switches. The product itself also refuses polluted developer environments.
    env = {k: v for k, v in os.environ.items() if not k.upper().startswith(('HOI4_', 'REPLAY_', 'CAPTURE_'))}
    with stdout.open('xb') as out, stderr.open('xb') as err:
        process = subprocess.Popen([str(exe), *args], stdout=out, stderr=err, env=env,
                                   creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        command['pid'] = process.pid
        # Retain launch PID before the stronger custody query, including failures.
        write(root/(label+'.launch.json'), command)
        command['creation_filetime'] = child_creation_filetime(process)
        write(root/(label+'.process.json'), command)
        try:
            code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            write(root/(label+'.exit.json'), {'exit_code': None, 'pid': process.pid,
                  'status': 'deadline_exceeded_process_retained_no_forced_termination'})
            raise ValueError('product command deadline exceeded; process retained: ' + str(process.pid))
        out.flush(); os.fsync(out.fileno())
        err.flush(); os.fsync(err.fileno())
    write(root/(label+'.exit.json'), {'exit_code': code, 'stdout': startup.identity(stdout),
                                    'stderr': startup.identity(stderr)})
    require(code == 0, 'product command refused: ' + label)
    require(startup.identity(exe)['sha256'] == exe_hash, 'viewer changed during command')
    return stdout


def finish(root):
    plan, observed = read(root/'PLAN.json'), read(root/'OBSERVATIONS.json')
    spec = contract(read(root/'EXPECTATIONS.json'))
    product = Path(plan['product_data_root'])
    # Preserve final journal bytes even when acquisition or graceful stop failed.
    for base, source in [('JOURNAL', 'auto_attach_journal.jsonl'), ('ANCHOR', 'auto_attach_journal.jsonl.anchor')]:
        if (product/source).is_file():
            retain(product/source, root/(base+'_AFTER.jsonl'))
    require(plan['mode'] == 'Run' and not observed['runtime_error'], 'no completed live observation: ' + str(observed['runtime_error']))
    require(observed['stop_requested'] is True and observed['recorder_exit_code'] == 0,
            'owned recorder did not complete graceful termination')
    admission = read(root/'ARTIFACT_ADMISSION.json')
    require(startup.identity(root/'CANDIDATE_ATTESTATION.json')['sha256'] == admission['receipt_sha256'], 'attestation changed')
    for role, path in admission['artifacts'].items():
        require(startup.identity(path)['sha256'] == admission['artifact_sha256'][role], 'admitted artifact changed: ' + role)
    viewer = admission['artifacts']['replay_app']
    viewer_hash = admission['artifact_sha256']['replay_app']
    require(startup.same_path(observed['recorder']['exe_path'], admission['artifacts']['recorder_host']), 'unadmitted recorder')
    require(startup.identity(plan['game_exe'])['sha256'] == plan['game_sha256'], 'game bytes changed during observation')
    for base, source in [('JOURNAL', 'auto_attach_journal.jsonl'), ('ANCHOR', 'auto_attach_journal.jsonl.anchor')]:
        require((root/(base+'_AFTER.jsonl')).read_bytes().startswith((root/(base+'_BEFORE.jsonl')).read_bytes()), 'journal/anchor prefix replaced')
    rows = startup.read_journal(startup.bounded_bytes(root/'JOURNAL_AFTER.jsonl', LIMIT))
    startup.check_anchors(startup.bounded_bytes(root/'ANCHOR_AFTER.jsonl', LIMIT), rows)
    prefix = startup.check_prefix(root)
    fresh = rows[prefix:]
    require(fresh and len({r['coordinator_instance_id'] for r in fresh}) == 1, 'not one fresh coordinator')
    require(not ({r['coordinator_instance_id'] for r in fresh} & {r['coordinator_instance_id'] for r in rows[:prefix]}), 'historical coordinator reused')
    witnesses, manifests, graphs, reports = {}, {}, {}, []
    previous_elapsed = -1
    for stage in spec['checkpoints']:
        name = stage['name']
        witness = read(root/('checkpoint_'+name+'.json'))
        scope = witness['scope']
        require(scope['observation_id'] == plan['observation_id'] and scope['checkpoint'] == name, 'stale checkpoint')
        require(witness['elapsed_ms'] > previous_elapsed, 'checkpoint order regressed')
        previous_elapsed = witness['elapsed_ms']
        game = witness['game']
        require(game['user_sid'] == observed['recorder']['user_sid'] and game['session_id'] == observed['recorder']['session_id']
                and startup.same_path(game['exe_path'], plan['game_exe']), 'checkpoint game scope differs')
        seq = integer(witness['journal_records'], 'checkpoint sequence', prefix+1, len(rows))
        checkpoint_raw = startup.bounded_bytes(root/('checkpoint_'+name+'.journal.jsonl'), LIMIT)
        complete_raw = checkpoint_raw[:checkpoint_raw.rfind(b'\n')+1]
        checkpoint_rows = startup.read_journal(complete_raw)
        exact(checkpoint_rows, rows[:seq], 'retained checkpoint journal prefix')
        exact(read(root/('checkpoint_'+name+'.scope.json')), scope, 'retained parent scope')
        state_before = read(root/('checkpoint_'+name+'.rollover-before.json'))
        state_after = read(root/('checkpoint_'+name+'.rollover-after.json'))
        exact(state_before, state_after, 'epoch crossed checkpoint observation')
        witness['rollover_state'] = state_before
        witness['ticket'] = read(root/('checkpoint_'+name+'.ticket.json'))
        assess_epoch(stage, scope, state_before, witness['ticket'])
        seen = [r for r in rows[prefix:seq] if startup.key(r) == startup.key(scope)]
        require(seen and seen[-1]['state_to'] == 'Recording' and seen[-1]['attempt_id'] == scope['attempt_id']
                and seen[-1]['session_id'] == scope['session_id'], 'checkpoint lacks actual Recording journal binding')
        terminal = [r for r in fresh if r.get('attempt_id') == scope['attempt_id']]
        require(terminal and terminal[-1]['state_to'] == 'Complete' and terminal[-1]['artifact_status'] == 'verified', 'producer attempt not verified complete')
        require(re.fullmatch('[A-Za-z0-9_-]+', scope['attempt_id']), 'unsafe attempt id')
        witnesses[name] = witness
    require({startup.key(r) for r in fresh} == {startup.key(w['game']) for w in witnesses.values()}, 'unscoped or missing attachment in journal')
    require({r['attempt_id'] for r in fresh if r.get('attempt_id')} == {w['scope']['attempt_id'] for w in witnesses.values()}, 'unscoped attempt in journal')
    for stage in spec['checkpoints']:
        name = stage['name']
        witness = witnesses[name]
        manifest, manifest_pin = admitted_member(root, product, stage, witness, viewer, viewer_hash, plan['product_timeout_seconds'])
        require(manifest.get('modset', {}).get('resolved') is True, 'manifest content unresolved')
        # Portable manifests deliberately redact local paths. Content/profile
        # compatibility is established by the product playback command below.
        exact(manifest['modset']['game']['exe_sha256'], plan['game_sha256'], 'manifest game bytes')
        compact = manifest['compact_stream']
        gaps = list(gap_ledger.expand_gaps(compact))
        domain_gaps = list(gap_ledger.expand_domain_gap_outcomes(compact))
        axes = capture_scorecard.score(manifest)
        report = dict(checkpoint=name, manifest=manifest_pin,
                      capture_complete=not any(n for _, n, _ in axes),
                      capture_axes=[dict(axis=a, count=n, detail=d) for a, n, d in axes],
                      gaps=gaps, domain_gaps=domain_gaps)
        write(root/(name+'.scorecard.json'), report)
        reports.append(report)
        manifests[name] = manifest
    for recording in spec['recordings']:
        name = recording['name']
        source = Path(recording['path'])
        require(recording['logical_stream_id'] is None and source.suffix == '.battleplan',
                'transition claim needs one complete stitched campaign .battleplan; independent rollover members cannot be joined by observer')
        pin = retain(source, root/(name+'.battleplan'))
        replay = root/(name+'.battleplan')
        common = ['--non-interactive']
        product_command(viewer, ['--validate-recording', str(replay), *common], root, name+'.admission', plan['product_timeout_seconds'], viewer_hash)
        assert_pin(pin)
        stdout = product_command(viewer, ['--validate-playback', str(replay), '--game', plan['game_root'],
                                '--userdata', plan['user_data'], *common], root, name+'.playback', plan['product_timeout_seconds'], viewer_hash)
        assert_pin(pin)
        reach = root/(name+'.reach.json')
        product_command(viewer, ['--report-recording-reach', str(replay), '--report-out', str(reach),
                                '--report-hours', 'all', *common], root, name+'.reach', plan['product_timeout_seconds'], viewer_hash)
        assert_pin(pin)
        index, graph = graph_evidence(replay)
        assert_pin(pin)
        write(root/(name+'.index.json'), index)
        write(root/(name+'.segments.json'), graph)
        assess_graph(index, graph, recording['expected'])
        assess_playback(stdout.read_text(encoding='utf-8'), read(reach), recording['expected'], str(replay))
        graphs[name] = graph
        for stage in spec['checkpoints']:
            if stage['recording'] == name:
                segment = next(s for s in graph['segments'] if s['segment_id'] == stage['segment_id'])
                raw_pin = retain_embedded_source(replay, index, segment, root/(stage['name']+'.raw'))
                assert_pin(pin)
                witnesses[stage['name']]['raw_identity'] = raw_pin
                write(root/(stage['name']+'.raw.json'), raw_pin)
    assess_transitions(spec, witnesses, graphs, manifests)
    return dict(schema=SCHEMA, status='observed', transitions=spec['transitions'], scorecards=reports,
                qualification='sampled passive transition and product playback evidence; not release acceptance',
                not_evaluated=['independent_worker_protocol_authentication', 'speed5_zero_loss', 'rendering', 'performance', 'clean_machine'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out-dir', required=True, type=Path)
    parser.add_argument('--check-expectations', type=Path)
    parser.add_argument('--check-prefix', action='store_true')
    args = parser.parse_args()
    try:
        if args.check_expectations:
            contract(read(args.check_expectations))
            return 0
        if args.check_prefix:
            startup.check_prefix(args.out_dir)
            return 0
        result = finish(args.out_dir)
    except (Exception, SystemExit) as error:
        if args.check_expectations or args.check_prefix:
            print(str(error))
            return 1
        result = dict(schema=SCHEMA, status='refused', issues=[str(error)], qualification='no transition acceptance')
    result['evidence'] = [startup.identity(p) for p in sorted(args.out_dir.iterdir()) if p.is_file()]
    write(args.out_dir/'TRANSITION_RECEIPT.json', result)
    return 0 if result['status'] == 'observed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
