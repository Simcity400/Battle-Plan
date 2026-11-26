import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { NavalLossesAtReply } from '../../generated/dashboard';
import { countryKey } from '../../dashboard/countryKey.ts';
import { BARS_VIEWS } from '../manifest/bars.ts';
import { navalLossesRaceDeclaration, navalLossesRaceProjection, type NavalLossesRaceOptions } from './navalLossesRaceProjection.ts';

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

const BASE_OPTS: NavalLossesRaceOptions = { surface: '#1e1e1e',
  kind: 'both',
  attribution: 'lost',
  period: 'month',
  split: true,
  top: 10,
  all: false,
  group: false,
};

test('naval-losses race splits ships and convoys from naval-losses-at and surfaces missing baseline as unavailable', () => {
  const reply: NavalLossesAtReply = {
    firstHour: 200,
    lastHour: 250,
    parts: [
      { key: 'ships', name: 'Ships', unresolved: false },
      { key: 'convoys', name: 'Convoys', unresolved: false },
    ],
    rows: [
      { country: 'ENG', slot: null, tag: 'ENG', total: 40, missing: null, values: [10, 30], partsMissing: null },
      { country: 'GER', slot: null, tag: 'GER', total: 15, missing: null, values: [12, 3], partsMissing: null },
    ],
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(220),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      ENG: { name: 'United Kingdom', color: '#e11d48' },
      GER: { name: 'German Reich', color: '#6b7280' },
    }),
  });
  const model = navalLossesRaceProjection.project(view, BASE_OPTS);
  assertBarsModelValid(model, 220);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.numeric]), [['ENG', 40], ['GER', 15]]);
  assert.deepEqual(rows[0]!.segments?.map(s => [s.label, Math.round(s.share * 100)]), [['Ships', 25], ['Convoys', 75]]);

  const missingModel = navalLossesRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(220), panels: new Map([['main', { ...reply, rows: [], missing: 'Tables unread' }]]) }),
    BASE_OPTS,
  );
  assert.equal(missingModel.emptyKind, 'unavailable');
  assert.equal(missingModel.empty, 'Tables unread');
});

test('every race and mix view sends a group whole, and naval losses keys each row by its recorded slot', async () => {
  const ctx = (view: string) => ({
    track: { source: { id: 's', revision: 1 }, axisOffset: 0 },
    branch: { id: '1' },
    options: { view },
    countries: { top: null, all: false, every: ['GER', 'ITA'], tags: ['GER', 'ITA'], membership: { GER: [[0, 1000]], ITA: [[600, 1000]] }, holders: null, unobserved: [] },
    widget: { countries: { kind: 'group', group: { type: 'faction', id: '42' }, membership: 'hour', aggregate: false } },
  }) as unknown as Parameters<typeof navalLossesRaceDeclaration.sources>[0];
  for (const [kind, view] of [['naval-losses', 'race'], ['naval-losses', 'mix'], ['army-size', 'race'], ['army-size', 'mix'], ['navy-size', 'mix'], ['air-force-size', 'mix'], ['nuclear-arsenal', 'race'], ['nuclear-arsenal', 'mix'], ['occupation-reserve-manpower', 'main'], ['game-score', 'race']] as const) {
    const spec = await BARS_VIEWS.find(entry => entry.kind === kind && entry.view === view)!.loadSpec();
    const declaration = spec.declaration as unknown as typeof navalLossesRaceDeclaration;
    const plan = declaration.sources(ctx(view), { following: true, staticSpan: null });
    assert.deepEqual(plan.entries.map(entry => entry.name), ['main'], `${kind} ${view}: one read`);
    const read = plan.entries[0]!.read;
    assert.ok(read.shape === 'point');
    if (read.shape !== 'point') continue;
    const asked = (read.buildExactParams ? read.buildExactParams(read.params!, 500 as never, 'static') : read.params) as { group?: unknown; countries?: readonly unknown[]; top?: unknown };
    assert.deepEqual(asked.group, { type: 'faction', id: '42', membership: 'hour' }, `${kind} ${view}: the group`);
    assert.ok(!asked.countries?.length && asked.top == null, `${kind} ${view}: never its members or a top`);
    assert.equal((declaration.projectionOptions(ctx(view), { extent: [0, 0], surface: '#1e1e1e' }) as { group?: boolean }).group, true);
  }
  const reply: NavalLossesAtReply = {
    firstHour: 321, lastHour: 599, parts: [], missing: null, warnings: [],
    rows: [{ country: 'GER', slot: 7, tag: 'GER', total: 4, missing: null, values: [], partsMissing: null }],
  };
  const read = navalLossesRaceDeclaration.sources(ctx('race'), { following: true, staticSpan: null }).entries[0]!.read;
  assert.ok(read.shape === 'point');
  if (read.shape !== 'point') return;
  assert.deepEqual(read.provenInterval?.(reply, 500), [321, 599], 'asked again at 600, when the membership changes');
  const options = navalLossesRaceDeclaration.projectionOptions(ctx('race'), { extent: [0, 0], surface: '#1e1e1e' });
  const view = createBoundedReadView({ hour: displayedHour(500), panels: new Map([['main', reply]]), identity: fakeIdentity({}) });
  assert.deepEqual(navalLossesRaceProjection.project(view, options).rows.map(row => row.key), [countryKey(7)]);
  const none = navalLossesRaceProjection.project(createBoundedReadView({ hour: displayedHour(500), panels: new Map([['main', { ...reply, rows: [] }]]) }), options);
  assert.deepEqual([none.emptyKind, none.empty], ['empty', 'No country of this group at this date.']);
});
