import assert from 'node:assert/strict';
import test from 'node:test';
import type {
  CountryFactsReply,
  CountryFlagChoice,
  CountryFlagReply,
  CountryGroupReply,
  CountryIdentitiesReply,
  CountryStyleReply,
  PlaceNameRunsReply,
  StrategicRegionsReply,
  LeaderNameRunsReply,
} from '../../generated/dashboard';
import { countryKey } from '../../dashboard/countryKey.ts';
import { barsLabelMissing, barsLabelMuted, barsLabelText, countryRowPresentation } from '../bars/model.ts';
import { presentFlowEndpoint } from '../flow/countryFlow.ts';
import { resolveEntityFromReadView } from '../heat/scale.ts';
import { LOADING_NAME, NAME_LOADING } from '../table/identityCell.ts';
import { countryNameAt, countrySeriesLooks } from '../time-chart/seriesHelpers.ts';
import { countryLook } from '../timeline/helpers.ts';
import { datedIdentity } from './dated.fixture.ts';
import { LaneArtCache } from '../art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider, flagArtHandle, laneArtDemand, readArtDemands, requestDemandedFlags } from '../art/provider.ts';
import { toArtSlotState } from '../art/adapters.ts';
import { NO_ART, artHandleKey, productArt, refusedArt } from '../art/types.ts';
import { RefusedError } from '../../dashboard/queryError.ts';
import { createBoundedReadView, FutureDataError } from '../boundedView.ts';
import type { ArtHandle, DisplayedHour } from '../plan.ts';
import { FLAG_BATCH_MAX, IdentityBroker, MAX_ENTITY_WINDOWS, type IdentityTransport } from './broker.ts';
import { createBrokerIdentityProvider, createSnapshotIdentityProvider, parseEntityKey } from './provider.ts';
import { UNHELD_HEAD_HOUR, countryPickerKeys, pickerDating, pickerHeaderTag, readIdentityAtHeadHour, subscribeIdentityAtHead, type HeldHeadHour } from './react.ts';
import {
  IdentityIndex,
  countrySlotKey,
  countryTagKey,
  designKey,
  factionKey,
  flagChoiceKey,
  leaderKey,
  regionKey,
  identityKeyString,
  stateKey,
  type IdentityScope,
  type RecordedFlagChoice,
  type IdentitySnapshot,
} from './index.ts';

const SCOPE: IdentityScope = { sourceId: 's1', revision: 'r1', branchId: '1', content: 1 };

const FLAG_A: CountryFlagChoice = { keys: ['GER_a', null, null], reasons: [null, null, null] };
const FLAG_B: CountryFlagChoice = { keys: ['GER_b', null, null], reasons: [null, null, null] };

function sampleFacts(): CountryFactsReply {
  return {
    branchId: '1',
    known: [[0, 500]],
    names: ['Early Republic', 'Later Empire', 'Alpha'],
    reasons: ['not a country at this hour'],
    flags: [FLAG_A, FLAG_B],
    countries: [
      {
        tag: 'GER',
        runs: [
          [0, 0, 0x334455, -1, 0],
          [150, 1, 0x667788, -1, 1],
        ],
      },
      {
        tag: 'ALP',
        runs: [[0, 2, 0x112233, -1, 0]],
      },
      {
        tag: 'ANX',
        runs: [
          [0, 0, 0x102030, -1, 0],
          [150, -1, -1, 0, -1],
        ],
      },
    ],
    warnings: [],
  };
}

function sampleIdentities(): CountryIdentitiesReply {
  return {
    branchId: '1',
    known: [{ firstHour: 0, lastHour: 500 }],
    countries: [
      {
        tag: 'GER',
        identities: [{ firstHour: 0, lastHour: 500, slot: 1, countryId: 10 }],
        unresolved: [],
      },
      {
        tag: 'ANX',
        identities: [{ firstHour: 0, lastHour: 149, slot: 2, countryId: 20 }],
        unresolved: [],
      },
    ],
    unknown: [],
    warnings: [],
  };
}

function sampleGroups(): CountryGroupReply {
  return {
    branchId: '1',
    humans: [],
    factions: [
      {
        id: '7',
        name: 'Axis Pact',
        nameReason: null,
        names: [
          { firstHour: 50, lastHour: 149, name: 'Berlin Pact', reason: null, token: null },
          { firstHour: 150, lastHour: 300, name: 'Axis Pact', reason: null, token: null },
        ],
        recordedName: 'AXIS',
        firstHour: 50,
        lastHour: 300,
        members: [{ tag: 'GER', firstHour: 50, lastHour: 300 }],
        leaders: [{ tag: 'GER', firstHour: 50, lastHour: 300 }],
        incomplete: [],
      },
    ],
    wars: [],
    warStarts: [],
    capitulations: [],
    identityChanges: [],
    countryIdentities: [],
    unobserved: { humans: [], factions: [{ firstHour: 350, lastHour: 360 }], wars: [], unattributed: [] },
    warnings: [],
  };
}

