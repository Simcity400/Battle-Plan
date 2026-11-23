import { gatedTime, heldCountry } from '../../test-support/gated.ts';
import { persistedSpec } from '../../dashboard/regionTime.ts';
import assert from 'node:assert/strict';
import type { DrillState } from '../drill/drillState';
import test from 'node:test';
import { registerWidgets } from '../../dashboard/definitions.ts';
import { partsRaceModel, validatePartsRaceReply } from '../../dashboard/partsRace.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { runAt } from '../series.ts';
import { axisAt, regionHour } from '../../dashboard/regionTime.ts';
import { drillAction } from '../drill/drillState.ts';
import { CLOSED_PARTS_RACE, partsRaceDrillActors, partsRaceDrillBands, partsRaceDrillData, pinPartsRace, reducePartsRaceDrill, validateNativePartsRaceReply } from './partsRaceDrill.ts';
import { bindCountryParts, countryPartsSelection, pinNativeCountryParts, savedCountryPartsSelection } from './countryPartsDrillData.ts';

const WIDE = { id: 'wide', axisOffset: 0, branch: { id: 'wide', firstHour: 0, lastHour: 100_000 } } as unknown as BoundTrack;
const drillAt = (state: DrillState, playhead: number) =>
  regionHour({ widget: { trackIds: ['wide'], time: { mode: 'follow' } } as unknown as WidgetSpec, action: drillAction(state) }, WIDE, [WIDE], axisAt(playhead));

test('drawn actor candidates override ranking without adding hidden or missing countries', () => {
  let fallbacks = 0;
  const fallback = () => { fallbacks++; return 'hidden rank leader'; };
  assert.deepEqual(partsRaceDrillActors(undefined, ['drawn B', 'drawn A', 'drawn B'], fallback), ['drawn B', 'drawn A']);
  assert.deepEqual(partsRaceDrillActors(undefined, [], fallback), []);
  assert.deepEqual(partsRaceDrillActors('hidden rank leader', ['drawn B'], fallback), ['drawn B']);
  assert.deepEqual(partsRaceDrillActors('direct race or mix', undefined, fallback), ['direct race or mix']);
  assert.equal(fallbacks, 0);
  assert.deepEqual(partsRaceDrillActors(undefined, undefined, fallback), ['hidden rank leader']);
  assert.equal(fallbacks, 1);
  const candidates = Array.from({ length: 5000 }, (_, index) => `authored country ${index}, with separators`);
  const tags = partsRaceDrillActors(undefined, candidates, fallback);
  const state = reducePartsRaceDrill(CLOSED_PARTS_RACE, { type: 'actors', key: 'branch', hour: 12, tags });
  assert.equal(state.choices?.tags.length, 5000);
  assert.equal(reducePartsRaceDrill(state, { type: 'choose', key: 'branch', hour: 12, tag: candidates[4999] }).breakdown, candidates[4999]);
  assert.equal(fallbacks, 1);
});

test('actor chooser retains the clicked hour and branch and rejects superseded input', () => {
  const key = JSON.stringify(['campaign', 'branch']);
  let state = reducePartsRaceDrill(CLOSED_PARTS_RACE, { type: 'open', breakdown: 'old actor', hour: 20, follow: true });
  state = reducePartsRaceDrill(state, { type: 'actors', key, hour: 31.9, tags: ['drawn A', 'drawn B'] });
  assert.equal(state.breakdown, null);
  assert.equal(state.follow, false);
  assert.equal(drillAt(state, 500), 31);
  assert.equal(reducePartsRaceDrill(state, { type: 'follow' }), state);
  assert.equal(reducePartsRaceDrill(state, { type: 'pick', hour: 500 }), state);
  state = reducePartsRaceDrill(state, { type: 'actors', key, hour: 44, tags: ['drawn C', 'drawn D'] });
  assert.equal(reducePartsRaceDrill(state, { type: 'choose', key, hour: 31, tag: 'drawn A' }), state);
  assert.equal(reducePartsRaceDrill(state, { type: 'choose', key: 'another branch', hour: 44, tag: 'drawn C' }), state);
  assert.equal(reducePartsRaceDrill(state, { type: 'choose', key, hour: 44, tag: 'hidden rank leader' }), state);
  state = reducePartsRaceDrill(state, { type: 'choose', key, hour: 44, tag: 'drawn D' });
  assert.deepEqual(state, { breakdown: 'drawn D', spec: false, follow: false, hour: 44, choices: null });
  state = reducePartsRaceDrill(state, { type: 'follow' });
  assert.equal(drillAt(state, 500), 500);
  state = reducePartsRaceDrill(state, { type: 'actors', key, hour: 60, tags: ['drawn A', 'drawn B'] });
  state = reducePartsRaceDrill(state, { type: 'close' });
  assert.equal(reducePartsRaceDrill(state, { type: 'choose', key, hour: 60, tag: 'drawn A' }), state);
  state = reducePartsRaceDrill(state, { type: 'actors', key, hour: 60, tags: ['drawn A', 'drawn B'] });
  state = reducePartsRaceDrill(state, { type: 'open', breakdown: 'direct actor', hour: 61, follow: true });
  assert.deepEqual(state, { breakdown: 'direct actor', spec: false, follow: true, hour: null, choices: null });
});

