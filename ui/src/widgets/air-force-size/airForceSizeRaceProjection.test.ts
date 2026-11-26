import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, deriveBarsAudit, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { ForceAtReply } from '../../generated/dashboard';
import { airForceSizeRaceProjection, type AirForceSizeRaceOptions } from './airForceSizeRaceProjection.ts';

function fakeIdentity(names: Record<string, { name: string; color: string }>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const hit = kind === 'country' ? names[key] : undefined;
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: key },
        name: hit ? { status: 'ready', value: hit.name } : { status: 'absent', reason: 'Name not recorded' },
        colour: hit ? { status: 'ready', value: hit.color } : { status: 'absent', reason: 'Colour not recorded' },
        flagChoice: { status: 'absent', reason: 'Flag not recorded' },
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };
}

const BASE_OPTS: AirForceSizeRaceOptions = { surface: '#1e1e1e',
  measure: 'planes',
  ownership: 'roster',
  split: true,
  top: 10,
  all: false,
  group: false,
};

test('air-force-size race builds split aircraft-type segments from air-force-at and withholds unreconciled parts', () => {
  const reply: ForceAtReply = {
    parts: [
      { key: 'small_plane_airframe', name: 'Fighter', unresolved: false },
      { key: 'medium_plane_airframe', name: 'Tactical Bomber', unresolved: false },
    ],
    rows: [
      { country: 'GER', slot: null, tag: 'GER', total: 1000, missing: null, values: [700, 300], partsMissing: null },
      { country: 'ENG', slot: null, tag: 'ENG', total: 800, missing: null, values: [null, null], partsMissing: 'Parts withheld' },
    ],
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      GER: { name: 'German Reich', color: '#6b7280' },
      ENG: { name: 'United Kingdom', color: '#e11d48' },
    }),
  });
  const model = airForceSizeRaceProjection.project(view, BASE_OPTS);
  assertBarsModelValid(model, 300);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.numeric]), [['GER', 1000], ['ENG', 800]]);
  assert.deepEqual(rows[0]!.segments?.map(s => [s.label, Math.round(s.share * 100)]), [['Fighter', 70], ['Tactical Bomber', 30]]);
  assert.equal(rows[1]!.segments, undefined, 'unreconciled parts never fabricate segments');
  assert.ok(deriveBarsAudit(model).length > 0);
});

test('air-force-size race under default Top handles unpopulated panel and preserves explicit missing country', () => {
  const unpopulated = createBoundedReadView({ hour: displayedHour(300), panels: new Map() });
  const pending = airForceSizeRaceProjection.project(unpopulated, { ...BASE_OPTS, split: false, top: 10 });
  assert.equal(pending.rows.length, 0);
  assert.equal(pending.empty, undefined);

  const missingExplicit = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', {
      branchId: '1',
      hour: 300,
      from: 300,
      until: 300,
      refused: 'Air planes not recorded at this hour',
      measures: [{ name: 'rosterPlanes', decimals: 0 }],
      rows: [
        { country: 'GER', readings: [{ country: 'GER', hour: 300, run: { firstHour: 300, lastHour: 300, value: 1200, valueRaw: '1200' }, missing: null }] },
        { country: 'ENG', readings: [{ country: 'ENG', hour: 300, run: null, missing: 'Air planes not recorded at this hour' }] },
      ],
    }]]),
    identity: fakeIdentity({
      GER: { name: 'German Reich', color: '#6b7280' },
      ENG: { name: 'United Kingdom', color: '#e11d48' },
    }),
  });
  const model = airForceSizeRaceProjection.project(missingExplicit, {
    ...BASE_OPTS,
    split: false,
    top: null,
  });
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.value?.value.kind]), [['GER', 'known'], ['ENG', 'notRecorded']]);
});