const UNUSED_ENTITY_READS: Pick<IdentityTransport, 'loadRegions' | 'loadPlaceNameRuns' | 'loadLeaderNameRuns' | 'loadBattleNames'> = {
  loadBattleNames: () => deferred<never>().promise,
  loadRegions: () => deferred<StrategicRegionsReply>().promise,
  loadPlaceNameRuns: () => deferred<PlaceNameRunsReply>().promise,
  loadLeaderNameRuns: () => deferred<LeaderNameRunsReply>().promise,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

test('held run answers hover at an unrequested hour synchronously with exact validity interval', () => {
  const index = new IdentityIndex(SCOPE, [0, 500]);
  index.setFacts(sampleFacts(), true);
  const at200 = index.resolve(countryTagKey('ALP'), 200);
  assert.equal(at200.kind, 'known');
  assert.deepEqual(at200.name, { kind: 'known', value: 'Alpha' });
  assert.deepEqual(at200.valid, [0, 500]);
  assert.equal(at200.nextBoundary, 501);

  const hoverAt100 = index.resolve(countryTagKey('ALP'), 100);
  assert.equal(hoverAt100.kind, 'known');
  assert.deepEqual(hoverAt100.name, { kind: 'known', value: 'Alpha' });
  assert.deepEqual(hoverAt100.valid, [0, 500]);
});

test('name change at hour 150 gives old name at 149 and new name at 150 simultaneously', () => {
  const index = new IdentityIndex(SCOPE, [0, 500]);
  index.setFacts(sampleFacts(), true);

  const row149 = index.resolve(countryTagKey('GER'), 149);
  const row150 = index.resolve(countryTagKey('GER'), 150);
  assert.equal(row149.kind, 'known');
  assert.equal(row150.kind, 'known');
  assert.deepEqual(row149.name, { kind: 'known', value: 'Early Republic' });
  assert.deepEqual(row149.valid, [0, 149]);
  assert.equal(row149.nextBoundary, 150);
  assert.deepEqual(row149.flagChoice, { kind: 'known', value: FLAG_A });

  assert.deepEqual(row150.name, { kind: 'known', value: 'Later Empire' });
  assert.deepEqual(row150.valid, [150, 500]);
  assert.equal(row150.nextBoundary, 501);
  assert.deepEqual(row150.flagChoice, { kind: 'known', value: FLAG_B });
  assert.equal(index.nextBoundary([countryTagKey('GER')], 149), 150);
});

test('warm identity answers match cold fields across tag changes, annexation, gaps and seeks', () => {
  const identities = sampleIdentities();
  identities.countries.push({
    tag: 'ALP', identities: [{ firstHour: 150, lastHour: 300, slot: 2, countryId: 21 }], unresolved: [],
  });
  identities.unknown.push({ firstHour: 220, lastHour: 225, reason: 'identity gap' });
  const snapshot: IdentitySnapshot = {
    scope: SCOPE, branchBounds: [0, 500],
    facts: { ...sampleFacts(), known: [[0, 99], [110, 500]] }, factsFinal: false,
    identities, groups: sampleGroups(),
    styles: { GER: [{ firstHour: 0, lastHour: 500, name: 'Style', color: null, absent: false, reason: null, warnings: [] }] },
  };
  const source = IdentityIndex.fromSnapshot(snapshot);
  source.setStyleFailure('unknown', 105, new RefusedError('style unavailable'));
  const heldSnapshot = source.exportSnapshot();
  const warm = IdentityIndex.fromSnapshot(heldSnapshot);
  const provider = createSnapshotIdentityProvider(warm);
  const queries: ['country' | 'faction', string][] = [
    ...identities.countries.map(c => ['country', c.tag] as ['country', string]),
    ...identities.countries.flatMap(c => c.identities.flatMap(run => [
      ['country', countryKey(run.slot)] as ['country', string],
      ['country', `holder:${run.slot}:${run.countryId}`] as ['country', string],
    ])),
    ['country', 'unknown'], ['faction', '7'],
  ];
  const edges = [0, 50, 100, 105, 110, 150, 220, 226, 301, 350, 361, 501];
  assert.deepEqual(provider.resolve('country', 'unknown', 105 as DisplayedHour).name, {
    status: 'failed', reason: 'style unavailable', retryable: false,
  });
  const hours = edges.flatMap(h => [h - 1, h, h + 1]);
  for (const hour of [...hours, ...hours.reverse()]) {
    for (const [kind, id] of queries) {
      const key = parseEntityKey(kind, id);
      const cold = IdentityIndex.fromSnapshot(heldSnapshot);
      assert.deepEqual(warm.resolve(key, hour), cold.resolve(key, hour));
      assert.deepEqual(provider.resolve(kind, id, hour as DisplayedHour),
        createSnapshotIdentityProvider(cold).resolve(kind, id, hour as DisplayedHour));
    }
  }
});

test('1,000 repeated country and faction lookups build no new result or field objects after warm-up', () => {
  const index = new IdentityIndex(SCOPE, [0, 500]);
  index.setFacts(sampleFacts());
  index.setIdentities(sampleIdentities());
  index.setGroups(sampleGroups());
  const provider = createSnapshotIdentityProvider(index);
  const seen = new Set<object>();
  function observe(kind: 'country' | 'faction', key: string) {
    const result = provider.resolve(kind, key, 100 as DisplayedHour);
    seen.add(result);
    for (const field of [result.holder, result.tag, result.colour, result.flagChoice, result.name]) {
      if (field) seen.add(field);
    }
    return result;
  }
  const country = observe('country', 'GER');
  const faction = observe('faction', '7');
  const warm = seen.size;
  for (let i = 0; i < 500; i++) {
    observe('country', 'GER');
    observe('faction', '7');
  }
  assert.equal(seen.size - warm, 0);
  assert.equal(country.colour, faction.colour);
  const key = countryTagKey('GER');
  assert.equal(index.resolve(key, 149), index.resolve(key, 100));
  const later = provider.resolve('country', 'GER', 149 as DisplayedHour);
  assert.equal(later, country);
  assert.equal(later.name, country.name);
  assert.equal(later.colour, country.colour);
  assert.equal(provider.resolve('country', 'GER', 149 as DisplayedHour), later);

  index.setEntityRuns(stateKey(3), [{ firstHour: 0, lastHour: 500, name: 'Peak' }]);
  assert.equal(observe('country', 'GER'), country);
  assert.equal(observe('faction', '7'), faction);
  const snapshot = index.exportSnapshot();
  assert.equal(index.exportSnapshot(), snapshot);
  const facts = sampleFacts();
  facts.countries[0]!.runs[0]![2] = 0x123456;
  index.setFacts(facts);
  const changed = observe('country', 'GER');
  assert.notDeepEqual(changed.colour, country.colour);
  assert.equal(observe('faction', '7').colour, changed.colour);
  assert.notEqual(index.exportSnapshot(), snapshot);
  assert.equal(provider.resolve('country', 'GER', 100 as DisplayedHour), changed);
  const other = IdentityIndex.fromSnapshot({ ...snapshot, scope: { ...SCOPE, branchId: '2', sourceId: 'other' } });
  const otherCountry = createSnapshotIdentityProvider(other).resolve('country', 'GER', 100 as DisplayedHour);
  assert.deepEqual(otherCountry, country);
  assert.notEqual(otherCountry, country);
  assert.equal(Object.isFrozen(facts), false);
});

test('the index answers a slot’s holder runs and a key’s runs, and counts a revision only when identities change', () => {
  const index = new IdentityIndex(SCOPE, [0, 500]);
  const before = index.slotRevision();
  assert.equal(index.hasRead('identities'), false);
  index.setIdentities({ branchId: '1', known: [], unknown: [], warnings: [], countries: [
    { tag: 'OLD', identities: [{ firstHour: 0, lastHour: 200, slot: 5, countryId: 90 }], unresolved: [] },
    { tag: 'NEW', identities: [{ firstHour: 150, lastHour: 200, slot: 5, countryId: 40 }], unresolved: [{ firstHour: 201, lastHour: 220 }] },
  ] } as unknown as CountryIdentitiesReply);
  assert.equal(index.hasRead('identities'), true);
  assert.notEqual(index.slotRevision(), before);
  const runs = (list: readonly { tag: string; firstHour: number; lastHour: number; unresolved?: boolean }[]) =>
    list.map(({ tag, firstHour, lastHour, unresolved }) => ({ tag, firstHour, lastHour, ...(unresolved ? { unresolved } : {}) }));
  assert.deepEqual(runs(index.tagRuns(5)), [{ tag: 'OLD', firstHour: 0, lastHour: 149 }, { tag: 'NEW', firstHour: 150, lastHour: 200 }], 'the lowest country id holds the slot at every hour');
  assert.deepEqual(runs(index.runsForKey(countryKey(5))), runs(index.tagRuns(5)));
  assert.deepEqual(runs(index.runsForKey(countryKey('NEW'))), [{ tag: 'NEW', firstHour: 150, lastHour: 200 }, { tag: 'NEW', firstHour: 201, lastHour: 220, unresolved: true }], 'a tag’s runs keep its unresolved stretch apart');
  const settled = index.slotRevision();
  index.setReadFailure('facts', undefined);
  assert.equal(index.slotRevision(), settled, 'a change elsewhere in the index is not a slot change');
  const failure = { kind: 'transient', message: 'lost' } as never;
  index.setReadFailure('identities', failure);
  assert.equal(index.readFailure('identities'), failure);
  assert.notEqual(index.slotRevision(), settled);
});

test('annexed country slot is absent after annexation with valid interval and reason', () => {
  const index = new IdentityIndex(SCOPE, [0, 500]);
  index.setFacts(sampleFacts(), true);
  index.setIdentities(sampleIdentities());

  const before = index.resolve(countrySlotKey(2), 149);
  assert.equal(before.kind, 'known');
  assert.deepEqual(before.tag, { kind: 'known', value: 'ANX' });
  assert.deepEqual(before.valid, [0, 149]);
  assert.equal(before.nextBoundary, 150);

  const after = index.resolve(countrySlotKey(2), 200);
  assert.equal(after.kind, 'absent');
  assert.deepEqual(after.valid, [150, 500]);
  assert.equal(after.nextBoundary, 501);
  assert.equal(after.reason, 'not a country at this hour');
});

test('point style answer without interval proves only its point; interval style covers 20 advances with zero extra requests', async () => {
  const styleCalls: { hour: number; tags: readonly string[] }[] = [];
  const factsHold = deferred<CountryFactsReply>();
  let includeValid = false;

  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: () => factsHold.promise,
    loadIdentities: async () => sampleIdentities(),
    loadGroups: async () => sampleGroups(),
    loadStyles: async (_scope, hour, tags) => {
      styleCalls.push({ hour, tags });
      return {
        branchId: '1',
        hour,
        styles: tags.map(tag => ({
          tag,
          name: 'Early Republic',
          color: [0x33, 0x44, 0x55],
          ...(includeValid ? { valid: [0, 149] as [number, number], flagChoice: FLAG_A } : {}),
        })),
        warnings: [],
      };
    },
    loadFlags: async (_scope, size, choices) => ({
      size,
      flags: choices.map((choice, index) => ({ flag: index, asset: `gfx/flags/${choice.keys[0]}.tga`, key: `${choice.keys[0]}:${size}`, width: 41, height: 26, png: `data:image/png;base64,${index}` })),
      unavailable: [],
      warnings: [],
    }),
  };

  const brokerPoint = new IdentityBroker({ transport });
  const owner = {};
  brokerPoint.setDemand(owner, SCOPE, { asks: [{ key: countryTagKey('GER'), hour: 100 }] });
  await Promise.resolve();
  await Promise.resolve();

  const at100 = brokerPoint.resolve(SCOPE, countryTagKey('GER'), 100);
  assert.equal(at100.kind, 'known');
  assert.deepEqual(at100.valid, [100, 100]);
  assert.equal(brokerPoint.resolve(SCOPE, countryTagKey('GER'), 101).kind, 'pending');
  brokerPoint.dispose();

  includeValid = true;
  styleCalls.length = 0;
  const brokerInterval = new IdentityBroker({ transport });
  brokerInterval.setDemand(owner, SCOPE, { asks: [{ key: countryTagKey('GER'), hour: 100 }] });
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(styleCalls.length, 1);

  for (let hour = 100; hour < 120; hour++) {
    brokerInterval.setDemand(owner, SCOPE, { asks: [{ key: countryTagKey('GER'), hour }] });
    const res = brokerInterval.resolve(SCOPE, countryTagKey('GER'), hour);
    assert.equal(res.kind, 'known', `hour ${hour} stays known without flashing pending`);
    assert.deepEqual(res.name, { kind: 'known', value: 'Early Republic' });
  }
  assert.equal(styleCalls.length, 1, 'twenty playback advances inside one run issue zero extra requests');
  brokerInterval.dispose();
});

