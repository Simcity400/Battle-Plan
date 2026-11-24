import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, deriveBarsAudit, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { NuclearArsenalReply } from '../../generated/dashboard';
import { nuclearArsenalRaceProjection, type NuclearArsenalRaceOptions } from './nuclearArsenalRaceProjection.ts';

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

const REPLY: NuclearArsenalReply = {
  types: [
    { ordinal: 0, token: 'nuclear_bomb', name: 'Nuclear Bomb' },
    { ordinal: 1, token: 'thermonuclear_bomb', name: 'Thermonuclear Bomb' },
  ],
  countries: [
    { tag: 'USA', countryId: 1, total: 10, counts: [{ ordinal: 0, value: 6, ready: 2 }, { ordinal: 1, value: 4, ready: 0 }] },
    { tag: 'SOV', countryId: 2, total: 3, counts: [{ ordinal: 0, value: 3, ready: 0 }] },
  ],
  missing: null,
  warnings: [],
};

const BASE_OPTS: NuclearArsenalRaceOptions = {
  all: false,
  chosenTags: [],
  holders: null,
  membership: null,
  unobserved: [],
  top: 10,
  split: true,
};

test('nuclear-arsenal race preserves Rust top order, builds weapon-type segments, and audits cleanly', () => {
  const view = createBoundedReadView({
    hour: displayedHour(500),
    panels: new Map([['main', REPLY]]),
    identity: fakeIdentity({
      USA: { name: 'United States', color: '#3b82f6' },
      SOV: { name: 'Soviet Union', color: '#dc2626' },
    }),
  });
  const model = nuclearArsenalRaceProjection.project(view, BASE_OPTS);
  assertBarsModelValid(model, 500);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.label, r.numeric]), [
    ['USA', 'United States', 10],
    ['SOV', 'Soviet Union', 3],
  ]);
  assert.deepEqual(rows[0]!.segments?.map(s => [s.key, s.label, Math.round(s.share * 100)]), [
    ['0', 'Nuclear Bomb', 60],
    ['1', 'Thermonuclear Bomb', 40],
  ]);
  assert.ok(deriveBarsAudit(model).length > 0);
  assert.throws(() => view.who('country', 'USA', displayedHour(501)), FutureDataError);
});

test('nuclear-arsenal race surfaces missing refusal as unavailable and empty stockpile as empty', () => {
  const refused = nuclearArsenalRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(100), panels: new Map([['main', { ...REPLY, countries: [], missing: 'Nukes not recorded' }]]) }),
    BASE_OPTS,
  );
  assert.equal(refused.emptyKind, 'unavailable');
  assert.equal(refused.empty, 'Nukes not recorded');

  const empty = nuclearArsenalRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(100), panels: new Map([['main', { ...REPLY, countries: [] }]]) }),
    BASE_OPTS,
  );
  assert.equal(empty.emptyKind, 'empty');
  assert.equal(empty.rows.length, 0);
});
