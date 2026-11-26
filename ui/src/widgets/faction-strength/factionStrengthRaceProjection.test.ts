import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import type { FactionTotalsRaceOptions } from '../../families/bars/factionTotalsRace.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { FactionTotalsAtReply } from '../../generated/dashboard';
import { factionStrengthRaceDeclaration, factionStrengthRaceProjection } from './factionStrengthRaceProjection.ts';

function fakeIdentity(names: Record<string, { name: string; color: string }>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const hit = names[`${kind}:${key}`];
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

const OPTS: FactionTotalsRaceOptions = { surface: '#1e1e1e',
  kind: 'country-factories',
  measure: 'free7',
  measureLabel: 'Factories, available',
  scaleUnit: 'Factories',
  aggregated: false,
  groupLabel: 'Group',
  top: null,
};

test('factionStrengthRaceProjection sends group to Rust, keeps missing factions and recorded zeros, and binds fallback colours by faction id', () => {
  const emptyModel = factionStrengthRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(300), panels: new Map(), identity: fakeIdentity({}) }),
    OPTS,
  );
  assertBarsModelValid(emptyModel, 300);
  assert.equal(emptyModel.rows.length, 0);

  const reply: FactionTotalsAtReply = {
    hour: 300,
    kind: 'country-factories',
    measure: 'free7',
    rows: [
      { id: '10', members: ['GER', 'ITA'], memberCountries: [7, 8], total: 410, missing: null },
      { id: '15', members: ['LUX'], memberCountries: [12], total: 0, missing: null },
      { id: '20', members: ['ENG'], memberCountries: [3], total: null, missing: 'A member\'s value (ENG) is not recorded at this hour' },
    ],
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      'faction:10': { name: 'Axis', color: '#6b7280' },
      'faction:15': { name: 'Benelux', color: '#0ea5e9' },
      'faction:20': { name: 'Allies', color: '#e11d48' },
      [`country:${countryKey(7)}`]: { name: 'German Reich', color: '#6b7280' },
      [`country:${countryKey(8)}`]: { name: 'Italy', color: '#16a34a' },
      [`country:${countryKey(12)}`]: { name: 'Luxembourg', color: '#0ea5e9' },
      [`country:${countryKey(3)}`]: { name: 'United Kingdom', color: '#e11d48' },
    }),
  });
  const model = factionStrengthRaceProjection.project(view, OPTS);
  assertBarsModelValid(model, 300);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => r.key), ['10', '15', '20']);
  assert.equal(rows[0]!.numeric, 410);
  assert.equal(rows[1]!.value?.value.kind, 'zero', 'recorded zero preserved');
  assert.equal(rows[2]!.value?.value.kind, 'notRecorded');

  const plan = factionStrengthRaceDeclaration.sources({
    widget: { id: 'w', kind: 'faction-strength', trackIds: ['t'], countries: { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: true }, time: { mode: 'follow' }, options: { view: 'race' } },
    track: { id: 't', source: { id: 's', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 100 }, axisOffset: 0 },
    branch: { id: '1', firstHour: 0, lastHour: 100 },
    options: { view: 'race' },
    settings: {},
    countries: { tags: [], every: [], top: null, all: false, holders: null, membership: null, unobserved: [], reason: null },
  } as never, { following: true, staticSpan: null });
  const sentParams = (plan.entries[0]!.read as { params: Record<string, unknown> }).params;
  assert.deepEqual(sentParams.group, { type: 'humans', membership: 'hour' }, 'aggregated group sent to Rust');
});