test('picker opened at hour 100 stays pending rather than borrowing hour 300, and flag swaps only when flagChoice changes', async () => {
  const flagBatches: RecordedFlagChoice[][] = [];
  const factsHold = deferred<CountryFactsReply>();
  const flagHoldB = deferred<CountryFlagReply>();

  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: () => factsHold.promise,
    loadIdentities: async () => sampleIdentities(),
    loadGroups: async () => sampleGroups(),
    loadStyles: () => deferred<CountryStyleReply>().promise,
    loadFlags: (_scope, size, choices) => {
      flagBatches.push([...choices]);
      if (choices.some(choice => choice.keys[0] === 'GER_b')) return flagHoldB.promise;
      return Promise.resolve({
        size,
        flags: choices.map((choice, index) => ({ flag: index, asset: `gfx/flags/${choice.keys[0]}.tga`, key: `${choice.keys[0]}:${size}`, width: 41, height: 26, png: `flag://${choice.keys[0]}` })),
        unavailable: [],
        warnings: [],
      });
    },
  };

  const broker = new IdentityBroker({ transport });
  const owner = {};
  broker.setDemand(owner, SCOPE, { asks: [{ key: countryTagKey('GER'), hour: 100 }], needFlags: true, flagSize: 'panel' });
  assert.equal(broker.resolve(SCOPE, countryTagKey('GER'), 100).kind, 'pending', 'pending at hour 100 never returns future catalogue name');

  factsHold.resolve({
    ...sampleFacts(),
    names: ['Early Republic', 'Renamed Same Flag', 'Later Empire'],
    countries: [
      {
        tag: 'GER',
        runs: [
          [0, 0, 0x334455, -1, 0],
          [120, 1, 0x334455, -1, 0],
          [150, 2, 0x667788, -1, 1],
        ],
      },
    ],
  });
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();

  const r100 = broker.resolve(SCOPE, countryTagKey('GER'), 100);
  assert.equal(r100.kind, 'known');
  assert.deepEqual(r100.name, { kind: 'known', value: 'Early Republic' });
  assert.equal(r100.flagChoice.kind, 'known');
  assert.equal(broker.flagFor(SCOPE, 'panel', r100.flagChoice.value)?.url, 'flag://GER_a');

  broker.setDemand(owner, SCOPE, { asks: [{ key: countryTagKey('GER'), hour: 120 }], needFlags: true, flagSize: 'panel' });
  await Promise.resolve();
  const r120 = broker.resolve(SCOPE, countryTagKey('GER'), 120);
  assert.equal(r120.kind, 'known');
  assert.deepEqual(r120.name, { kind: 'known', value: 'Renamed Same Flag' });
  assert.equal(r120.flagChoice.kind, 'known');
  assert.equal(broker.flagFor(SCOPE, 'panel', r120.flagChoice.value)?.url, 'flag://GER_a');
  assert.equal(flagBatches.length, 1, 'name change with unchanged flagChoice does not re-fetch flag');

  broker.setDemand(owner, SCOPE, { asks: [{ key: countryTagKey('GER'), hour: 150 }], needFlags: true, flagSize: 'panel' });
  await Promise.resolve();
  const r150 = broker.resolve(SCOPE, countryTagKey('GER'), 150);
  assert.equal(r150.kind, 'known');
  assert.deepEqual(r150.name, { kind: 'known', value: 'Later Empire' });
  assert.equal(r150.flagChoice.kind, 'known');
  assert.equal(broker.flagFor(SCOPE, 'panel', r150.flagChoice.value), undefined, 'changed flagChoice stays pending rather than showing old flag beside new name');

  flagHoldB.resolve({ size: 'panel', flags: [{ flag: 0, asset: 'gfx/flags/GER_b.tga', key: 'GER_b:panel', width: 41, height: 26, png: 'flag://GER_b' }], unavailable: [], warnings: [] });
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(broker.flagFor(SCOPE, 'panel', r150.flagChoice.value)?.url, 'flag://GER_b');
  broker.dispose();
});

test('faction identity resolves dated name, leader, leader colour, unobserved spans, absence and nextBoundary', () => {
  const index = new IdentityIndex(SCOPE, [0, 500]);
  index.setFacts(sampleFacts(), true);
  index.setGroups(sampleGroups());

  const before = index.resolve(factionKey('7'), 20);
  assert.equal(before.kind, 'absent');
  assert.deepEqual(before.valid, [0, 49]);
  assert.equal(before.nextBoundary, 50);

  const early = index.resolve(factionKey('7'), 100);
  assert.equal(early.kind, 'known');
  assert.deepEqual(early.name, { kind: 'known', value: 'Berlin Pact' });
  assert.deepEqual(early.leader, { kind: 'known', value: 'GER' });
  assert.deepEqual(early.color, { kind: 'known', value: [0x33, 0x44, 0x55] });
  assert.deepEqual(early.valid, [50, 149]);
  assert.equal(early.nextBoundary, 150);

  const late = index.resolve(factionKey('7'), 150);
  assert.equal(late.kind, 'known');
  assert.deepEqual(late.name, { kind: 'known', value: 'Axis Pact' });
  assert.deepEqual(late.color, { kind: 'known', value: [0x66, 0x77, 0x88] });
  assert.deepEqual(late.valid, [150, 300]);
  assert.equal(late.nextBoundary, 301);

  const unobserved = index.resolve(factionKey('7'), 355);
  assert.equal(unobserved.kind, 'known');
  assert.equal(unobserved.incomplete, true);
  assert.equal(unobserved.name.kind, 'missing');
  assert.deepEqual(unobserved.valid, [350, 360]);
});

test('generic entity keys (leader, design, state, region) and worker snapshot round-trip cleanly', () => {
  const index = new IdentityIndex(SCOPE, [0, 500]);
  index.setFacts(sampleFacts(), true);
  index.setEntityRuns(leaderKey('GER', 'rommel'), [{ firstHour: 0, lastHour: 200, name: 'Erwin Rommel', artKey: 'portrait:rommel' }]);
  index.setEntityRuns(designKey('GER', '42'), [{ firstHour: 0, lastHour: 500, name: 'Panzer IV' }]);
  index.setEntityRuns(stateKey(64), [{ firstHour: 0, lastHour: 500, name: 'Brandenburg' }]);
  index.setEntityRuns(regionKey(1), [{ firstHour: 0, lastHour: 500, name: 'Southern England' }]);

  const workerIndex = IdentityIndex.fromSnapshot(structuredClone(index.exportSnapshot()));
  const leader = workerIndex.resolve(leaderKey('GER', 'rommel'), 100);
  assert.equal(leader.kind, 'known');
  assert.deepEqual(leader.name, { kind: 'known', value: 'Erwin Rommel' });
  assert.deepEqual(leader.artKey, { kind: 'known', value: 'portrait:rommel' });
  assert.equal(workerIndex.resolve(leaderKey('GER', 'rommel'), 250).kind, 'pending');
  const state = workerIndex.resolve(stateKey(64), 100);
  assert.equal(state.kind, 'known');
  assert.deepEqual(state.name, { kind: 'known', value: 'Brandenburg' });
  const region = workerIndex.resolve(regionKey(1), 100);
  assert.equal(region.kind, 'known');
  assert.deepEqual(region.name, { kind: 'known', value: 'Southern England' });
  const design = workerIndex.resolve(designKey('GER', '42'), 100);
  assert.equal(design.kind, 'known');
  assert.deepEqual(design.name, { kind: 'known', value: 'Panzer IV' });
});

test('1,500 countries use one facts request, batched flags, zero requests advancing in run, and cancel obsolete asks', async () => {
  let factsCalls = 0;
  let flagCalls = 0;
  let abortedStyles = 0;
  const countries = Array.from({ length: 1500 }, (_, i) => `X${String(i).padStart(4, '0')}`);
  const flags: CountryFlagChoice[] = countries.map(tag => ({ keys: [tag, null, null], reasons: [null, null, null] }));
  const factsReply: CountryFactsReply = {
    branchId: '1',
    known: [[0, 1000]],
    names: countries.map(tag => `Country ${tag}`),
    reasons: [],
    flags,
    countries: countries.map((tag, i) => ({ tag, runs: [[0, i, i & 0xffffff, -1, i]] })),
    warnings: [],
  };

  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: async () => { factsCalls++; return factsReply; },
    loadIdentities: async () => ({ branchId: '1', known: [], countries: [], unknown: [], warnings: [] }),
    loadGroups: async () => sampleGroups(),
    loadStyles: (_scope, _hour, _tags, signal) => new Promise((_res, rej) => {
      signal.addEventListener('abort', () => { abortedStyles++; rej(new DOMException('Aborted', 'AbortError')); }, { once: true });
    }),
    loadFlags: async (_scope, size, batch) => {
      flagCalls++;
      return {
        size,
        flags: batch.map((choice, i) => ({ flag: i, asset: `gfx/flags/${choice.keys[0]}.tga`, key: `${choice.keys[0]}:${size}`, width: 41, height: 26, png: `flag://${choice.keys[0]}` })),
        unavailable: [],
        warnings: [],
      };
    },
  };

  const broker = new IdentityBroker({ transport });
  const owner = {};
  broker.setDemand(owner, SCOPE, { asks: countries.map(tag => ({ key: countryTagKey(tag), hour: 100 })), needFlags: true, flagSize: 'panel' });
  for (let i = 0; i < 60; i++) await Promise.resolve();

  assert.equal(factsCalls, 1);
  assert.equal(flagCalls, Math.ceil(1500 / 32));
  assert.ok(abortedStyles >= 1, 'in-flight style fallbacks abort once branch facts land');

  const beforeFlags = flagCalls;
  for (let hour = 101; hour <= 120; hour++) {
    broker.setDemand(owner, SCOPE, { asks: countries.map(tag => ({ key: countryTagKey(tag), hour })), needFlags: true, flagSize: 'panel' });
  }
  await Promise.resolve();
  assert.equal(factsCalls, 1);
  assert.equal(flagCalls, beforeFlags, 'advancing inside loaded run adds zero requests');
  broker.dispose();
});

