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
      { country: 'ENG', tag: 'ENG', total: 100, missing: null, values: [25, 75], partsMissing: null },
      { country: 'GER', tag: 'GER', total: 50, missing: null, values: [30, null], partsMissing: null },
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
  const model = projectForceAtMix(view, reply, { top: 10, all: false }, {
    decimals: 0,
    valueFormat: { decimals: 0 },
    valueLabel: 'Ships and convoys lost',
    partsLabel: 'Ships and convoys',
    emptyMessage: 'No naval losses.',
  });
  assertBarsModelValid(model, 300);
  assert.equal(model.form, 'share');
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows[0]!.segments?.map(s => [s.label, s.share]), [['Ships', 0.25], ['Convoys', 0.75]]);
  assert.equal(rows[0]!.readout?.primary, '100');
  assert.equal(rows[1]!.segments?.length, 0);
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
  assert.deepEqual(model.legend?.map(l => l.label), ['Ships', 'Convoys']);
});
