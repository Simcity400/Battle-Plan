import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { assertBarsModelValid, deriveBarsAudit, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { MinefieldsReply } from '../../generated/dashboard';
import { minefieldsProjection } from './minefieldsProjection.ts';

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
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };
}

const REPLY: MinefieldsReply = {
  regions: [
    { region: 3, owners: [{ tag: 'ENG' }, { tag: 'GER' }], unknownOwners: 0 },
    { region: 5, owners: [], unknownOwners: 2 },
  ],
  unknown: [],
  choices: [
    { id: '3', name: 'North Sea' },
    { id: '5', name: 'Baltic Sea' },
  ],
  scale: 100000,
  warnings: [],
};

test('minefields projection projects owner shares from Rust live totals and labels mines clearly', () => {
  const identity = fakeIdentity({
    ENG: { name: 'United Kingdom', color: '#e11d48' },
    GER: { name: 'German Reich', color: '#6b7280' },
  });
  const live = valuesOf(new Map([
    [liveKey('3', 'ENG'), 900000],
    [liveKey('3', 'GER'), 500000],
    [liveKey('3', 'total'), 1400000],
  ]), 'Not recorded at this hour');

  const view = createBoundedReadView({
    hour: displayedHour(350),
    panels: new Map([['main', REPLY]]),
    live: new Map([['main', live]]),
    identity,
  });

  const model = minefieldsProjection.project(view, { order: 'mines' });
  assertBarsModelValid(model, 350);
  assert.equal(model.form, 'share');

  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.label]), [['3', 'North Sea'], ['5', 'Baltic Sea']]);
  assert.equal(rows[0]!.readout?.primary, '14 mines');
  assert.deepEqual(rows[0]!.segments?.map(s => [s.key, s.label, Math.round(s.share * 100)]), [
    ['ENG', 'United Kingdom', 64],
    ['GER', 'German Reich', 36],
  ]);

  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
  assert.equal(rows[1]!.readout?.primary, 'Not recorded');

  const audit = deriveBarsAudit(model);
  assert.ok(audit.some(entry => entry.row === '3' && entry.key === 'segment:ENG'));
  assert.ok(audit.some(entry => entry.row === '5' && entry.key === 'value' && entry.value?.startsWith('["notRecorded"')));
});

test('minefields projection marks truncated total as notRecorded rather than zero and rejects future hours', () => {
  const live = valuesOf(new Map([[liveKey('3', 'ENG'), 400000]]), 'Naval regional mines truncated');
  const view = createBoundedReadView({
    hour: displayedHour(100),
    panels: new Map([['main', REPLY]]),
    live: new Map([['main', live]]),
  });

  const model = minefieldsProjection.project(view, { order: 'mines' });
  assertBarsModelValid(model, 100);
  const first = model.rows[0] as BarsDataRow;
  assert.equal(first.value?.value.kind, 'notRecorded');
  assert.equal(first.segments?.length, 0);
  assert.equal(first.readout?.primary, 'Not recorded');
  assert.throws(() => view.who('country', 'ENG', displayedHour(101)), FutureDataError);
});