test('IdentityProvider plugs into BoundedReadView.who() in both worker snapshot and broker modes without leaking past displayed hour', async () => {
  const index = new IdentityIndex(SCOPE, [0, 500]);
  index.setFacts(sampleFacts(), true);
  index.setIdentities(sampleIdentities());
  index.setGroups(sampleGroups());

  const workerSnapshot = structuredClone(index.exportSnapshot());
  const workerIndex = IdentityIndex.fromSnapshot(workerSnapshot);
  const workerProvider = createSnapshotIdentityProvider(workerIndex);
  const workerDemands: [string, string, number][] = [];

  const view = createBoundedReadView({
    hour: 200,
    identity: workerProvider,
    onIdentityDemand: (kind, key, atHour) => { workerDemands.push([kind, key, atHour]); },
  });

  const ger149 = view.who('country', 'GER', 149);
  assert.deepEqual(ger149.name, { status: 'ready', value: 'Early Republic' });
  assert.deepEqual(ger149.colour, { status: 'ready', value: '#334455' });
  assert.equal(ger149.flagChoice?.status, 'ready');
  assert.deepEqual(ger149.validInterval, [0, 149]);
  assert.equal(ger149.nextChangeAt, 150);

  const ger150 = view.who('country', 'GER', 150);
  assert.deepEqual(ger150.name, { status: 'ready', value: 'Later Empire' });
  assert.deepEqual(ger150.validInterval, [150, 500]);

  const annexed = view.who('country', countryKey(2), 160);
  assert.equal(annexed.holder.status, 'absent');
  assert.deepEqual(annexed.validInterval, [150, 500]);

  const faction = view.who('faction', '7', 100);
  assert.deepEqual(faction.name, { status: 'ready', value: 'Berlin Pact' });
  assert.deepEqual(faction.holder, { status: 'ready', value: 'GER' });
  assert.deepEqual(faction.colour, { status: 'ready', value: '#334455' });
  assert.equal(workerDemands.length, 0);

  assert.throws(() => view.who('country', 'GER', 201), FutureDataError);

  const factsHold = deferred<CountryFactsReply>();
  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: () => factsHold.promise,
    loadIdentities: async () => sampleIdentities(),
    loadGroups: async () => sampleGroups(),
    loadStyles: () => deferred<CountryStyleReply>().promise,
    loadFlags: async (_scope, size, choices) => ({
      size,
      flags: choices.map((c, i) => ({ flag: i, asset: `${c.keys[0]}.tga`, key: `${c.keys[0]}:${size}`, width: 41, height: 26, png: `flag://${c.keys[0]}` })),
      unavailable: [],
      warnings: [],
    }),
  };
  const broker = new IdentityBroker({ transport });
  let woke = 0;
  const unsub = broker.subscribe(SCOPE, () => { woke++; });
  const owner = {};
  const brokerProvider = createBrokerIdentityProvider(broker, SCOPE, owner, { branchBounds: [0, 500] });
  const liveView = createBoundedReadView({ hour: 150, identity: brokerProvider });

  const pending = liveView.who('country', 'GER', 100);
  assert.equal(pending.name.status, 'pending');
  await Promise.resolve();
  factsHold.resolve(sampleFacts());
  for (let i = 0; i < 6; i++) await Promise.resolve();
  assert.ok(woke >= 1, 'broker wakes subscribers when pending who() settles');
  const settled = liveView.who('country', 'GER', 100);
  assert.deepEqual(settled.name, { status: 'ready', value: 'Early Republic' });
  unsub();
  broker.dispose();
});

test('useIdentityAtHead does not subscribe to the frame clock or churn snapshots when scope is null or keys are empty', async () => {
  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: async () => sampleFacts(),
    loadIdentities: async () => sampleIdentities(),
    loadGroups: async () => sampleGroups(),
    loadStyles: () => deferred<CountryStyleReply>().promise,
    loadFlags: async (_scope, size) => ({ size, flags: [], unavailable: [], warnings: [] }),
  };
  const broker = new IdentityBroker({ transport });
  let clockHour = 100;
  let frameSubscribes = 0;
  let anchorSubscribes = 0;
  const clock = {
    getHour: () => clockHour,
    subscribeFrame: (_cb: () => void) => { frameSubscribes++; return () => {}; },
    subscribe: (_cb: () => void) => { anchorSubscribes++; return () => {}; },
  };
  const held: { current: HeldHeadHour } = { current: UNHELD_HEAD_HOUR };

  const unsubNoScope = subscribeIdentityAtHead(broker, clock, null, true, true, true, null, undefined, () => {});
  const unsubNoKeys = subscribeIdentityAtHead(broker, clock, SCOPE, true, true, false, null, undefined, () => {});
  assert.equal(frameSubscribes, 0, 'never subscribes to frame clock without both scope and keys');
  assert.equal(anchorSubscribes, 0, 'never subscribes to anchor clock without both scope and keys');
  unsubNoScope();
  unsubNoKeys();

  assert.equal(readIdentityAtHeadHour(broker, clock, null, true, [countryTagKey('GER')], h => h, null, undefined, held), null);
  clockHour = 120;
  assert.equal(readIdentityAtHeadHour(broker, clock, SCOPE, true, [], h => h, null, undefined, held), null);
  clockHour = 140;
  assert.equal(readIdentityAtHeadHour(broker, clock, SCOPE, true, [], h => h, null, undefined, held), null);

  const unsubActive = subscribeIdentityAtHead(broker, clock, SCOPE, true, true, true, null, [0, 500], () => {});
  assert.equal(frameSubscribes, 1, 'subscribes to frame clock once both scope and keys are present');
  broker.setDemand({}, SCOPE, { asks: [{ key: countryTagKey('GER'), hour: 100 }], needFlags: false, flagSize: 'panel' }, undefined, [0, 500]);
  for (let i = 0; i < 6; i++) await Promise.resolve();
  clockHour = 100;
  const h1 = readIdentityAtHeadHour(broker, clock, SCOPE, true, [countryTagKey('GER')], h => h, null, [0, 500], held);
  clockHour = 125;
  const h2 = readIdentityAtHeadHour(broker, clock, SCOPE, true, [countryTagKey('GER')], h => h, null, [0, 500], held);
  assert.equal(h1, 100);
  assert.equal(h2, 100, 'advancing within a loaded run keeps the held hour without triggering a React commit');
  clockHour = 150;
  const h3 = readIdentityAtHeadHour(broker, clock, SCOPE, true, [countryTagKey('GER')], h => h, null, [0, 500], held);
  assert.equal(h3, 150, 'crossing the run boundary advances the hour');
  unsubActive();
  broker.dispose();
});

test('1,500 keys at the head resolve once per proven interval, not per frame, and settled flags do not invalidate them', async () => {
  const countries = Array.from({ length: 1500 }, (_, i) => `X${String(i).padStart(4, '0')}`);
  const flags: CountryFlagChoice[] = countries.map(tag => ({ keys: [tag, null, null], reasons: [null, null, null] }));
  const factsReply: CountryFactsReply = {
    branchId: '1',
    known: [[0, 1000]],
    names: [...countries.map(tag => `Country ${tag}`), 'Renamed'],
    reasons: [],
    flags,
    countries: countries.map((tag, i) => ({ tag, runs: i === 7 ? [[0, i, i, -1, i], [600, 1500, i, -1, i]] : [[0, i, i, -1, i]] })),
    warnings: [],
  };
  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: async () => factsReply,
    loadIdentities: async () => ({ branchId: '1', known: [], countries: [], unknown: [], warnings: [] }),
    loadGroups: async () => sampleGroups(),
    loadStyles: () => deferred<CountryStyleReply>().promise,
    loadFlags: async (_scope, size, batch) => ({
      size,
      flags: batch.map((choice, i) => ({ flag: i, asset: `${choice.keys[0]}.tga`, key: `${choice.keys[0]}:${size}`, width: 41, height: 26, png: `flag://${choice.keys[0]}` })),
      unavailable: [],
      warnings: [],
    }),
  };
  const broker = new IdentityBroker({ transport });
  const keys = countries.map(countryTagKey);
  broker.setDemand({}, SCOPE, { asks: keys.map(key => ({ key, hour: 100 })), needFlags: true, flagSize: 'panel' }, undefined, [0, 1000]);
  for (let i = 0; i < 120; i++) await Promise.resolve();

  const index = broker.indexOf(SCOPE);
  assert.equal(broker.flagFor(SCOPE, 'panel', flags[1499]!)?.url, 'flag://X1499', 'every flag batch settled');
  const revision = index.resolveRevision();
  let resolved = 0;
  const resolve = index.resolve.bind(index);
  index.resolve = ((key, hour) => { resolved++; return resolve(key, hour); }) as typeof index.resolve;

  let clockHour = 100;
  const clock = { getHour: () => clockHour };
  const held: { current: HeldHeadHour } = { current: UNHELD_HEAD_HOUR };
  const read = () => readIdentityAtHeadHour(broker, clock, SCOPE, true, keys, h => h, null, [0, 1000], held);
  assert.equal(read(), 100);
  assert.equal(resolved, 1500, 'the first frame resolves each key once');
  for (let frame = 0; frame < 600; frame++) {
    clockHour = 100 + frame * 0.8;
    assert.equal(read(), 100, 'a frame inside the proven interval keeps the held hour');
  }
  assert.equal(resolved, 1500, 'frames inside the proven interval resolve nothing');

  broker.requestFlagChoices(SCOPE, 'small', flags, undefined);
  for (let i = 0; i < 120; i++) await Promise.resolve();
  assert.equal(broker.flagFor(SCOPE, 'small', flags[1499]!)?.url, 'flag://X1499');
  assert.equal(index.resolveRevision(), revision, 'settled flag batches leave every resolve answer, and the host scan keyed on it, untouched');
  assert.equal(read(), 100);
  assert.equal(resolved, 1500);

  clockHour = 600;
  assert.equal(read(), 600, 'crossing the next boundary advances the hour');
  assert.equal(resolved, 3000, 'one resolve per key at the boundary');
  broker.dispose();
});

test('a lane flag at a size other than the default is demanded, drawn at that size and found in the next snapshot', async () => {
  const sizes: string[] = [];
  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: async () => sampleFacts(),
    loadIdentities: async () => sampleIdentities(),
    loadGroups: async () => sampleGroups(),
    loadStyles: () => deferred<CountryStyleReply>().promise,
    loadFlags: async (_scope, size, batch) => {
      sizes.push(size);
      return {
        size,
        flags: batch.map((choice, i) => ({ flag: i, asset: `${choice.keys[0]}.tga`, key: `${choice.keys[0]}:${size}`, width: 10, height: 7, png: `flag://${choice.keys[0]}/${size}` })),
        unavailable: [],
        warnings: [],
      };
    },
  };
  const broker = new IdentityBroker({ transport });
  const handle = flagArtHandle(flagChoiceKey(FLAG_A), 'small');
  const lane = new LaneArtCache({ fetchBitmap: async () => ({ width: 10, height: 7, close() {} }) as unknown as ImageBitmap });

  const urls = new Set<string>();
  const asks = new Map<string, ArtHandle>();
  const before = IdentityIndex.fromSnapshot(broker.snapshotOf(SCOPE));
  laneArtDemand(handle, before, undefined, urls, asks);
  assert.deepEqual([...asks.values()], [handle], 'an undrawn flag at its own size is a demand');
  assert.equal(createLaneArtProvider(lane, { index: before }).resolve(handle).status, 'pending');

  requestDemandedFlags(broker, SCOPE, [...asks.values()]);
  for (let i = 0; i < 6; i++) await Promise.resolve();
  assert.deepEqual(sizes, ['small'], 'the product is asked at the demanded size');

  const after = IdentityIndex.fromSnapshot(structuredClone(broker.snapshotOf(SCOPE)));
  assert.equal(artUrlForHandle(handle, after), 'flag://GER_a/small');
  assert.equal(artUrlForHandle(flagArtHandle(flagChoiceKey(FLAG_A), 'panel'), after), null, 'another size is not borrowed');
  urls.clear();
  asks.clear();
  laneArtDemand(handle, after, undefined, urls, asks);
  assert.equal(asks.size, 0, 'a drawn flag is not asked again');
  assert.deepEqual([...urls], ['flag://GER_a/small']);
  broker.dispose();
});

