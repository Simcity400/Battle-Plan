import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, type BarsDataRow } from './model.ts';
import { createBoundedReadView, displayedHour } from '../boundedView.ts';
import type { IdentityProvider } from '../plan.ts';
import type { ForceAtReply } from '../../generated/dashboard';
import { projectForceAtMix } from './forceAtMix.ts';

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

test('projectForceAtMix builds share rows with legend and marks unreconciled parts unavailable instead of folding', () => {
  const reply: ForceAtReply = {
    parts: [
      { key: 'ships', name: 'Ships', unresolved: false },
      { key: 'convoys', name: 'Convoys', unresolved: false },
    ],
    rows: [
      { country: 'ENG', slot: null, tag: 'ENG', total: 100, missing: null, values: [25, 75], partsMissing: null },
      { country: 'GER', slot: null, tag: 'GER', total: 50, missing: null, values: [30, null], partsMissing: null },
      { country: 'ITA', slot: null, tag: 'ITA', total: 40, missing: null, values: [12, 20], partsMissing: 'Part breakdown does not reconcile with the recorded total at this hour' },
    ],
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      ENG: { name: 'United Kingdom', color: '#e11d48' },
      GER: { name: 'German Reich', color: '#6b7280' },
    }),
  });
  const model = projectForceAtMix(view, reply, { surface: '#1e1e1e', top: 10, all: false, group: false }, {
    decimals: 0,
    valueFormat: { decimals: 0 },
    valueLabel: 'Ships and convoys lost',
    unit: 'Ships & convoys',
    partsLabel: 'Ships and convoys',
    emptyMessage: 'No naval losses.',
  });
  assertBarsModelValid(model, 300);
  assert.equal(model.form, 'share');
  assert.equal(model.scaleUnit, 'Ships & convoys');
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows[0]!.segments?.map(s => [s.label, s.share]), [['Ships', 0.25], ['Convoys', 0.75]]);
  assert.equal(rows[0]!.readout?.primary, '100');
  assert.equal(rows[1]!.segments?.length, 0);
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
  assert.deepEqual(model.legend?.map(l => l.label), ['Ships', 'Convoys']);
  assert.equal(rows[2]!.segments?.length, 0, 'parts that do not reconcile are never drawn as shares');
  assert.deepEqual(rows[2]!.tooltip?.items?.slice(1), [
    { label: 'Ships and convoys', missingReason: 'Part breakdown does not reconcile with the recorded total at this hour' },
    { label: 'Ships', value: '12' },
    { label: 'Convoys', value: '20' },
  ], 'each witnessed count is still shown');
});

test('a part keeps its colour when the parts beside it change (convoys alone or beside ships)', () => {
  const view = createBoundedReadView({ hour: displayedHour(300), panels: new Map(), identity: fakeIdentity({}) });
  const project = (parts: ForceAtReply['parts'], values: number[]) => projectForceAtMix(view, {
    parts, rows: [{ country: 'ENG', slot: null, tag: 'ENG', total: values.reduce((a, b) => a + b, 0), missing: null, values, partsMissing: null }], missing: null, warnings: [],
  }, { top: 10, all: false, group: false }, { decimals: 0, valueFormat: { decimals: 0 }, valueLabel: 'Lost', unit: 'Lost', partsLabel: 'Ships and convoys (colour test)', emptyMessage: 'None.' });
  const colours = (model: ReturnType<typeof project>) => new Map(model.legend?.map(item => [item.key, item.color]));
  const both = colours(project([{ key: 'ships', name: 'Ships', unresolved: false }, { key: 'convoys', name: 'Convoys', unresolved: false }], [2, 3]));
  const alone = colours(project([{ key: 'convoys', name: 'Convoys', unresolved: false }], [3]));
  const added = colours(project([{ key: 'aaa', name: 'Earlier kind', unresolved: false }, { key: 'ships', name: 'Ships', unresolved: false }, { key: 'convoys', name: 'Convoys', unresolved: false }], [1, 2, 3]));
  const reversed = colours(project([{ key: 'convoys', name: 'Convoys', unresolved: false }, { key: 'ships', name: 'Ships', unresolved: false }], [3, 2]));
  assert.deepEqual([reversed.get('ships'), reversed.get('convoys')], [both.get('ships'), both.get('convoys')], 'the order a reply lists its parts in, or a view met them in, never changes a colour');
  assert.equal(alone.get('convoys'), both.get('convoys'));
  assert.equal(added.get('ships'), both.get('ships'), 'an earlier-sorting part added later does not recolour one on screen');
  assert.notEqual(added.get('aaa'), added.get('ships'));
});

test('a total not known beside a known part keeps the known part in its tooltip (ships known, convoys not)', () => {
  const reply: ForceAtReply = {
    parts: [{ key: 'ships', name: 'Ships', unresolved: false }, { key: 'convoys', name: 'Convoys', unresolved: false }],
    rows: [{ country: 'ENG', slot: null, tag: 'ENG', total: null, missing: 'Convoys sunk are not known at this hour', values: [3, null], partsMissing: null }],
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({ hour: displayedHour(300), panels: new Map([['main', reply]]), identity: fakeIdentity({}) });
  const model = projectForceAtMix(view, reply, { top: null, all: false, group: false }, { decimals: 0, valueFormat: { decimals: 0 }, valueLabel: 'Lost', unit: 'Lost', partsLabel: 'Ships and convoys', emptyMessage: 'None.' });
  const row = model.rows.find((r): r is BarsDataRow => r.kind === 'data')!;
  assert.deepEqual(row.tooltip?.items?.slice(1), [{ label: 'Ships', value: '3' }, { label: 'Convoys', missingReason: 'Not recorded at this hour' }]);
});