test('drill stacks retain exact totals through missing classification, zeros, gaps and identity changes', () => {
  const model = partsRaceModel(validatePartsRaceReply({ branchId: 'branch', warnings: [],
    parts: [{ key: 'mod type', name: 'Mod type', unresolved: false }, { key: 'unknown type', name: null, unresolved: true }],
    series: [{ tag: 'mod country', membership: null, segments: [{ firstHour: 0, lastHour: 3 }, { firstHour: 5, lastHour: 5 }],
      runs: [{ firstHour: 0, lastHour: 0, startsSegment: true, total: 5, values: [3, 2] },
        { firstHour: 1, lastHour: 1, startsSegment: false, total: null, values: [null, null] },
        { firstHour: 2, lastHour: 2, startsSegment: false, total: 5, values: [3, null] },
        { firstHour: 3, lastHour: 3, startsSegment: false, total: 0, values: [0, 0] },
        { firstHour: 5, lastHour: 5, startsSegment: true, total: 5, values: [3, null] }] }] }));
  const bands = partsRaceDrillBands(model, 'mod country', null, { firstHour: 0, lastHour: 5 });
  const values = (hour: number) => bands.map(runs => runAt(runs, hour)?.value ?? null);
  assert.deepEqual(values(0), [3, 2, 0]);
  assert.deepEqual(values(1), [null, null, null]);
  assert.deepEqual(values(2), [3, 0, 2]);
  assert.deepEqual(values(3), [0, 0, 0]);
  assert.deepEqual(values(4), [null, null, null]);
  assert.ok(bands.every(runs => runAt(runs, 5)?.breakBefore));
  const cut = partsRaceDrillBands(model, 'mod country', [{ firstHour: 2, lastHour: 3 }], { firstHour: 0, lastHour: 5 });
  assert.ok(cut.every(runs => runAt(runs, 0) == null && runAt(runs, 5) == null));
});

registerWidgets([{ kind: 'parts-drill-test', version: 1, title: 'Force size', icon: 'layers',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 } }]);

test('pin keeps the drilled country and chosen moment instead of re-ranking its source group', () => {
  const source: WidgetSpec = { id: 'source', kind: 'parts-drill-test', configVersion: 1, title: 'Force size', campaign: 'tracks',
    trackIds: ['track'], countries: { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false },
    time: { mode: 'follow' }, contentScale: 125, options: { drill: 'popout', ownership: 'roster', measure: 'planes' } };
  const track = { id: 'track', branch: { firstHour: 1000 } } as Pick<BoundTrack, 'id' | 'branch'>;
  const held = pinPartsRace(source, track, 'country with , separators', gatedTime({ mode: 'pin', windows: { track: { firstOffset: 51, lastOffset: 51 } } }), 'Types');
  assert.deepEqual(held.countries, { kind: 'tags', tags: ['country with , separators'] });
  assert.deepEqual(held.time.value, { mode: 'pin', windows: { track: { firstOffset: 51, lastOffset: 51 } } });
  assert.deepEqual(held.options, { drill: 'popout', ownership: 'roster', measure: 'planes', show: 'breakdown' });
  assert.equal(held.contentScale, 125);
  assert.deepEqual(pinPartsRace(source, track, 'country with , separators', gatedTime({ mode: 'follow' }), 'Types').time.value, { mode: 'follow' });
  assert.deepEqual(pinPartsRace({ ...source, time: { mode: 'campaign' } }, track, 'country with , separators', gatedTime({ mode: 'follow' }), 'Types').time.value, { mode: 'follow' });
});

