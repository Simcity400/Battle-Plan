import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { RaidTallyAtReply } from '../../generated/dashboard';
import { raidTallyRaceProjection } from './raidTallyRaceProjection.ts';

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

test('raidTallyRaceProjection handles default Top unpopulated panel, split launched/succeeded bars, and missing selected country', () => {
  const emptyView = createBoundedReadView({
    hour: displayedHour(600),
    panels: new Map(),
    identity: fakeIdentity({}),
  });
  const emptyModel = raidTallyRaceProjection.project(emptyView, {
    metric: 'both',
    top: 10,
    all: false,
  });
  assertBarsModelValid(emptyModel, 600);
  assert.equal(emptyModel.rows.length, 0);

  const reply: RaidTallyAtReply = {
    branchId: '1',
    hour: 600,
    firstHour: 500,
    lastHour: 700,
    metric: 'both',
    available: true,
    unavailable: null,
    rows: [
      { country: 'ENG', tag: 'ENG', launched: 10, succeeded: 6, missing: null },
      { country: 'GER', tag: 'GER', launched: 0, succeeded: 0, missing: 'Not recorded at this hour' },
    ],
    unattributed: 0,
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(600),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      ENG: { name: 'United Kingdom', color: '#e11d48' },
      GER: { name: 'German Reich', color: '#6b7280' },
    }),
  });
  const model = raidTallyRaceProjection.project(view, {
    metric: 'both',
    top: null,
    all: false,
  });
  assertBarsModelValid(model, 600);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.equal(rows.length, 2);
  assert.equal(rows[0]!.numeric, 10);
  assert.equal(rows[0]!.readout?.primary, '10+ \u00b7 6+');
  assert.deepEqual(rows[0]!.segments?.map(s => [s.key, s.share, Boolean(s.hatched)]), [
    ['succeeded', 0.6, false],
    ['other', 0.4, true],
  ]);
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
});
