import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { assertBarsModelValid, formatBarsValueField, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import { BARS_VIEWS } from '../manifest/bars.ts';
import { worldTensionRaceDeclaration, worldTensionRaceProjection, type WorldTensionRaceOptions } from './worldTensionRaceProjection.ts';

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

const BASE_OPTS: WorldTensionRaceOptions = { surface: '#1e1e1e',
  top: 10,
  all: false,
  group: false,
};

const reading = (country: string, hour: number, valueRaw: string | null, missing: string | null = null) => ({
  country, hour, run: valueRaw === null ? null : { valueRaw, firstHour: hour - 10, lastHour: hour + 10 }, missing,
});

test('world-tension race keeps the product order, keys each row by its recorded slot, and formats fixed-5 percentages', () => {
  const identity = fakeIdentity({
    [countryKey(7)]: { name: 'German Reich', color: '#6b7280' },
    [countryKey(31)]: { name: 'Imperial Japan', color: '#e5b89b' },
  });
  const view = createBoundedReadView({
    hour: displayedHour(500),
    panels: new Map([['main', {
      branchId: '1', hour: 500, from: 500, until: 500, refused: null,
      measures: [{ name: 'contribution', decimals: 5 }],
      rows: [
        { country: 'GER', slot: 7, readings: [reading('GER', 500, '4525000')] },
        { country: 'JAP', slot: 31, readings: [reading('JAP', 500, '1800000')] },
        { country: 9, slot: 12, unnamed: 'The country holding this slot is not resolved at this hour', readings: [reading('GER', 500, '900000')] },
      ],
    }]]),
    identity,
  });
  const model = worldTensionRaceProjection.project(view, BASE_OPTS);
  assertBarsModelValid(model, 500);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.label, r.numeric, formatBarsValueField(r.value!).text]), [
    [countryKey(7), { kind: 'known', value: 'German Reich' }, 45.25, '45.25%'],
    [countryKey(31), { kind: 'known', value: 'Imperial Japan' }, 18, '18%'],
    [countryKey(12), { kind: 'notRecorded', reason: 'The country holding this slot is not resolved at this hour' }, 9, '9%'],
  ]);
  assert.deepEqual(rows[2]!.tooltip?.items?.[0], { label: 'Country', missingReason: 'The country holding this slot is not resolved at this hour' });
  assert.throws(() => view.who('country', 'GER', displayedHour(501)), FutureDataError);
});

test('a group race follows the group and shows its ranked members', async () => {
  const entry = BARS_VIEWS.find(view => view.kind === 'world-tension' && view.view === 'race')!;
  const spec = await entry.loadSpec();
  assert.equal(spec.projection, worldTensionRaceProjection);
  const declaration = spec.declaration as unknown as typeof worldTensionRaceDeclaration;
  const ctx = {
    track: { source: { id: 's', revision: 1 }, axisOffset: 0 },
    branch: { id: '1' },
    options: {},
    countries: { top: null, all: false, every: ['GER', 'ITA'], tags: ['GER', 'ITA'], membership: { GER: [[0, 1000]], ITA: [[600, 1000]] }, holders: null, unobserved: [] },
    widget: { countries: { kind: 'group', group: { type: 'faction', id: '42' }, membership: 'hour', aggregate: false } },
  } as unknown as Parameters<typeof declaration.sources>[0];
  const plan = declaration.sources(ctx, { following: true, staticSpan: null });
  assert.deepEqual(plan.entries.map(e => e.name), ['main'], 'one read: the rank, never a followed member list beside it');
  const read = plan.entries[0]!.read;
  assert.ok(read.shape === 'point' && read.followQuery);
  assert.deepEqual(read.followQuery, { panel: 'series-rank', ...read.params });
  const asked = read.params!;
  assert.deepEqual([asked.group, asked.countries, asked.top], [{ type: 'faction', id: '42', membership: 'hour' }, undefined, undefined]);
  const reply = {
    branchId: '1', hour: 500, from: 321, until: 599, refused: null,
    measures: [{ name: 'contribution', decimals: 5 }],
    rows: [{ country: 'GER', slot: 7, readings: [reading('GER', 500, '4525000')] }],
  };
  const options = declaration.projectionOptions(ctx, { extent: [0, 0], surface: '#1e1e1e' });
  const view = createBoundedReadView({ hour: displayedHour(500), panels: new Map([['main', reply]]), identity: fakeIdentity({}) });
  assert.deepEqual(worldTensionRaceProjection.project(view, options).rows.map(row => row.key), [countryKey(7)]);
  const empty = createBoundedReadView({ hour: displayedHour(500), panels: new Map([['main', { ...reply, rows: [] }]]), identity: fakeIdentity({}) });
  const none = worldTensionRaceProjection.project(empty, options);
  assert.deepEqual([none.emptyKind, none.empty], ['empty', 'No country of this group at this date.']);
});

test('world-tension race keeps missing readings as notRecorded with null numeric', () => {
  const identity = fakeIdentity({
    GER: { name: 'German Reich', color: '#6b7280' },
    ITA: { name: 'Italy', color: '#4d7c0f' },
  });
  const view = createBoundedReadView({
    hour: displayedHour(200),
    panels: new Map([['main', {
      branchId: '1',
      hour: 200,
      from: 200,
      until: 200,
      refused: 'Threat not recorded',
      measures: [{ name: 'contribution', decimals: 5 }],
      rows: [
        { country: 'GER', readings: [{ country: 'GER', hour: 200, run: { valueRaw: '0', firstHour: 190, lastHour: 210 }, missing: null }] },
        { country: 'ITA', readings: [{ country: 'ITA', hour: 200, run: null, missing: 'Threat not recorded' }] },
      ],
    }]]),
    identity,
  });

  const model = worldTensionRaceProjection.project(view, { surface: '#1e1e1e', top: null, all: false, group: false });
  assertBarsModelValid(model, 200);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.equal(rows[0]!.key, 'GER');
  assert.equal(rows[0]!.value?.value.kind, 'zero');
  assert.equal(rows[1]!.key, 'ITA');
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
  assert.equal(rows[1]!.numeric, null);
});