test('native force binding and restored pins follow aliases, refuse reused tags, gaps and conflicting holders', () => {
  const item = (tag: string, slot: number, countryId: number, firstHour: number, lastHour: number, total: number) => ({ tag,
    identities: [{ firstHour, lastHour, slot, countryId }], segments: [{ firstHour, lastHour }], membership: null,
    runs: [{ firstHour, lastHour, startsSegment: true, total, values: [total] }] });
  const original = item('OLD', 7, 101, 0, 4, 5), renamed = item('NEW', 7, 101, 5, 20, 7);
  const later = item('OLD', 8, 102, 10, 20, 99);
  original.identities.push(...later.identities); original.segments.push(...later.segments); original.runs.push(...later.runs);
  const reply = validateNativePartsRaceReply({ branchId: 'branch', warnings: [],
    parts: [{ key: 'mod force part', name: 'Force part', unresolved: false }], series: [original, renamed] });
  const native = partsRaceDrillData(partsRaceModel(reply), reply.series, () => null);
  const selected = countryPartsSelection(reply.series, 'OLD', 1)!;
  const binding = bindCountryParts(native, selected.tag, selected.hour, selected.key);
  assert.equal(binding.tagAt(12), 'NEW');
  assert.equal(binding.data.at('OLD', 12)?.total, 7);
  assert.equal(runAt(binding.data.bands('OLD', { firstHour: 0, lastHour: 20 })[0], 12)?.value, 7);
  assert.equal(countryPartsSelection(reply.series, 'OLD', 6), null);
  assert.equal(bindCountryParts(native, 'OLD', 1, '[7,999]').data.at('OLD', 12), null);
  const source: WidgetSpec = { id: 'source', kind: 'parts-drill-test', title: 'Force', configVersion: 1,
    trackIds: ['track'], countries: { kind: 'tags', tags: ['OLD'] }, time: { mode: 'campaign' } };
  const track = { id: 'track', branch: { id: 'branch', firstHour: 0, lastHour: 20 }, source: { metadata: { campaignKey: 'campaign' } } } as BoundTrack;
  const [slot, countryId] = JSON.parse(selected!.key) as [number, number];
  const held = heldCountry({ tag: selected!.tag, origin: selected!.hour, slot, countryId });
  const pin = persistedSpec(pinNativeCountryParts(source, track, held, gatedTime({ mode: 'pin', windows: { track: { firstOffset: 12, lastOffset: 12 } } }), 'Parts'));
  assert.deepEqual(savedCountryPartsSelection(pin, track), selected);
  assert.equal(pin.options?.drillOriginHour, 1);
  assert.equal(pinNativeCountryParts(source, track, held, gatedTime({ mode: 'follow' }), 'Parts').time.value.mode, 'follow');
  assert.equal(savedCountryPartsSelection({ ...pin, options: { ...pin.options, holder: 'malformed' } }, track), null);
  assert.equal(savedCountryPartsSelection(pin, { ...track, branch: { ...track.branch, id: 'another' } }), null);
  assert.equal(savedCountryPartsSelection({ ...pin, options: { show: 'breakdown' } }, track), undefined);
  renamed.identities = [{ firstHour: 5, lastHour: 8, slot: 7, countryId: 101 }, { firstHour: 10, lastHour: 20, slot: 7, countryId: 101 }];
  const conflict = item('CONFLICT', 7, 101, 12, 14, 8);
  const ambiguous = validateNativePartsRaceReply({ ...reply, series: [original, renamed, conflict] });
  const broken = bindCountryParts(partsRaceDrillData(partsRaceModel(ambiguous), ambiguous.series, () => null), 'OLD', 1, selected.key);
  const bands = broken.data.bands('OLD', { firstHour: 0, lastHour: 20 });
  for (const hour of [9, 12, 13, 14]) {
    assert.equal(broken.tagAt(hour), null);
    assert.equal(broken.data.at('OLD', hour), null);
    assert.ok(bands.every(runs => runAt(runs, hour) === null));
  }
  assert.equal(broken.data.at('OLD', 15)?.total, 7);
  assert.throws(() => validateNativePartsRaceReply({ ...reply, series: [{ ...original, identities: undefined }] }), /malformed/);
});