test('the playback picker dates every shown row by the displayed hour and moves at the earliest boundary among them', async () => {
  const facts = sampleFacts();
  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: async () => facts,
    loadIdentities: async () => sampleIdentities(),
    loadGroups: async () => sampleGroups(),
    loadStyles: () => deferred<CountryStyleReply>().promise,
    loadFlags: async (_scope, size) => ({ size, flags: [], unavailable: [], warnings: [] }),
  };
  const broker = new IdentityBroker({ transport });
  broker.setDemand({}, SCOPE, { asks: [{ key: countryTagKey('ALP'), hour: 0 }], needFlags: false, flagSize: 'panel' }, undefined, [0, 500]);
  for (let i = 0; i < 6; i++) await Promise.resolve();
  let clockHour = 25;
  const clock = { getHour: () => clockHour };
  const held: { current: HeldHeadHour } = { current: UNHELD_HEAD_HOUR };
  const keys = countryPickerKeys(null, 'ALP', ['ALP', 'GER']);
  const read = () => readIdentityAtHeadHour(broker, clock, SCOPE, true, keys, h => h, null, [0, 500], held);
  assert.equal(read(), 25);
  clockHour = 160;
  const at = read();
  assert.equal(at, 160, 'a shown row renamed at 150 moves the picker although the selection did not change');
  assert.equal(broker.resolve(SCOPE, countryTagKey('GER'), at!).kind === 'known' && (broker.resolve(SCOPE, countryTagKey('GER'), at!) as { name: { value?: string } }).name.value, 'Later Empire');
  clockHour = 20;
  assert.equal(read(), 20, 'a seek back is read at the displayed hour, never a later one');
  assert.deepEqual(countryPickerKeys(null, null, ['GER']).length, 1, 'an empty selection still dates the shown rows');
  assert.deepEqual(countryPickerKeys(3, 'ALP', []), [countrySlotKey(3)], 'a selected slot stays first, so it is the header');
  const defaultHeader = pickerHeaderTag([{ id: 'p1', tag: 'GER' }, { id: 'p2', tag: 'ALP' }], null, 'p1');
  assert.equal(defaultHeader, 'GER', 'default player selection resolves its header tag when selectedTag is null');
  assert.equal(pickerHeaderTag([{ id: 'p1', tag: 'GER' }], null, 'p1', 'Add country'), null, 'an add-country picker has no default header tag');
  assert.deepEqual(countryPickerKeys(null, defaultHeader, []), [countryTagKey('GER')], 'a closed default-player picker still subscribes to the player country');
  assert.deepEqual(countryPickerKeys(null, 'GER', ['GER', 'ALP']), [countryTagKey('GER'), countryTagKey('ALP')], 'opening the picker does not duplicate the header tag');
  const defaultHeld: { current: HeldHeadHour } = { current: UNHELD_HEAD_HOUR };
  clockHour = 25;
  assert.equal(readIdentityAtHeadHour(broker, clock, SCOPE, true, countryPickerKeys(null, defaultHeader, []), h => h, null, [0, 500], defaultHeld), 25);
  clockHour = 160;
  assert.equal(readIdentityAtHeadHour(broker, clock, SCOPE, true, countryPickerKeys(null, defaultHeader, []), h => h, null, [0, 500], defaultHeld), 160, 'crossing a rename moves the closed default-player trigger');
  const absentSlotKeys = countryPickerKeys(99, 'ALP', [], ['ALP', defaultHeader]);
  assert.deepEqual(absentSlotKeys, [countrySlotKey(99), countryTagKey('ALP'), countryTagKey('GER')], 'a selected slot keeps both its selected and default-player fallback tags subscribed when the picker is closed');
  const absentHeld: { current: HeldHeadHour } = { current: UNHELD_HEAD_HOUR };
  clockHour = 50;
  assert.equal(readIdentityAtHeadHour(broker, clock, SCOPE, true, absentSlotKeys, h => h, null, [0, 500], absentHeld), 50);
  clockHour = 160;
  assert.equal(readIdentityAtHeadHour(broker, clock, SCOPE, true, absentSlotKeys, h => h, null, [0, 500], absentHeld), 160, 'crossing a player rename moves a closed picker whose selected slot is absent');

  const gapScope = { ...SCOPE, revision: 'gap' };
  const gapFacts: CountryFactsReply = {
    branchId: 'b1',
    known: [[0, 500]],
    names: ['Alpha Early', 'Alpha Late', 'Germany'],
    reasons: [],
    flags: [FLAG_A, FLAG_B],
    countries: [
      { tag: 'ALP', runs: [[0, 0, 0x112233, -1, 0], [150, 1, 0x112233, -1, 1]] },
      { tag: 'GER', runs: [[0, 2, 0x334455, -1, 0]] },
    ],
    warnings: [],
  };
  const gapIdentities: CountryIdentitiesReply = {
    ...sampleIdentities(),
    branchId: 'b1',
    known: [],
    unknown: [{ firstHour: 0, lastHour: 500, reason: 'identities unrecorded' }],
    countries: [],
    warnings: [],
  };
  const gapBroker = new IdentityBroker({
    transport: {
      ...UNUSED_ENTITY_READS,
      loadFacts: async () => gapFacts,
      loadIdentities: async () => gapIdentities,
      loadGroups: async () => sampleGroups(),
      loadStyles: () => deferred<CountryStyleReply>().promise,
      loadFlags: async (_scope, size) => ({ size, flags: [], unavailable: [], warnings: [] }),
    },
  });
  gapBroker.setDemand({}, gapScope, { asks: [{ key: countrySlotKey(3), hour: 25 }, { key: countryTagKey('ALP'), hour: 25 }], needFlags: false, flagSize: 'panel' }, undefined, [0, 500]);
  for (let i = 0; i < 6; i++) await Promise.resolve();
  const gapKeys = countryPickerKeys(3, 'ALP', [], ['ALP', 'GER']);
  const gapHeld: { current: HeldHeadHour } = { current: UNHELD_HEAD_HOUR };
  clockHour = 25;
  assert.equal(readIdentityAtHeadHour(gapBroker, clock, gapScope, true, gapKeys, h => h, null, [0, 500], gapHeld), 25);
  clockHour = 160;
  assert.equal(readIdentityAtHeadHour(gapBroker, clock, gapScope, true, gapKeys, h => h, null, [0, 500], gapHeld), 160, 'crossing a selected-tag rename moves a closed picker during a slot identity gap');
  gapBroker.dispose();
  broker.dispose();
});

test('an identity key and an art handle keep their whole identity from demand to the drawn picture', () => {
  const index = new IdentityIndex(SCOPE, [0, 500]);
  index.setFacts(sampleFacts(), true);
  index.setIdentities(sampleIdentities());
  index.setDrawnFlag('panel', FLAG_A, { url: 'flag://panel' });
  index.setDrawnFlag('shield', FLAG_A, { url: 'flag://shield' });
  const worker = IdentityIndex.fromSnapshot(structuredClone(index.exportSnapshot()));

  const view = createBoundedReadView({ hour: 200, identity: createSnapshotIdentityProvider(worker) });
  const selected = view.who('country', countryKey(1), 100);
  assert.equal(worker.resolve(countrySlotKey(1), 100).kind, 'known');
  assert.deepEqual(selected.tag, { status: 'ready', value: 'GER' });
  assert.deepEqual(selected.name, { status: 'ready', value: 'Early Republic' });

  const choice = flagChoiceKey(FLAG_A);
  assert.equal(artUrlForHandle(flagArtHandle(choice, 'panel'), worker), 'flag://panel');
  assert.equal(artUrlForHandle(flagArtHandle(choice, 'shield'), worker), 'flag://shield');

  const plain: ArtHandle = { kind: 'sprite', key: 'GFX_x', frame: 0 };
  const asText: ArtHandle = { kind: 'sprite', key: 'GFX_x', frame: 0, text: true };
  const asRecorded: ArtHandle = { kind: 'sprite', key: 'GFX_x', frame: 0, recorded: 'GFX_y' };
  assert.equal(artHandleKey({ kind: 'sprite', key: 'GFX_x' }), artHandleKey(plain));
  assert.deepEqual(readArtDemands([], [plain, asText, asRecorded, plain]), [plain, asText, asRecorded]);
  assert.equal(new Set([plain, asText, asRecorded].map(artHandleKey)).size, 3);
});

