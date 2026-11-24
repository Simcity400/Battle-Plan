import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { NavalLossesAtReply } from '../../generated/dashboard';
import { navalLossesRaceProjection, type NavalLossesRaceOptions } from './navalLossesRaceProjection.ts';

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

const BASE_OPTS: NavalLossesRaceOptions = {
  kind: 'both',
  attribution: 'lost',
  period: 'month',
  split: true,
  candidates: [],
  holders: null,
  membership: null,
  unobserved: [],
  top: 10,
  all: false,
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
      { country: 'ENG', tag: 'ENG', total: 40, missing: null, values: [10, 30], partsMissing: null },
      { country: 'GER', tag: 'GER', total: 15, missing: null, values: [12, 3], partsMissing: null },
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
