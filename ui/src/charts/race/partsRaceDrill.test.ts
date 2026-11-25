import { gatedTime } from '../../test-support/gated.ts';
import assert from 'node:assert/strict';
import type { DrillState } from '../drill/drillState';
import test from 'node:test';
import { registerWidgets } from '../../dashboard/definitions.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { axisAt, regionHour } from '../../dashboard/regionTime.ts';
import { drillAction } from '../drill/drillState.ts';
import { CLOSED_PARTS_RACE, partsRaceDrillActors, pinPartsRace, reducePartsRaceDrill } from './partsRaceDrill.ts';

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