test('leader, state, province, region and battle asks each start their product read and answer with its validity', async () => {
  const calls: string[] = [];
  const transport: IdentityTransport = {
    loadFacts: () => deferred<CountryFactsReply>().promise,
    loadIdentities: () => deferred<CountryIdentitiesReply>().promise,
    loadGroups: () => deferred<CountryGroupReply>().promise,
    loadStyles: () => deferred<CountryStyleReply>().promise,
    loadFlags: () => deferred<CountryFlagReply>().promise,
    loadBattleNames: async (_scope, battles) => {
      calls.push(`battles ${battles.join(',')}`);
      return { titles: { 'result:7': { name: 'Second Battle of Coral Reach', missing: null }, 'result:8': { name: null, missing: 'Battle place name not recorded' } }, warnings: [] };
    },
    loadRegions: async () => {
      calls.push('regions');
      return { regions: [{ id: 3, name: 'Coral Reach', nameMissing: null, land: false, sea: true, membershipMissing: null }], warnings: [] };
    },
    loadPlaceNameRuns: async (_scope, from, to, states, provinces) => {
      calls.push(`places ${from}-${to} ${states.join(',')} ${provinces.join(',')}`);
      return {
        from,
        to,
        unknown: [{ firstHour: 100, lastHour: 120, reason: 'Not in this recording: the game was paused' }],
        states: states.map(id => ({ id, runs: [{ firstHour: from, lastHour: 199, name: `State ${id}`, missing: null, presence: 'present' }, { firstHour: 200, lastHour: to, name: `Renamed ${id}`, missing: null, presence: 'present' }] })),
        provinces: provinces.map(id => ({ id, runs: [{ firstHour: from, lastHour: to, name: null, missing: 'Province unnamed in this map', presence: 'present' }] })),
        warnings: [],
      };
    },
    loadLeaderNameRuns: async (_scope, owner, from, to, references) => {
      calls.push(`leaders ${owner} ${from}-${to} ${references.join(',')}`);
      return {
        from,
        to,
        unknown: [],
        leaders: [
          { reference: 'L1', runs: [{ firstHour: from, lastHour: 179, name: null, missing: 'Not a leader of this country at this hour', presence: 'absent' }, { firstHour: 180, lastHour: to, name: 'Ada Vance', missing: null, presence: 'present' }] },
          { reference: 'L2', runs: [{ firstHour: from, lastHour: to, name: null, missing: "The leader's owner is not named at this hour", presence: 'unknown' }] },
        ],
        warnings: [],
      };
    },
  };
  const broker = new IdentityBroker({ transport });
  const provider = createBrokerIdentityProvider(broker, SCOPE, {}, { branchBounds: [0, 500], needFlags: false });
  const view = createBoundedReadView({ hour: 200, identity: provider });
  const ask = () => [
    view.who('battle', 'result:7', 20), view.who('battle', 'result:8', 20),
    view.who('region', '3', 150), view.who('region', '9', 150),
    view.who('state', '64', 150), view.who('province', '7', 150), view.who('leader', 'ALP:L1', 150), view.who('leader', 'ALP:L2', 150),
  ];
  assert.ok(ask().every(result => result.name.status === 'pending'));
  for (let i = 0; i < 20; i++) await Promise.resolve();
  ask();
  for (let i = 0; i < 20; i++) await Promise.resolve();
  assert.deepEqual([...calls].sort(), ['battles result:7,result:8', 'leaders ALP 0-500 L1,L2', 'places 0-500 64 7', 'regions']);

  const [battle, unknownBattle, region, unknownRegion, state, province, leader, otherLeader] = ask();
  assert.deepEqual(battle!.name, { status: 'ready', value: 'Second Battle of Coral Reach' });
  assert.deepEqual(unknownBattle!.name, { status: 'absent', reason: 'Battle place name not recorded' });
  assert.deepEqual(region!.name, { status: 'ready', value: 'Coral Reach' });
  assert.deepEqual(region!.validInterval, [0, 500]);
  assert.equal(unknownRegion!.name.status, 'absent');
  assert.deepEqual(state!.name, { status: 'ready', value: 'State 64' });
  assert.deepEqual(state!.validInterval, [121, 199], 'a run is valid up to its rename and never across an unknown span');
  assert.deepEqual(view.who('state', '64', 110).name, { status: 'absent', reason: 'Not in this recording: the game was paused' });
  assert.deepEqual(view.who('state', '64', 90).validInterval, [0, 99]);
  assert.deepEqual(view.who('state', '64', 200).name, { status: 'ready', value: 'Renamed 64' });
  const worker = createSnapshotIdentityProvider(IdentityIndex.fromSnapshot(structuredClone(broker.snapshotOf(SCOPE))));
  assert.deepEqual(worker.resolve('state', '64', 110 as DisplayedHour).name, { status: 'absent', reason: 'Not in this recording: the game was paused' }, 'unknown spans cross to the worker');
  assert.deepEqual(province!.name, { status: 'absent', reason: 'Province unnamed in this map' });
  assert.equal(leader!.holder.status, 'absent', 'a leader before it serves its owner is absent, not named');
  assert.deepEqual(view.who('leader', 'ALP:L1', 190).name, { status: 'ready', value: 'Ada Vance' });
  assert.deepEqual(view.who('leader', 'ALP:L1', 190).validInterval, [180, 500]);
  assert.deepEqual(otherLeader!.holder, { status: 'absent', reason: "The leader's owner is not named at this hour" }, 'an unnamed owner is not known, never another owner and never ready');
  assert.deepEqual(view.who('state', '64', 110).holder, { status: 'absent', reason: 'Not in this recording: the game was paused' }, 'a place inside a recording gap is not ready');
  assert.deepEqual(worker.resolve('leader', 'ALP:L2', 150 as DisplayedHour).holder, otherLeader!.holder, 'unknown ownership crosses to the worker');
  assert.equal(calls.length, 4, 'answers inside their validity ask nothing again');
  broker.dispose();
});

test('coalesced sparse identity runs stay missing on unrecorded gaps outside known spans', () => {
  const index = new IdentityIndex(SCOPE, [0, 40]);
  index.setFacts({ ...sampleFacts(), known: [[10, 10], [30, 30]] }, true);
  index.setIdentities({
    branchId: '1',
    known: [{ firstHour: 10, lastHour: 10 }, { firstHour: 30, lastHour: 30 }],
    countries: [
      {
        tag: 'GER',
        identities: [{ firstHour: 10, lastHour: 30, slot: 1, countryId: 10 }],
        unresolved: [],
      },
    ],
    unknown: [],
    warnings: [],
  });

  const at10 = index.resolve(countrySlotKey(1), 10);
  assert.equal(at10.kind, 'known');
  assert.deepEqual(at10.tag, { kind: 'known', value: 'GER' });
  assert.deepEqual(at10.valid, [10, 10]);
  assert.equal(at10.nextBoundary, 11);

  const gap20 = index.resolve(countrySlotKey(1), 20);
  assert.equal(gap20.kind, 'known');
  assert.equal(gap20.holder.kind, 'missing');
  assert.equal(gap20.tag.kind, 'missing');
  assert.deepEqual(gap20.valid, [11, 29]);
  assert.equal(gap20.nextBoundary, 30);
  assert.equal(index.slotHolderAt(1, 20), null);

  const byTagGap = index.resolve(countryTagKey('GER'), 20);
  assert.equal(byTagGap.kind, 'known');
  assert.equal(byTagGap.holder.kind, 'missing');
  assert.deepEqual(byTagGap.valid, [11, 29]);

  const at30 = index.resolve(countrySlotKey(1), 30);
  assert.equal(at30.kind, 'known');
  assert.deepEqual(at30.tag, { kind: 'known', value: 'GER' });
  assert.deepEqual(at30.valid, [30, 30]);
});

test('a country the lane asks for and the snapshot lacks becomes a broker demand and returns in the next snapshot', async () => {
  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: async () => sampleFacts(),
    loadIdentities: () => deferred<CountryIdentitiesReply>().promise,
    loadGroups: () => deferred<CountryGroupReply>().promise,
    loadStyles: () => deferred<CountryStyleReply>().promise,
    loadFlags: async (_scope, size, choices) => ({
      size,
      flags: choices.map((choice, flag) => ({ flag, asset: `${choice.keys[0]}.tga`, key: `${choice.keys[0]}:${size}`, width: 41, height: 26, png: `flag://${choice.keys[0]}` })),
      unavailable: [],
      warnings: [],
    }),
  };
  const broker = new IdentityBroker({ transport });
  const owner = {};
  const laneView = (snapshot: ReturnType<IdentityBroker['snapshotOf']>, demands: [string, string, number][]) => createBoundedReadView({
    hour: 200,
    identity: createSnapshotIdentityProvider(IdentityIndex.fromSnapshot(structuredClone(snapshot))),
    onIdentityDemand: (kind, key, atHour) => { demands.push([kind, key, atHour]); },
  });

  const demands: [string, string, number][] = [];
  const first = laneView(broker.snapshotOf(SCOPE, undefined, [0, 500]), demands);
  assert.equal(first.who('country', 'ALP', 120).name.status, 'pending');
  assert.deepEqual(demands, [['country', 'ALP', 120]]);

  broker.setDemand(owner, SCOPE, { asks: demands.map(([kind, key, hour]) => ({ key: parseEntityKey(kind as 'country', key), hour })), needFlags: true, flagSize: 'panel' }, undefined, [0, 500]);
  for (let i = 0; i < 20; i++) await Promise.resolve();

  const after: [string, string, number][] = [];
  const second = laneView(broker.snapshotOf(SCOPE, undefined, [0, 500]), after);
  const alp = second.who('country', 'ALP', 120);
  assert.deepEqual(alp.name, { status: 'ready', value: 'Alpha' });
  assert.equal(alp.flagChoice?.status, 'ready');
  assert.equal(artUrlForHandle(flagArtHandle(alp.flagChoice!.status === 'ready' ? alp.flagChoice!.value : '', 'panel'), IdentityIndex.fromSnapshot(broker.snapshotOf(SCOPE))), 'flag://GER_a');
  broker.dispose();
});

