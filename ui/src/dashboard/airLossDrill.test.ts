import { gatedTime, heldCountry } from '../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { registerWidgets } from './definitions.ts';
import type { BoundTrack, WidgetSpec } from './model';
import { airLossParts, ledgerCountryAt, ledgerShare, militaryAirIndex, validateMilitaryAirReply } from './militaryAirLedger.ts';
import { airLossDrillActor, airLossDrillActors, airLossDrillHistory, pinAirLossDrill } from './airLossDrill.ts';
import { runAt } from '../charts/series.ts';
import { definition } from '../widgets/air-kills-losses/definition.ts';
import { partOf } from './militaryAirParts.fixture.ts';
import type { Leaf } from './militaryAirParts.fixture';
import type { MilitaryAirLedger } from './militaryAirLedger';

const span = (firstHour: number, lastHour = firstHour) => ({ firstHour, lastHour });
const leaf = (extra: Partial<Leaf>): Leaf => ({ tag: 'renamed', identity: '7:91', dealt: false, opponent: null,
  region: 40001, buildingId: null, buildingToken: null, state: null, cause: 1,
  points: [[11, 16, '300000'], [12, 18, '200000']], ...extra });
const reply = (): MilitaryAirLedger => ({ family: 'airLosses', firstHour: 10, lastHour: 20, available: true,
  observed: [span(10, 16), span(18, 20)], incomplete: [], countries: [
    { tag: 'old tag', identity: '7:91', present: [span(10, 14)], membership: [span(10, 14)] },
    { tag: 'renamed', identity: '7:91', present: [span(15, 16), span(18, 20)], membership: [] },
    { tag: 'old tag', identity: '8:99', present: [span(15, 16), span(18, 20)], membership: [] },
  ], parts: [leaf({}), leaf({ cause: 3, points: [[11, 16, '-100000']] }),
    leaf({ dealt: true, opponent: 'opponent', points: [[11, 16, '700000']] })].map(partOf), regions: [], buildings: [], warnings: [] });

test('full cause histories wait for supplying reads, retain signed corrections, aliases and gaps outside top N', () => {
  const index = militaryAirIndex(validateMilitaryAirReply(reply()));
  const history = airLossDrillHistory(index, '7:91', 10, 20, false);
  const at = (hour: number) => history.map(part => runAt(part.runs, hour)?.value ?? null);
  assert.deepEqual(at(14), [0, 0]);
  assert.deepEqual(at(15), [0, 0], 'native date 11 cannot disclose the future read 16');
  assert.deepEqual(at(16), [3, -1]);
  assert.deepEqual(at(17), [null, null]);
  assert.deepEqual(at(18), [2, -1]);
  assert.equal(history.some(part => part.runs.some(run => run.firstHour === 15 && run.breakBefore)), false,
    'a tag rename cannot break the same native holder history');
  assert.equal(ledgerCountryAt(index, 'old tag', 10, 18, '7:91')?.tag, 'renamed');
  assert.equal(ledgerCountryAt(index, 'old tag', 10, 18, '7:91')?.taken, 100000n);
  assert.deepEqual(airLossParts(ledgerCountryAt(index, 'renamed', 10, 18, '7:91')!, 10, 18).causes.get(3), { taken: -100000n, dealt: 0n });
  assert.deepEqual(airLossDrillHistory(index, '7:91', 10, 20, true).map(part => runAt(part.runs, 18)?.value), [7, 0]);
  assert.deepEqual(airLossDrillActor(airLossDrillActors(index, 16, ['old tag'])[0]), { tag: 'old tag', identity: '8:99' });
  assert.deepEqual(airLossDrillActors(index, 17, ['renamed']), []);
});

test('drawn candidates keep all 5,000 opaque tags and the supplying-hour holder without fallback', () => {
  const data = reply();
  data.countries = Array.from({ length: 5000 }, (_, id) => ({ tag: `authored Ω ${id}, separators|[]`, identity: `${id}:9`, present: [span(10, 16)], membership: [] }));
  data.parts = [];
  const index = militaryAirIndex(validateMilitaryAirReply(data));
  const tags = data.countries.map(country => country.tag);
  const actors = airLossDrillActors(index, 12, [...tags, tags[0]]);
  assert.equal(actors.length, 5000);
  assert.deepEqual(airLossDrillActor(actors[4999]), { tag: tags[4999], identity: '4999:9' });
  assert.deepEqual(airLossDrillActors(index, 12, []), []);
  assert.deepEqual(airLossDrillActors(index, 12, ['not drawn or recorded']), []);
  assert.deepEqual(airLossDrillActors(index, 17, tags), []);
});

const RENAMED = heldCountry({ tag: 'renamed', origin: 12, slot: 7, countryId: 91 });
registerWidgets([definition]);
test('Pin retains the selected holder, filters, exact held offsets and the native range lower bound', () => {
  const track = { id: 'track', axisOffset: 1000, source: { metadata: { campaignKey: 'campaign' } },
    branch: { id: 'branch', firstHour: 10, lastHour: 20 } } as unknown as BoundTrack;
  const widget = { kind: 'air-kills-losses', title: 'Air', trackIds: ['track'], countries: { kind: 'all' }, time: { mode: 'follow' },
    options: { regions: '["40001"]', causes: '["1","3"]', measure: 'dealt' } } as unknown as WidgetSpec;
  const pinned = pinAirLossDrill(widget, track, RENAMED, gatedTime({ mode: 'pin', windows: { track: { firstOffset: 6, lastOffset: 6 } } }));
  assert.deepEqual(pinned.time.value, { mode: 'pin', windows: { track: { firstOffset: 6, lastOffset: 6 } } });
  assert.deepEqual(pinned.countries, { kind: 'tags', tags: ['renamed'] });
  assert.deepEqual(pinned.options, { ...widget.options, show: 'breakdown', holder: JSON.stringify(['campaign', 'branch', 'renamed', '7:91']) });
  assert.deepEqual(pinAirLossDrill(widget, track, RENAMED, gatedTime({ mode: 'follow' })).time.value, widget.time);
  widget.time = { mode: 'campaign' };
  assert.deepEqual(pinAirLossDrill(widget, track, RENAMED, gatedTime({ mode: 'follow' })).time.value, { mode: 'follow' });
  widget.time = { mode: 'pin', windows: { track: { firstOffset: 2, lastOffset: 2 } } };
  assert.deepEqual(pinAirLossDrill(widget, track, RENAMED, gatedTime({ mode: 'follow' })).time.value, { mode: 'follow' });
  widget.time = { mode: 'range', firstOffset: 1, lastOffset: 10 };
  assert.deepEqual(pinAirLossDrill(widget, track, RENAMED, gatedTime({ mode: 'range', firstOffset: 1, lastOffset: 6 })).time.value, { mode: 'range', firstOffset: 1, lastOffset: 6 });
  assert.equal(ledgerShare(-100000n, 200000n), '−50.0%');
  assert.equal(ledgerShare(9007199254740993n, 1n), '900,719,925,474,099,300.0%');
  assert.equal(ledgerShare(0n, 0n), '—');
});
