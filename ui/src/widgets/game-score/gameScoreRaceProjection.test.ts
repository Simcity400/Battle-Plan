import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { assertBarsModelValid, deriveBarsAudit, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { GameScoreReply } from './gameScoreData.ts';
import { gameScoreRaceProjection, type GameScoreRaceOptions } from './gameScoreRaceProjection.ts';

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

function fakeLive(values: Record<string, number>): LiveValues<number | bigint | string> {
  return {
    get: key => values[key],
    raw: key => (key in values ? BigInt(Math.round(values[key]!)) : undefined),
    missing: () => 'Not recorded',
  };
}

const KNOWN = { missing: null };
const BASE_OPTS: GameScoreRaceOptions = {
  all: false,
  chosenTags: [],
  holders: null,
  membership: null,
  unobserved: [],
  top: 3,
  by: 'total',
  split: true,
};

function makeReply(overrides?: Partial<GameScoreReply>): GameScoreReply {
  return {
    rows: [
      { tag: 'GER', countryId: 1, total: KNOWN, army: KNOWN, navy: KNOWN, air: KNOWN, industry: KNOWN, land: KNOWN },
      { tag: 'USA', countryId: 2, total: KNOWN, army: KNOWN, navy: KNOWN, air: KNOWN, industry: KNOWN, land: KNOWN },
      { tag: 'ENG', countryId: 3, total: KNOWN, army: KNOWN, navy: KNOWN, air: KNOWN, industry: KNOWN, land: KNOWN },
    ],
    multipliers: [],
    hints: {},
    missing: null,
    top: { tags: ['USA', 'GER', 'ENG'], missing: null },
    warnings: [],
    ...overrides,
  };
}

test('game-score race preserves Rust top order, builds split segments in follow and pinned modes, and matches exact audit', () => {
  const identity = fakeIdentity({
    USA: { name: 'United States', color: '#3b82f6' },
    GER: { name: 'German Reich', color: '#6b7280' },
    ENG: { name: 'United Kingdom', color: '#e11d48' },
  });
  const live = fakeLive({
    [liveKey('USA', 'total')]: 4000,
    [liveKey('USA', 'army')]: 1500,
    [liveKey('USA', 'navy')]: 1500,
    [liveKey('USA', 'air')]: 1000,
    [liveKey('GER', 'total')]: 3500,
    [liveKey('GER', 'army')]: 2000,
    [liveKey('GER', 'navy')]: 500,
    [liveKey('GER', 'air')]: 1000,
    [liveKey('ENG', 'total')]: 2500,
    [liveKey('ENG', 'army')]: 500,
    [liveKey('ENG', 'navy')]: 1500,
    [liveKey('ENG', 'air')]: 500,
  });

  const view = createBoundedReadView({
    hour: displayedHour(500),
    panels: new Map([['main', makeReply()]]),
    live: new Map([['main', live]]),
    identity,
  });

  const model = gameScoreRaceProjection.project(view, BASE_OPTS);
  assertBarsModelValid(model, 500);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.label, r.numeric]), [
    ['USA', 'United States', 4000],
    ['GER', 'German Reich', 3500],
    ['ENG', 'United Kingdom', 2500],
  ]);
  assert.deepEqual(rows[0]!.segments?.map(s => [s.key, Math.round(s.share * 100)]), [
    ['army', 38],
    ['navy', 38],
    ['air', 25],
  ]);
  assert.ok(deriveBarsAudit(model).length > 0);
  assert.throws(() => view.who('country', 'USA', displayedHour(501)), FutureDataError);
});

test('game-score race withholds top ranking with the Rust missing reason when any country score is unrecorded', () => {
  const view = createBoundedReadView({
    hour: displayedHour(400),
    panels: new Map([['main', makeReply({
      top: { tags: [], missing: 'The top 3 by total score needs every country’s total score, and 1 of 3 were not recorded at this hour' },
    })]]),
    live: new Map([['main', fakeLive({})]]),
  });

  const model = gameScoreRaceProjection.project(view, BASE_OPTS);
  assertBarsModelValid(model, 400);
  assert.equal(model.rows.length, 0);
  assert.equal(model.emptyKind, 'unavailable');
  assert.match(model.empty ?? '', /1 of 3 were not recorded/);
});

test('game-score race shows explicit countries with missing scores as notRecorded sorted last', () => {
  const identity = fakeIdentity({
    GER: { name: 'German Reich', color: '#6b7280' },
    ENG: { name: 'United Kingdom', color: '#e11d48' },
  });
  const reply = makeReply({
    rows: [
      { tag: 'GER', countryId: 1, total: { missing: 'Score not recorded' }, army: KNOWN, navy: KNOWN, air: KNOWN, industry: KNOWN, land: KNOWN },
      { tag: 'ENG', countryId: 3, total: KNOWN, army: KNOWN, navy: KNOWN, air: KNOWN, industry: KNOWN, land: KNOWN },
    ],
    top: undefined,
  });
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', reply]]),
    live: new Map([['main', fakeLive({ [liveKey('ENG', 'total')]: 1200 })]]),
    identity,
  });

  const model = gameScoreRaceProjection.project(view, { ...BASE_OPTS, top: null, chosenTags: ['GER', 'ENG'], split: false });
  assertBarsModelValid(model, 300);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.equal(rows[0]!.key, 'ENG');
  assert.equal(rows[0]!.numeric, 1200);
  assert.equal(rows[1]!.key, 'GER');
  assert.equal(rows[1]!.numeric, null);
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
});