test('a failed branch read is not restarted by later demands: a transient failure waits for its backoff, a refusal for an explicit retry', async () => {
  let now = 0;
  const timers: { at: number; run: () => void }[] = [];
  let identityReads = 0;
  let groupReads = 0;
  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: async () => sampleFacts(),
    loadIdentities: async () => { identityReads++; throw new RefusedError('Unsupported identity table'); },
    loadGroups: async () => { groupReads++; throw new TypeError('connection reset'); },
    loadStyles: () => deferred<CountryStyleReply>().promise,
    loadFlags: async (_scope, size) => ({ size, flags: [], unavailable: [], warnings: [] }),
  };
  const broker = new IdentityBroker({ transport, clock: { now: () => now, after: (ms, run) => { timers.push({ at: now + ms, run }); return () => {}; } } });
  const owner = {};
  for (let hour = 100; hour < 160; hour++) {
    broker.setDemand(owner, SCOPE, { asks: [{ key: countryTagKey('GER'), hour }], needFlags: false, needGroups: true, flagSize: 'panel' });
    for (let i = 0; i < 4; i++) await Promise.resolve();
  }
  assert.equal(identityReads, 1, 'a refused read is asked once');
  assert.equal(groupReads, 1, 'a transient failure waits for its backoff');
  assert.equal(timers.length, 1);
  now = timers[0]!.at;
  timers[0]!.run();
  for (let i = 0; i < 6; i++) await Promise.resolve();
  assert.equal(groupReads, 2, 'the backoff asks again');
  assert.equal(identityReads, 1);
  broker.retry(SCOPE);
  for (let i = 0; i < 6; i++) await Promise.resolve();
  assert.equal(identityReads, 2, 'an explicit retry asks a refused read again');
  broker.dispose();
});

for (const omitted of [false, true]) test(`a ${omitted ? 'missing' : 'failed'} flag response can retry without rebuilding carried status`, async () => {
  const index = new IdentityIndex(SCOPE, [0, 500]);
  index.setFacts(sampleFacts(), true);
  index.setIdentities({ ...sampleIdentities(), countries: [{ tag: 'GER', identities: [], unresolved: [{ firstHour: 0, lastHour: 500 }] }] });
  index.setReadFailure('groups', new RefusedError('Unsupported faction table'));
  const worker = createSnapshotIdentityProvider(IdentityIndex.fromSnapshot(structuredClone(index.exportSnapshot())));
  const ger = worker.resolve('country', 'GER', 100 as DisplayedHour);
  assert.deepEqual(ger.tag, { status: 'ready', value: 'GER' });
  assert.equal(ger.holder.status, 'absent');
  assert.deepEqual(worker.resolve('faction', '7', 100 as DisplayedHour).name, { status: 'failed', reason: 'Unsupported faction table', retryable: false });

  let now = 0;
  const timers: { at: number; run: () => void }[] = [];
  let flagCalls = 0;
  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: async () => sampleFacts(),
    loadIdentities: async () => sampleIdentities(),
    loadGroups: () => deferred<CountryGroupReply>().promise,
    loadStyles: () => deferred<CountryStyleReply>().promise,
    loadFlags: async (_scope, size, choices) => {
      flagCalls++;
      if (flagCalls === 1) {
        if (omitted) return { size, flags: [], unavailable: [], warnings: [] };
        throw new TypeError('connection reset');
      }
      return { size, flags: choices.map((choice, flag) => ({ flag, asset: 'a.tga', key: 'k', width: 41, height: 26, png: `flag://${choice.keys[0]}` })), unavailable: [], warnings: [] };
    },
  };
  const broker = new IdentityBroker({ transport, clock: { now: () => now, after: (ms, run) => { timers.push({ at: now + ms, run }); return () => {}; } } });
  broker.setDemand({}, SCOPE, { asks: [{ key: countryTagKey('GER'), hour: 100 }], needFlags: true, flagSize: 'panel' });
  for (let i = 0; i < 20; i++) await Promise.resolve();
  assert.equal(flagCalls, 1);
  assert.ok(broker.flagFor(SCOPE, 'panel', FLAG_A)?.failed);
  assert.equal(timers.length, 1, 'a retryable flag failure schedules its retry');
  now = timers[0]!.at;
  timers[0]!.run();
  for (let i = 0; i < 20; i++) await Promise.resolve();
  assert.equal(flagCalls, 2);
  assert.equal(broker.flagFor(SCOPE, 'panel', FLAG_A)?.url, 'flag://GER_a');
  broker.dispose();

  assert.deepEqual(toArtSlotState(refusedArt('Unsupported flag format')), { kind: 'refused', reason: 'Unsupported flag format' });
  assert.deepEqual(toArtSlotState(NO_ART), { kind: 'none' });
  assert.deepEqual(toArtSlotState(productArt({ status: 'confirmedMissing', reason: 'No flag file' }), 'GER'), { kind: 'missing', reason: 'No flag file', label: 'GER' });
});

test('an entity failure belongs to its window and a retryable one is asked again after its backoff', async () => {
  let now = 0;
  const timers: { at: number; run: () => void }[] = [];
  const clock = { now: () => now, after: (ms: number, run: () => void) => { const timer = { at: now + ms, run }; timers.push(timer); return () => { timers.splice(timers.indexOf(timer), 1); }; } };
  const asked: string[] = [];
  let failNext = true;
  const transport = {
    loadPlaceNameRuns: async (_scope: unknown, from: number, to: number, states: readonly number[]) => {
      asked.push(`${from}-${to}`);
      if (from === 0 && failNext) {
        failNext = false;
        throw new TypeError('network dropped');
      }
      return { from, to, unknown: [], states: states.map(id => ({ id, runs: [{ firstHour: from, lastHour: to, name: `State ${id}`, missing: null, presence: 'present' }] })), provinces: [], warnings: [] };
    },
  } as unknown as IdentityTransport;
  const broker = new IdentityBroker({ transport, clock });
  const owner = {};
  broker.setDemand(owner, SCOPE, { asks: [{ key: stateKey(7), hour: 10 }, { key: stateKey(7), hour: 800 }] }, undefined, [0, 2000]);
  for (let i = 0; i < 20; i++) await Promise.resolve();
  assert.equal(broker.resolve(SCOPE, stateKey(7), 10).kind, 'failed');
  assert.equal(broker.resolve(SCOPE, stateKey(7), 800).kind, 'known', 'a failure in one window never answers another');
  assert.ok(timers.length > 0, 'the failed window stays scheduled through its backoff');
  now = timers[0]!.at;
  timers.shift()!.run();
  for (let i = 0; i < 20; i++) await Promise.resolve();
  assert.deepEqual(asked.filter(span => span === '0-719').length, 2);
  const shown = broker.resolve(SCOPE, stateKey(7), 10);
  assert.equal(shown.kind === 'known' && shown.name.kind === 'known' ? shown.name.value : null, 'State 7');
  broker.dispose();
});

test('1,500 flags drain from what is still outstanding, not a rescan per batch, and an entity settlement leaves the country revision alone', async () => {
  const countries = Array.from({ length: 1500 }, (_, i) => `X${String(i).padStart(4, '0')}`);
  const factsReply: CountryFactsReply = {
    branchId: '1', known: [[0, 1000]], names: countries.map(tag => `Country ${tag}`), reasons: [],
    flags: countries.map(tag => ({ keys: [tag, null, null], reasons: [null, null, null] })),
    countries: countries.map((tag, i) => ({ tag, runs: [[0, i, 0, -1, i]] })), warnings: [],
  };
  let flagCalls = 0;
  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: async () => factsReply,
    loadIdentities: async () => ({ branchId: '1', known: [], countries: [], unknown: [], warnings: [] }),
    loadGroups: async () => sampleGroups(),
    loadStyles: () => deferred<CountryStyleReply>().promise,
    loadFlags: async (_scope, size, batch) => {
      flagCalls++;
      return { size, flags: batch.map((choice, i) => ({ flag: i, asset: `gfx/flags/${choice.keys[0]}.tga`, key: `${choice.keys[0]}:${size}`, width: 41, height: 26, png: `flag://${choice.keys[0]}` })), unavailable: [], warnings: [] };
    },
  };
  const broker = new IdentityBroker({ transport });
  const index = broker.indexOf(SCOPE);
  let lookups = 0;
  const lookup = index.getDrawnFlag.bind(index);
  index.getDrawnFlag = (size, choice) => { lookups++; return lookup(size, choice); };
  broker.setDemand({}, SCOPE, { asks: countries.map(tag => ({ key: countryTagKey(tag), hour: 100 })), needFlags: true, flagSize: 'panel' });
  for (let i = 0; i < 400; i++) await Promise.resolve();
  assert.equal(flagCalls, Math.ceil(1500 / FLAG_BATCH_MAX));
  assert.ok(lookups <= 3 * 1500, `drawn-flag lookups grow with the flags, not flags x batches (${lookups})`);
  const countryRevision = index.countryRevision();
  index.replaceEntityRuns(stateKey(7), 0, 719, [{ firstHour: 0, lastHour: 719, name: 'State 7' }]);
  assert.equal(index.countryRevision(), countryRevision, 'a place settling does not ask the host to rescan country flags');
  broker.dispose();
});

test('visiting 40 windows keeps at most the window bound of names, the demanded window stays answered, and a revisit reads again', async () => {
  const asked: number[] = [];
  const transport = {
    loadPlaceNameRuns: async (_scope: unknown, from: number, to: number, states: readonly number[]) => {
      asked.push(from);
      return { from, to, unknown: [], states: states.map(id => ({ id, runs: [{ firstHour: from, lastHour: to, name: `State ${id} at ${from}`, missing: null, presence: 'present' }] })), provinces: [], warnings: [] };
    },
  } as unknown as IdentityTransport;
  const broker = new IdentityBroker({ transport });
  const owner = {};
  for (let window = 0; window < 40; window++) {
    broker.setDemand(owner, SCOPE, { asks: [{ key: stateKey(7), hour: window * 720 + 5 }] }, undefined, [0, 40 * 720]);
    for (let i = 0; i < 20; i++) await Promise.resolve();
  }
  const held = broker.snapshotOf(SCOPE).entities?.[identityKeyString(stateKey(7))] ?? [];
  assert.ok(held.length <= MAX_ENTITY_WINDOWS, `names held for ${held.length} windows`);
  const last = broker.resolve(SCOPE, stateKey(7), 39 * 720 + 5);
  assert.equal(last.kind === 'known' && last.name.kind === 'known' ? last.name.value : null, `State 7 at ${39 * 720}`, 'the demanded window is pinned');
  assert.equal(broker.resolve(SCOPE, stateKey(7), 5).kind, 'pending', 'an evicted window is not answered from memory');
  broker.setDemand(owner, SCOPE, { asks: [{ key: stateKey(7), hour: 5 }] }, undefined, [0, 40 * 720]);
  for (let i = 0; i < 20; i++) await Promise.resolve();
  assert.equal(asked.filter(from => from === 0).length, 2, 'a revisited window is read again');
  broker.dispose();
});

