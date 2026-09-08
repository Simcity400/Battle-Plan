"""Offline fixtures only: these tests are never capture or product playback proof.

Run: python -B -m unittest discover -s extractor/native/scripts
     -p test_campaign_transition_observation.py -v
Shapes follow recorder_host::{campaign_identity,stitching} and replay_app's
recording_reach.v4 / validate_playback.completed.v1 at the owning source revision.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import struct
import sys
import tempfile
import unittest
from unittest import mock
import zipfile
import zlib

import campaign_transition_observation as observer
import test_automatic_capture_scenarios as startup_tests
import test_capture_scorecard as scorecard_tests


def fixture():
    raw = dict(game_unique_id='a4b17fed-ed12-4361-9064-fb6576638110', game_unique_seed='-1')
    key = observer.campaign(raw)
    campaign = dict(raw, schema='hoi4.campaign_identity.v1', state='known', source='live_memory',
                    campaign_id=key.hex())
    segments = []
    witnesses, manifests = {}, {}
    for number, name in enumerate(('before', 'after')):
        scope = dict(pid=10, creation_filetime='1000', attempt_id='attempt-10', session_id='session-10')
        ticket=dict(schema='product-stitch-ticket-v1',logical_stream_id=number,campaign_key_hex=key.hex(),
                    identity_evidence_sha256=str(number)*64,play_order=0,session_nonce=scope['session_id'],
                    process_start_identity='pid:10:creation-filetime:1000')
        state=dict(schema='cid1-rollover-fs-v1',phase='streaming',logical_stream_id=number,
                   campaign_id_hex=key.hex(),cid1_payload_sha256_hex=str(number)*64)
        witnesses[name] = dict(scope=scope, game=dict(scope), previous_attachment_absent=False,
                               raw_identity={'sha256': str(number)*64},ticket=ticket,rollover_state=state)
        manifests[name] = dict(pid=10, session_id='session-10', capture_id='attempt-10', campaign_identity=campaign)
        segments.append(dict(segment_id=number, parent_segment=None if number == 0 else 0,
                             cutover_hour=10 if number == 0 else 11, head_hour=14 if number == 0 else 12,
                             start_reason='session_start' if number == 0 else 'time_rollback', stop_reason='session_end',
                             visible_spans=[dict(start_hour=10,end_hour=10)] if number == 0 else [dict(start_hour=11,end_hour=12)],
                             superseded_spans=[dict(start_hour=11,end_hour=14,superseded_by=1)] if number == 0 else [],
                             process_start_identity='pid:10:creation-filetime:1000', session_nonce='session-10',
                             source_ref={'stored_digest': str(number)*64}, empty_recovery=False,
                             in_session_ordinal=number,epoch_token=str(number)*64,play_order=0))
    graph = dict(campaign={'campaign_key': key.hex(), 'campaign_key_digest': hashlib.sha256(key).hexdigest()},
                 segments=segments, main_head=dict(segment_id=1,hour_index=12),
                 main_slices=[dict(segment_id=0,start_hour=10,end_hour=10),dict(segment_id=1,start_hour=11,end_hour=12)])
    index = dict(game_id='fixture',index_schema='stitched-main-v1',segments_file='segments.json',main_head=graph['main_head'],
                 coverage=[],snapshots=[dict(hour_index=h) for h in (10,11,12)])
    hours = [dict(hour_index=h,gap_inputs=[],recorder_domain_problems=[],coverage_warnings=[],map_authority_error=None) for h in (10,11,12)]
    # Nonzero typed gaps must survive unchanged; fixture does not claim zero loss.
    hours[1]['gap_inputs'] = [dict(input_id='test_input',disposition='gap',blocker='source_unavailable')]
    expected = dict(segments=[{k:s[k] for k in observer.SEGMENT_FIELDS} for s in segments],
                    main_slices=graph['main_slices'],main_head=graph['main_head'],coverage=[],
                    surviving_hours=[10,11,12],playback_gaps=hours)
    root = Path(tempfile.gettempdir())
    spec = dict(schema='hoi4.campaign_transition_expectations.v1',
                checkpoints=[dict(name=name,scope_file=str(root/(name+'.scope.json')),recording='campaign',
                                  segment_id=i,logical_stream_id=i,campaign=raw) for i,name in enumerate(('before','after'))],
                transitions=[dict(kind='rollback',attachment='same',**{'from':'before','to':'after'})],
                recordings=[dict(name='campaign',path=str(root/'campaign.battleplan'),logical_stream_id=None,expected=expected)])
    reach = dict(schema='hoi4.replay.recording_reach.v4',recording='fixture.battleplan',hour_count=3,hours=hours)
    stdout = json.dumps(dict(schema='replay.validate_playback.completed.v1',result='PASS',hours_validated=3))
    return spec,witnesses,{'campaign':graph},manifests,index,reach,stdout


def finish_fixture(root):
    """Build a test-only receipt tree. No actual PE, H4CR, or admitted container."""
    spec,witnesses,graphs,manifests,index,reach,stdout=fixture()
    product=root/'product'; product.mkdir()
    run=product/'run_attempt-10'; run.mkdir()
    game=root/'hoi4.exe'; game.write_bytes(b'inert fixture game')
    viewer=root/'replay_app.exe'; viewer.write_bytes(b'inert fixture viewer')
    host=root/'collector_record.exe'; host.write_bytes(b'inert fixture recorder')
    attestation=root/'CANDIDATE_ATTESTATION.json'; attestation.write_text('not a real attestation')
    write=observer.write
    write(root/'PLAN.json',dict(mode='Run',observation_id='fixture-only',game_exe=str(game),game_sha256=observer.startup.identity(game)['sha256'],
                               user_data=str(root/'profile'),game_root=str(root),product_data_root=str(product),product_timeout_seconds=1))
    write(root/'ARTIFACT_ADMISSION.json',dict(receipt_sha256=observer.startup.identity(attestation)['sha256'],
              artifacts={'recorder_host':str(host),'replay_app':str(viewer)},
              artifact_sha256={'recorder_host':observer.startup.identity(host)['sha256'],'replay_app':observer.startup.identity(viewer)['sha256']}))
    write(root/'OBSERVATIONS.json',dict(runtime_error=None,stop_requested=True,recorder_exit_code=0,
                 recorder=dict(pid=90,creation_filetime='900',session_id=3,user_sid='fixture-owner',exe_path=str(host))))
    _,rows=startup_tests.fixture()
    for row in rows:
        row['process_creation_filetime']=1000
        row['exe_path']=str(game)
    journal=startup_tests.encode(rows)
    rows=observer.startup.read_journal(journal)
    anchors=b''.join(json.dumps(dict(schema='auto_attach_journal_anchor.v1',event_seq=r['event_seq'],head_hash=r['record_hash'])).encode()+b'\n' for r in rows)
    (product/'auto_attach_journal.jsonl').write_bytes(journal)
    (product/'auto_attach_journal.jsonl.anchor').write_bytes(anchors)
    (root/'JOURNAL_BEFORE.jsonl').write_bytes(b'')
    (root/'ANCHOR_BEFORE.jsonl').write_bytes(b'')
    members=[]; raw_members=[]
    for i,stage in enumerate(spec['checkpoints']):
        name=stage['name']; witness=witnesses[name]
        raw=run/(name+'.h4cr'); raw.write_bytes((name+' inert raw fixture').encode())
        manifest=scorecard_tests.manifest([], [scorecard_tests.derived([10],11,11,'source_unavailable')])
        manifest.update(copy.deepcopy(manifests[name]))
        manifest['logical_stream_id']=i
        manifest['stream_storage']={'decoded_sha256':observer.startup.identity(raw)['sha256']}
        manifest['campaign_identity']['stream_header_sha256']=str(i)*64
        manifest['modset']=dict(resolved=True,pdx_user_dir='profile',game=dict(exe_path='hoi4.exe',exe_sha256=observer.startup.identity(game)['sha256']))
        member_dir=run/'logical-streams/packages'/f'{i:020d}'; member_dir.mkdir(parents=True)
        manifest_path=member_dir/'portable_session_manifest.json'; write(manifest_path,manifest)
        package=member_dir/'native_session.battleplan'
        with zipfile.ZipFile(package,'w') as archive:
            embedded_path='payload/fixture/recorder/native_session_manifest.json'
            archive.write(manifest_path,embedded_path)
            archive.writestr('manifest.json',json.dumps({'entries':[dict(role='recorder_manifest',path=embedded_path)]}))
        member=dict(logical_stream_id=i,campaign_id_hex=manifest['campaign_identity']['campaign_id'],cid1_payload_sha256_hex=str(i)*64,
                    manifest_relative_path=manifest_path.relative_to(run).as_posix(),manifest_sha256=observer.startup.identity(manifest_path)['sha256'],
                    package_relative_path=package.relative_to(run).as_posix(),package_sha256=observer.startup.identity(package)['sha256'],
                    raw_sha256=observer.startup.identity(raw)['sha256'],raw_bytes=raw.stat().st_size,raw_decoded_sha256=observer.startup.identity(raw)['sha256'],
                    recording_admitted=True,pre_world_transient=False,recovery=False)
        members.append(member)
        raw_members.append(dict(logical_stream_id=i,campaign_id_hex=member['campaign_id_hex'],cid1_payload_sha256_hex=str(i)*64,
                                file_sha256_hex=member['raw_sha256'],file_bytes=raw.stat().st_size))
        witness['scope'].update(observation_id='fixture-only',checkpoint=name,manifest_path=str(manifest_path))
        witness['game'].update(session_id=3,user_sid='fixture-owner',exe_path=str(game))
        witness.update(elapsed_ms=1+i,journal_records=1)
        graphs['campaign']['segments'][i]['source_ref']['stored_digest']=observer.startup.identity(raw)['sha256']
        graphs['campaign']['segments'][i]['source_ref']['decoded_digest']=observer.startup.identity(raw)['sha256']
        graphs['campaign']['segments'][i]['source_ref']['session_path']='raw/'+name+'.h4cr'
        write(root/('checkpoint_'+name+'.json'),witness)
        write(root/('checkpoint_'+name+'.scope.json'),witness['scope'])
        write(root/('checkpoint_'+name+'.rollover-before.json'),witness['rollover_state'])
        write(root/('checkpoint_'+name+'.rollover-after.json'),witness['rollover_state'])
        write(root/('checkpoint_'+name+'.ticket.json'),witness['ticket'])
        (root/('checkpoint_'+name+'.journal.jsonl')).write_bytes(startup_tests.encode(rows[:1]))
    replay=product/'campaign.battleplan'
    with zipfile.ZipFile(replay,'w') as archive:
        archive.writestr('payload/fixture/index.json',json.dumps(index))
        archive.writestr('payload/fixture/segments.json',json.dumps(graphs['campaign']))
        for name in ('before','after'):
            archive.write(run/(name+'.h4cr'),'payload/fixture/raw/'+name+'.h4cr')
    spec['recordings'][0]['path']=str(replay)
    source_descriptor=run/'logical-streams/logical-stream-set.json'
    write(source_descriptor,dict(schema='cid1-rollover-set-v1',ordered_streams=raw_members,transitions=[]))
    write(run/'logical-stream-package-set.json',dict(schema='cid1-rollover-package-set-v1',ordered_streams=members,transitions=[],
          source_descriptor='logical-streams/logical-stream-set.json',source_descriptor_sha256=observer.startup.identity(source_descriptor)['sha256']))
    write(root/'EXPECTATIONS.json',spec)
    def fake_product(exe,args,out,label,timeout,exe_hash):
        # Used ONLY under mock.patch in offline tests. Never launches a product.
        log=out/(label+'.stdout.txt')
        log.write_text(stdout if '--validate-playback' in args else 'offline mock only')
        if '--report-out' in args:
            report=copy.deepcopy(reach)
            report['recording']=args[1]
            write(Path(args[args.index('--report-out')+1]),report)
        return log
    return fake_product


class FullAssessmentTests(unittest.TestCase):
    def test_substituted_gap_manifest_cannot_impersonate_published_member(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); runner=finish_fixture(root)
            scope=observer.read(root/'checkpoint_before.scope.json')
            manifest_path=Path(scope['manifest_path']); manifest=observer.read(manifest_path)
            manifest['compact_stream']['gap_ledger']=observer.gap_ledger.pack_rows_for_tests([],[])
            manifest_path.write_text(json.dumps(manifest))
            descriptor_path=root/'product/run_attempt-10/logical-stream-package-set.json'
            descriptor=observer.read(descriptor_path)
            descriptor['ordered_streams'][0]['manifest_sha256']=observer.startup.identity(manifest_path)['sha256']
            descriptor_path.write_text(json.dumps(descriptor))
            with mock.patch.object(observer,'product_command',side_effect=runner),self.assertRaisesRegex(ValueError,'external/embedded manifest'):
                observer.finish(root)

    def test_recording_mutation_between_every_product_consumer_is_refused(self):
        for phase in ('campaign.admission','campaign.playback','campaign.reach'):
            with self.subTest(phase=phase),tempfile.TemporaryDirectory() as temp:
                root=Path(temp); runner=finish_fixture(root)
                def mutate(exe,args,out,label,timeout,exe_hash):
                    log=runner(exe,args,out,label,timeout,exe_hash)
                    if label==phase:
                        with (root/'campaign.battleplan').open('ab') as changed: changed.write(b'changed bytes')
                    return log
                with mock.patch.object(observer,'product_command',side_effect=mutate),self.assertRaisesRegex(ValueError,'retained artifact changed'):
                    observer.finish(root)

    def test_stale_or_crossed_epoch_is_rejected_before_any_product_command(self):
        for side in ('before','after'):
            with self.subTest(side=side),tempfile.TemporaryDirectory() as temp:
                root=Path(temp); finish_fixture(root)
                source=root/'checkpoint_after.rollover-before.json'
                (root/('checkpoint_before.rollover-'+side+'.json')).write_bytes(source.read_bytes())
                with mock.patch.object(observer,'product_command') as commands,self.assertRaisesRegex(ValueError,'epoch crossed'):
                    observer.finish(root)
                commands.assert_not_called()

    def test_full_offline_fixture_retains_raw_scorecards_and_calls_all_product_routes(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); runner=finish_fixture(root)
            with mock.patch.object(observer,'product_command',side_effect=runner) as commands:
                result=observer.finish(root)
            self.assertEqual(result['status'],'observed')  # test-only model result
            self.assertEqual([c.args[1][0] for c in commands.call_args_list],
                             ['--validate-recording','--validate-recording','--validate-recording','--validate-playback','--report-recording-reach'])
            self.assertIn('all',commands.call_args_list[-1].args[1])
            self.assertTrue(result['scorecards'][0]['gaps'])
            self.assertFalse(result['scorecards'][0]['capture_complete'])
            self.assertTrue((root/'after.raw').is_file())
            self.assertTrue((root/'campaign.segments.json').is_file())

    def test_stale_checkpoint_wrong_journal_and_ungraceful_exit_refuse_before_products(self):
        for defect in ('stale','journal','scope','stop','runtime'):
            with self.subTest(defect=defect),tempfile.TemporaryDirectory() as temp:
                root=Path(temp); finish_fixture(root)
                if defect in ('stale','scope'):
                    path=root/'checkpoint_after.json'; data=observer.read(path)
                    data['scope']['observation_id' if defect=='stale' else 'session_id']='wrong'
                    path.write_text(json.dumps(data))
                elif defect=='journal':
                    (root/'checkpoint_after.journal.jsonl').write_bytes(b'')
                else:
                    path=root/'OBSERVATIONS.json'; data=observer.read(path)
                    data['runtime_error' if defect=='runtime' else 'stop_requested']='failed' if defect=='runtime' else False
                    path.write_text(json.dumps(data))
                with mock.patch.object(observer,'product_command') as commands,self.assertRaises(ValueError):
                    observer.finish(root)
                commands.assert_not_called()
                self.assertTrue((root/'JOURNAL_AFTER.jsonl').is_file())

    def test_product_refusal_never_reaches_transition_success(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); finish_fixture(root)
            with mock.patch.object(observer,'product_command',side_effect=ValueError('actual product refusal')):
                with self.assertRaisesRegex(ValueError,'actual product refusal'): observer.finish(root)
            self.assertTrue((root/'before.manifest.json').is_file())
            self.assertFalse((root/'TRANSITION_RECEIPT.json').exists())


class TransitionTests(unittest.TestCase):
    def setUp(self):
        self.spec,self.witnesses,self.graphs,self.manifests,self.index,self.reach,self.stdout = fixture()

    def assess(self):
        observer.contract(self.spec)
        return observer.assess_transitions(self.spec,self.witnesses,self.graphs,self.manifests)

    def test_rollback_retains_abandoned_future_and_product_excludes_it(self):
        self.assess()
        expected = self.spec['recordings'][0]['expected']
        observer.assess_graph(self.index,self.graphs['campaign'],expected)
        observer.assess_playback(self.stdout,self.reach,expected,'fixture.battleplan')
        self.assertEqual(expected['surviving_hours'],[10,11,12])
        self.assertEqual(self.graphs['campaign']['segments'][0]['head_hour'],14)

    def test_reload_needs_actual_producer_boundary(self):
        self.spec['transitions'][0]['kind'] = 'reload'
        self.assess()
        self.graphs['campaign']['segments'][1]['start_reason'] = 'reload_uncertain'
        with self.assertRaisesRegex(ValueError,'reload boundary'): self.assess()

    def test_relaunch_accepts_reused_pid_with_new_creation_and_observed_exit(self):
        self.spec['transitions'][0].update(kind='relaunch',attachment='replaced')
        self.witnesses['after']['scope']['creation_filetime'] = '2000'
        self.witnesses['after']['game']['creation_filetime'] = '2000'
        self.witnesses['after']['previous_attachment_absent'] = True
        self.witnesses['after']['ticket']['process_start_identity'] = 'pid:10:creation-filetime:2000'
        self.witnesses['after']['scope']['session_id'] = 'new-session'
        self.witnesses['after']['ticket']['session_nonce'] = 'new-session'
        self.manifests['after']['session_id'] = 'new-session'
        self.graphs['campaign']['segments'][1]['session_nonce'] = 'new-session'
        self.graphs['campaign']['segments'][1]['process_start_identity'] = 'pid:10:creation-filetime:2000'
        self.assess()
        self.witnesses['after']['previous_attachment_absent'] = False
        with self.assertRaisesRegex(ValueError,'old-process exit'): self.assess()

    def test_process_reuse_never_authorizes_campaign_join(self):
        self.witnesses['after']['game']['creation_filetime'] = '2000'
        with self.assertRaisesRegex(ValueError,'attachment'): self.assess()

    def test_identity_change_starts_distinct_campaign(self):
        stage = self.spec['checkpoints'][1]
        stage['campaign'] = dict(stage['campaign'],game_unique_id='b4b17fed-ed12-4361-9064-fb6576638110',game_unique_seed='-2')
        stage['recording'] = 'new'
        self.spec['transitions'][0]['kind'] = 'identity-change'
        self.spec['recordings'].append(dict(self.spec['recordings'][0],name='new',path=str(Path(tempfile.gettempdir())/'new.battleplan')))
        key = observer.campaign(stage['campaign'])
        self.manifests['after']['campaign_identity'] = dict(self.manifests['after']['campaign_identity'],**stage['campaign'],campaign_id=key.hex())
        self.witnesses['after']['ticket']['campaign_key_hex']=key.hex()
        self.witnesses['after']['rollover_state']['campaign_id_hex']=key.hex()
        self.graphs['new'] = copy.deepcopy(self.graphs['campaign'])
        self.graphs['new']['campaign'] = {'campaign_key':key.hex(),'campaign_key_digest':hashlib.sha256(key).hexdigest()}
        self.assess()
        self.graphs['new']['campaign'] = self.graphs['campaign']['campaign']
        with self.assertRaisesRegex(ValueError,'raw campaign join'): self.assess()

    def test_single_component_change_is_not_a_supported_in_process_positive_scenario(self):
        for field,value in [('game_unique_seed','-2'),('game_unique_id','b4b17fed-ed12-4361-9064-fb6576638110')]:
            with self.subTest(field=field):
                spec=copy.deepcopy(self.spec)
                spec['checkpoints'][1]['campaign']=dict(spec['checkpoints'][1]['campaign'],**{field:value})
                spec['checkpoints'][1]['recording']='new'
                spec['recordings'].append(dict(spec['recordings'][0],name='new'))
                spec['transitions'][0]['kind']='identity-change'
                with self.assertRaisesRegex(ValueError,'single-component'): observer.contract(spec)

    def test_real_producer_manifest_fixture_uses_hashed_key_then_graph_digest(self):
        source=Path(observer.__file__).parents[1]/'recorder_host/src/fixtures/campaign_identity_known_manifest.json'
        manifest=observer.read(source)['campaign_identity']
        key=observer.campaign(manifest,producer=True)
        self.assertEqual(key.hex(),'b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f')
        self.assertEqual(len(key),32)
        self.graphs['campaign']['campaign']['campaign_key']=(b'hoi4-campaign-id-v1\0'+self.spec['checkpoints'][0]['campaign']['game_unique_id'].encode()+b'\0-1').hex()
        with self.assertRaisesRegex(ValueError,'raw campaign join'): self.assess()

    def test_earlier_epoch_cannot_satisfy_current_before_after_checkpoint(self):
        self.witnesses['before']['rollover_state']=copy.deepcopy(self.witnesses['after']['rollover_state'])
        with self.assertRaisesRegex(ValueError,'contemporaneous logical stream'): self.assess()

    def test_seed_is_canonical_signed_i32_including_extremes(self):
        for seed in ('-2147483648','2147483647','0','-1'):
            observer.campaign(dict(self.spec['checkpoints'][0]['campaign'],game_unique_seed=seed))
        for seed in ('2147483648','-2147483649','-0','+1','01',1,True,None):
            with self.subTest(seed=seed),self.assertRaisesRegex(ValueError,'signed'):
                observer.campaign(dict(self.spec['checkpoints'][0]['campaign'],game_unique_seed=seed))

    def test_header_and_filename_are_not_campaign_identity(self):
        for value in ('campaign.battleplan','','a4b17fed-ed12-4361-9064-fb6576638110'.upper(),None):
            with self.subTest(value=value),self.assertRaises(ValueError):
                observer.campaign(dict(self.spec['checkpoints'][0]['campaign'],game_unique_id=value))
        self.manifests['after']['campaign_identity']['source'] = 'save_header'
        with self.assertRaisesRegex(ValueError,'live-memory'): self.assess()

    def test_wrong_manifest_hash_or_raw_source_refuses(self):
        self.witnesses['after']['raw_identity']['sha256'] = 'f'*64
        with self.assertRaisesRegex(ValueError,'raw source'): self.assess()

    def test_missing_or_wrong_manifest_session_refuses(self):
        self.manifests['after']['session_id'] = 'other-session'
        with self.assertRaisesRegex(ValueError,'manifest attachment'): self.assess()

    def test_unretained_rollback_refuses(self):
        self.graphs['campaign']['segments'][0]['superseded_spans'] = []
        with self.assertRaisesRegex(ValueError,'superseded'): self.assess()

    def test_equal_tick_is_not_rollback(self):
        self.graphs['campaign']['segments'][1]['cutover_hour'] = 14
        with self.assertRaisesRegex(ValueError,'rollback boundary'): self.assess()

    def test_extra_or_missing_branch_refuses_exact_contract(self):
        graph = self.graphs['campaign']
        graph['segments'].append(copy.deepcopy(graph['segments'][1]))
        with self.assertRaisesRegex(ValueError,'retained branches'):
            observer.assess_graph(self.index,graph,self.spec['recordings'][0]['expected'])

    def test_abandoned_future_in_product_playback_refuses(self):
        self.reach['hours'].append(dict(self.reach['hours'][-1],hour_index=14))
        with self.assertRaisesRegex(ValueError,'surviving line'):
            observer.assess_playback(self.stdout,self.reach,self.spec['recordings'][0]['expected'],'fixture.battleplan')

    def test_gap_is_never_silently_made_clean(self):
        reach = copy.deepcopy(self.reach)
        reach['hours'][1]['gap_inputs'] = []
        with self.assertRaisesRegex(ValueError,'gaps and warnings'):
            observer.assess_playback(self.stdout,reach,self.spec['recordings'][0]['expected'],'fixture.battleplan')

    def test_partial_or_missing_whole_playback_pass_refuses(self):
        for stdout in ('',self.stdout+'\n'+self.stdout,self.stdout.replace('3','2'),self.stdout.replace('PASS','STARTED')):
            with self.subTest(stdout=stdout),self.assertRaises(ValueError):
                observer.assess_playback(stdout,self.reach,self.spec['recordings'][0]['expected'],'fixture.battleplan')

    def test_wrong_recording_in_reach_refuses(self):
        with self.assertRaisesRegex(ValueError,'recording differs'):
            observer.assess_playback(self.stdout,self.reach,self.spec['recordings'][0]['expected'],'other.battleplan')

    def test_missing_expected_gaps_never_defaults_to_zero(self):
        del self.spec['recordings'][0]['expected']['coverage']
        with self.assertRaisesRegex(ValueError,'exact recording expectation'): observer.contract(self.spec)

    def test_unknown_kind_and_nonadjacent_transition_refuse(self):
        self.spec['transitions'][0]['kind'] = 'anything'
        with self.assertRaises(ValueError): observer.contract(self.spec)
        self.spec['transitions'][0]['kind'] = 'rollback'
        self.spec['transitions'][0]['from'] = 'after'
        with self.assertRaises(ValueError): observer.contract(self.spec)

    def test_strict_json_and_tick_types(self):
        for raw in ('{"x":1,"x":2}','{"x":NaN}'):
            with self.assertRaises(ValueError): observer.decode(raw)
        for hour in (True,10.0,'10'):
            with self.assertRaises(ValueError): observer.integer(hour,'hour')

    def test_atomic_retention_never_overwrites_previous_attempt(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); source=root/'source'; destination=root/'retained'
            source.write_bytes(b'fixture raw bytes, not a capture')
            pin=observer.retain(source,destination)
            self.assertEqual(pin['sha256'],hashlib.sha256(source.read_bytes()).hexdigest())
            with self.assertRaises(FileExistsError): observer.retain(source,destination)

    def test_zip_graph_requires_actual_stitched_index(self):
        with tempfile.TemporaryDirectory() as temp:
            path=Path(temp)/'fixture.battleplan'
            with zipfile.ZipFile(path,'w',compression=zipfile.ZIP_DEFLATED) as archive:
                archive.writestr('payload/fixture/index.json',json.dumps(self.index))
                archive.writestr('payload/fixture/segments.json',json.dumps(self.graphs['campaign']))
            index,graph=observer.graph_evidence(path)
            observer.assess_graph(index,graph,self.spec['recordings'][0]['expected'])
            with zipfile.ZipFile(path,'w') as archive:
                archive.writestr('payload/fixture/index.json','{"snapshots":[]}')
            with self.assertRaisesRegex(ValueError,'no stitched branch evidence'): observer.graph_evidence(path)

    def test_production_zstandard_metadata_reader_checks_size_and_crc(self):
        import zstandard
        value={'index_schema':'stitched-main-v1'}
        raw=json.dumps(value).encode(); compressed=zstandard.ZstdCompressor().compress(raw)
        name=b'payload/fixture/index.json'
        header=struct.pack('<4s5H3I2H',b'PK\x03\x04',63,0,93,0,0,zlib.crc32(raw),len(compressed),len(raw),len(name),0)
        info=zipfile.ZipInfo(name.decode()); info.header_offset=0; info.compress_type=93
        info.compress_size=len(compressed); info.file_size=len(raw); info.CRC=zlib.crc32(raw)
        with tempfile.TemporaryDirectory() as temp:
            path=Path(temp)/'method93-fixture'; path.write_bytes(header+name+compressed)
            archive=mock.Mock(filename=str(path)); archive.getinfo.return_value=info
            self.assertEqual(observer.zip_json(archive,name.decode()),value)
            info.CRC=0
            with self.assertRaisesRegex(ValueError,'size/CRC'): observer.zip_json(archive,name.decode())

    def test_checkpoint_names_are_case_insensitively_unique(self):
        self.spec['checkpoints'][1]['name']='BEFORE'
        with self.assertRaisesRegex(ValueError,'unique safe names'): observer.contract(self.spec)

    def test_command_timeout_preserves_process_and_raw_argv(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); exe=root/'never-executed-fixture.exe'; exe.write_bytes(b'fixture')
            process=mock.Mock(pid=123)
            process.wait.side_effect=subprocess.TimeoutExpired('fixture',1)
            with mock.patch.object(observer.subprocess,'Popen',return_value=process),mock.patch.object(observer,'child_creation_filetime',return_value='1000'),self.assertRaisesRegex(ValueError,'retained'):
                observer.product_command(exe,['--validate-recording','fixture'],root,'command',1,observer.startup.identity(exe)['sha256'])
            process.kill.assert_not_called(); process.terminate.assert_not_called()
            self.assertEqual(observer.read(root/'command.argv.json')['argv'],['--validate-recording','fixture'])
            self.assertIsNone(observer.read(root/'command.exit.json')['exit_code'])

    def test_missing_live_observations_cli_is_durable_refusal(self):
        with tempfile.TemporaryDirectory() as temp:
            result=subprocess.run([sys.executable,'-B',observer.__file__,'--out-dir',temp],capture_output=True,text=True)
            self.assertEqual(result.returncode,1,result.stderr)
            receipt=observer.read(Path(temp)/'TRANSITION_RECEIPT.json')
            self.assertEqual(receipt['status'],'refused')
            self.assertNotIn('scenario_observed',receipt)

    @unittest.skipUnless(os.name == 'nt','PowerShell entrypoint parser and no-launch refusal are Windows checks')
    def test_powershell_parse_and_missing_input_refusal_without_launch(self):
        script=Path(observer.__file__).with_name('Invoke-CampaignTransitionObservation.ps1')
        command="$tokens=$null; $errors=$null; [void][System.Management.Automation.Language.Parser]::ParseFile($args[0],[ref]$tokens,[ref]$errors); $errors | ForEach-Object { $_.ToString() }; if ($errors.Count) { exit 1 }"
        # -Command positional forwarding differs from -File; use a temporary wrapper.
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); wrapper=root/'parse.ps1'; wrapper.write_text(command)
            parsed=subprocess.run(['powershell.exe','-NoProfile','-ExecutionPolicy','Bypass','-File',str(wrapper),str(script)],capture_output=True,text=True)
            self.assertEqual(parsed.returncode,0,parsed.stdout+parsed.stderr)
            args=['powershell.exe','-NoProfile','-ExecutionPolicy','Bypass','-File',str(script),'-Mode','Plan',
                  '-RepoRoot',str(script.parents[3]),'-ExpectationsPath',str(root/'missing.json'),
                  '-ArtifactAttestationPath',str(root/'missing-attestation.json'),'-GameExe',str(root/'missing-game.exe'),
                  '-GameExeSha256','a'*64,'-UserData',str(root/'profile'),'-ProductDataRoot',str(root/'product'),
                  '-OutDir',str(root/'evidence'),'-CheckpointTimeoutSeconds','1','-FinalizeTimeoutSeconds','1','-ProductTimeoutSeconds','1']
            result=subprocess.run(args,capture_output=True,text=True)
            self.assertEqual(result.returncode,1,result.stdout+result.stderr)
            self.assertEqual(observer.read(root/'evidence/TRANSITION_RECEIPT.json')['status'],'refused')
            self.assertFalse((root/'evidence/recorder.argv.json').exists())


if __name__ == '__main__':
    unittest.main()
