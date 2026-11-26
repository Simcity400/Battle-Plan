import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { RaidTallyAtReply } from '../../generated/dashboard';
import { raidTallyRaceDeclaration, raidTallyRaceProjection, type RaidTallyRaceOptions } from './raidTallyRaceProjection.ts';

function fakeIdentity(names: Record<string, { name: string; color: string; tag?: string }>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const hit = kind === 'country' ? names[key] : undefined;
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: hit?.tag ?? key },
        name: hit ? { status: 'ready', value: hit.name } : { status: 'absent', reason: 'Name not recorded' },
        colour: hit ? { status: 'ready', value: hit.color } : { status: 'absent', reason: 'Colour not recorded' },
        flagChoice: { status: 'absent', reason: 'Flag not recorded' },
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };
}

const OPTS: RaidTallyRaceOptions = { surface: '#1e1e1e',
  metric: 'both',
  group: false,
  top: 10,
  all: false,
};

test('raidTallyRaceProjection sends group to Rust, keys by slot, never claims empty when unattributed raids exist, and keeps zeros and missing rows', () => {
  const emptyModel = raidTallyRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(300), panels: new Map(), identity: fakeIdentity({}) }),
    OPTS,
  );
  assertBarsModelValid(emptyModel, 300);
  assert.equal(emptyModel.rows.length, 0);

  const unattributedOnly: RaidTallyAtReply = {
    branchId: '1',
    hour: 300,
    firstHour: 1,
    lastHour: 500,
    metric: 'both',
    available: true,
    unavailable: null,
    rows: [],
    unattributed: 2,
    unattributedLaunched: 2,
    unattributedSucceeded: 1,
    missing: null,
    warnings: [],
  };
  const unattributedModel = raidTallyRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(300), panels: new Map([['main', unattributedOnly]]), identity: fakeIdentity({}) }),
    { ...OPTS, top: null, all: true },
  );
  assertBarsModelValid(unattributedModel, 300);
  assert.equal(unattributedModel.emptyKind, 'unavailable', 'never claims no raids when unattributed raids exist');
  assert.match(unattributedModel.empty ?? '', /2 raid\(s\) recorded up to this date with an unrecorded raider/);

  const reply: RaidTallyAtReply = {
    branchId: '1',
    hour: 300,
    firstHour: 200,
    lastHour: 400,
    metric: 'both',
    available: true,
    unavailable: null,
    rows: [
      { country: 7, tag: 'GER', launched: 5, succeeded: 3, missing: null },
      { country: 9, tag: 'GER', launched: 0, succeeded: 0, missing: null },
      { country: 14, tag: 'FRA', launched: 0, succeeded: 0, missing: 'Raids were not recorded at this hour' },
    ],
    unattributed: 0,
    unattributedLaunched: 0,
    unattributedSucceeded: 0,
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      [countryKey(7)]: { name: 'German Reich', color: '#6b7280', tag: 'GER' },
      [countryKey(9)]: { name: 'West Germany', color: '#4b5563', tag: 'GER' },
      [countryKey(14)]: { name: 'France', color: '#2563eb', tag: 'FRA' },
    }),
  });
  const model = raidTallyRaceProjection.project(view, { ...OPTS, top: null });
  assertBarsModelValid(model, 300);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => r.key), [countryKey(7), countryKey(9), countryKey(14)], 'rows keyed by slot');
  assert.equal(rows[0]!.numeric, 5);
  assert.equal(rows[0]!.segments?.length, 2);
  assert.equal(rows[0]!.readout?.primary, '5 · 3');
  assert.equal(rows[1]!.value?.value.kind, 'zero', 'recorded zero preserved');
  assert.equal(rows[2]!.value?.value.kind, 'notRecorded');

  const plan = raidTallyRaceDeclaration.sources({
    widget: { id: 'w', kind: 'raid-tally', trackIds: ['t'], countries: { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }, time: { mode: 'follow' }, options: { view: 'race' } },
    track: { id: 't', source: { id: 's', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 100 }, axisOffset: 0 },
    branch: { id: '1', firstHour: 0, lastHour: 100 },
    options: { view: 'race' },
    settings: {},
    countries: { tags: [], every: [], top: null, all: false, holders: null, membership: null, unobserved: [], reason: null },
  } as never, { following: true, staticSpan: null });
  const read = plan.entries[0]!.read as unknown as { params: Record<string, unknown>; buildExactParams: (base: Record<string, unknown>, hour: number) => Record<string, unknown> | null };
  const exact = read.buildExactParams(read.params, 300);
  assert.deepEqual(exact?.group, { type: 'humans', membership: 'hour' }, 'group selection sent to Rust');
});