test('a permanent failure for one state holds back only that state: another state in the same window is still read', async () => {
  const asked: number[][] = [];
  const transport = {
    loadPlaceNameRuns: async (_scope: unknown, from: number, to: number, states: readonly number[]) => {
      asked.push([...states]);
      if (states.includes(7)) throw new RefusedError('state 7 is refused by name');
      return { from, to, unknown: [], states: states.map(id => ({ id, runs: [{ firstHour: from, lastHour: to, name: `State ${id}`, missing: null, presence: 'present' }] })), provinces: [], warnings: [] };
    },
  } as unknown as IdentityTransport;
  const broker = new IdentityBroker({ transport, clock: { now: () => 0, after: () => () => {} } });
  const owner = {};
  broker.setDemand(owner, SCOPE, { asks: [{ key: stateKey(7), hour: 10 }] }, undefined, [0, 2000]);
  for (let i = 0; i < 20; i++) await Promise.resolve();
  assert.equal(broker.resolve(SCOPE, stateKey(7), 10).kind, 'failed');
  broker.setDemand(owner, SCOPE, { asks: [{ key: stateKey(7), hour: 10 }, { key: stateKey(8), hour: 10 }] }, undefined, [0, 2000]);
  for (let i = 0; i < 20; i++) await Promise.resolve();
  assert.deepEqual(asked, [[7], [8]], 'the refused state is not asked again; the new state in its window is');
  const shown = broker.resolve(SCOPE, stateKey(8), 10);
  assert.equal(shown.kind === 'known' && shown.name.kind === 'known' ? shown.name.value : null, 'State 8');
  broker.dispose();
});

test('releasing a demand trims the held windows without another load, and an evicted window takes its failures and gaps with it', async () => {
  const transport = {
    loadPlaceNameRuns: async (_scope: unknown, from: number, to: number, states: readonly number[]) => {
      if (from === 0) throw new RefusedError('this window is refused');
      return { from, to, unknown: [{ firstHour: from, lastHour: from + 5, reason: 'Not in this recording: gap' }], states: states.map(id => ({ id, runs: [{ firstHour: from + 6, lastHour: to, name: `State ${id}`, missing: null, presence: 'present' }] })), provinces: [], warnings: [] };
    },
  } as unknown as IdentityTransport;
  const broker = new IdentityBroker({ transport, clock: { now: () => 0, after: () => () => {} } });
  const owner = {};
  const windows = MAX_ENTITY_WINDOWS + 2;
  broker.setDemand(owner, SCOPE, { asks: Array.from({ length: windows }, (_, window) => ({ key: stateKey(7), hour: window * 720 + 10 })) }, undefined, [0, windows * 720]);
  for (let i = 0; i < 600; i++) await Promise.resolve();
  const before = broker.snapshotOf(SCOPE);
  assert.equal(before.failures?.entities?.[identityKeyString(stateKey(7))]?.length, 1, 'the failed window is held');
  broker.setDemand(owner, SCOPE, null);
  const after = broker.snapshotOf(SCOPE);
  const runs = after.entities?.[identityKeyString(stateKey(7))] ?? [];
  assert.ok(runs.length <= MAX_ENTITY_WINDOWS, `released windows are trimmed at once (${runs.length} held)`);
  assert.equal(after.failures?.entities?.[identityKeyString(stateKey(7))], undefined, 'the oldest windows, the failed one among them, leave with their failures');
  assert.ok(!(after.entityUnknown?.state ?? []).some(gap => gap.firstHour === 720), 'an evicted window leaves no gap behind');
  assert.ok((after.entityUnknown?.state ?? []).some(gap => gap.firstHour === 2 * 720), 'a retained window keeps its gap');
  broker.dispose();
});

test('the picker dates the rows that decide what it shows in the same render: a newly visible row and a search candidate that renames into a match', async () => {
  const transport: IdentityTransport = {
    ...UNUSED_ENTITY_READS,
    loadFacts: async () => sampleFacts(),
    loadIdentities: async () => sampleIdentities(),
    loadGroups: async () => sampleGroups(),
    loadStyles: () => deferred<CountryStyleReply>().promise,
    loadFlags: async (_scope, size) => ({ size, flags: [], unavailable: [], warnings: [] }),
  };
  const broker = new IdentityBroker({ transport });
  broker.setDemand({}, SCOPE, { asks: [{ key: countryTagKey('ALP'), hour: 0 }], needFlags: false, flagSize: 'panel' }, undefined, [0, 500]);
  for (let i = 0; i < 6; i++) await Promise.resolve();
  let clockHour = 25;
  const dating = pickerDating(broker, { getHour: () => clockHour }, SCOPE, null, 'ALP', h => h, null, [0, 500]);
  assert.equal(dating(['ALP']), 25);
  clockHour = 160;
  const held = dating(['ALP']);
  assert.equal(dating(['ALP', 'GER']), 160, 'a row scrolled into view is dated at the displayed hour before it is painted, not at the hour held for the rows before it');
  assert.ok(held === 25 || held === 160);
  clockHour = 170;
  const searchTags = ['ALP', 'GER'];
  const at = dating(searchTags)!;
  const ger = broker.resolve(SCOPE, countryTagKey('GER'), at);
  assert.equal(ger.kind === 'known' && ger.name.kind === 'known' ? ger.name.value : null, 'Later Empire', 'while searching every candidate dates the hour, so one renamed into a match is found');
  broker.dispose();
});

test('a country name that is still loading reads as loading in every family, never as its tag, and as the name once it arrives', () => {
  const waiting = createBoundedReadView({ hour: 10, identity: datedIdentity({}) });
  const named = createBoundedReadView({ hour: 10, identity: datedIdentity({ GER: [{ from: 0, name: 'Germany' }] }) });
  const looks = (view: typeof waiting) => countrySeriesLooks(['GER'], view, { holders: null, from: 0, surface: '#111111', withFlag: false }).get('GER')!;
  const shown = (view: typeof waiting) => ({
    heat: resolveEntityFromReadView(view, 'country', 'GER', 10, 'GER').label,
    timeChart: countryNameAt(view, 'GER', 10),
    timeChartLine: looks(view).name,
    timeChartRun: looks(view).identities.map(run => run.name).join('|'),
    timeline: countryLook(view, 'GER', 10).name,
  });
  const all = (label: string) => ({ heat: label, timeChart: label, timeChartLine: label, timeChartRun: label, timeline: label });
  assert.deepEqual(shown(waiting), all(LOADING_NAME));
  assert.deepEqual(shown(named), all('Germany'));
  const waitingBars = countryRowPresentation(waiting, 'GER');
  const namedBars = countryRowPresentation(named, 'GER');
  assert.deepEqual(waitingBars.label, { kind: 'pending', reason: NAME_LOADING });
  assert.equal(barsLabelText(waitingBars.label), 'Loading…');
  assert.equal(barsLabelMuted(waitingBars.label), true, 'the bars row knows the name is loading, so it is drawn muted');
  assert.equal(barsLabelMissing(waitingBars.label), false, 'loading is not "not recorded"');
  assert.deepEqual(namedBars.label, { kind: 'known', value: 'Germany' });
  assert.equal(resolveEntityFromReadView(waiting, 'country', 'GER', 10, 'GER').pending, true);
  const flow = presentFlowEndpoint(waiting, { key: 's7', slot: 7, tag: 'GER', collective: false, unknown: null });
  assert.deepEqual(flow.label, { kind: 'pending', reason: NAME_LOADING }, 'a flow endpoint waiting for its name does not print the tag as the name');
});

test('snapshot reuse refuses equal counters from a recreated index or a different language', () => {
  const scopeEn = { sourceId: 's', revision: '1', branchId: 'b1', content: 1, language: 'english' };
  const first = new IdentityIndex(scopeEn, [0, 500]);
  first.setEntityRuns({ kind: 'state', id: '1' }, [{ firstHour: 0, lastHour: 500, name: 'State One' }]);
  const workerFirst = IdentityIndex.fromSnapshot(structuredClone(first.exportSnapshot()));

  const recreated = new IdentityIndex(scopeEn, [0, 500]);
  recreated.setEntityRuns({ kind: 'state', id: '2' }, [{ firstHour: 0, lastHour: 500, name: 'State Two' }]);
  const workerRecreated = IdentityIndex.fromSnapshot(structuredClone(recreated.exportSnapshot()), workerFirst);
  const stateTwo = workerRecreated.resolve({ kind: 'state', id: '2' }, 10);
  assert.equal(stateTwo.kind, 'known', 'a recreated index with equal state revision never reuses stale entities');
  assert.equal(stateTwo.name.kind === 'known' ? stateTwo.name.value : null, 'State Two');

  const scopeDe = { ...scopeEn, language: 'german' };
  const translated = new IdentityIndex(scopeDe, [0, 500]);
  translated.setEntityRuns({ kind: 'state', id: '1' }, [{ firstHour: 0, lastHour: 500, name: 'Staat Eins' }]);
  const workerTranslated = IdentityIndex.fromSnapshot(structuredClone(translated.exportSnapshot()), workerFirst);
  const stateOneDe = workerTranslated.resolve({ kind: 'state', id: '1' }, 10);
  assert.equal(stateOneDe.kind === 'known' && stateOneDe.name.kind === 'known' ? stateOneDe.name.value : null, 'Staat Eins');
});
