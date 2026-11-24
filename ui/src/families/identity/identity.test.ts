import assert from 'node:assert/strict';
import test from 'node:test';
import type {
  CountryFactsReply,
  CountryFlagChoice,
  CountryFlagReply,
  CountryGroupReply,
  CountryIdentitiesReply,
  CountryStyleReply,
} from '../../generated/dashboard';
import { countryKey } from '../../dashboard/countryKey.ts';
import { artUrlForHandle, flagArtHandle, mergeArtDemands } from '../art/provider.ts';
import { artHandleKey } from '../art/types.ts';
import { createBoundedReadView, FutureDataError } from '../boundedView.ts';
import type { ArtHandle } from '../plan.ts';
import { IdentityBroker, type IdentityTransport } from './broker.ts';
import { createBrokerIdentityProvider, createSnapshotIdentityProvider } from './provider.ts';
import { readIdentityAtHeadHour, subscribeIdentityAtHead } from './react.ts';
import {
  IdentityIndex,
  countrySlotKey,
  countryTagKey,
  designKey,
  factionKey,
  flagChoiceKey,
  leaderKey,
  regionKey,
  stateKey,
  type IdentityScope,
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
          { firstHour: 50, lastHour: 149, name: 'Berlin Pact', reason: null },
          { firstHour: 150, lastHour: 300, name: 'Axis Pact', reason: null },
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
    unobserved: { humans: [], factions: [{ firstHour: 350, lastHour: 360 }], wars: [] },
    warnings: [],
  };
}

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
  const flagBatches: CountryFlagChoice[][] = [];
  const factsHold = deferred<CountryFactsReply>();
  const flagHoldB = deferred<CountryFlagReply>();

  const transport: IdentityTransport = {
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
    loadFacts: async () => { factsCalls++; return factsReply; },
    loadIdentities: async () => ({ branchId: '1', countries: [], unknown: [], warnings: [] }),
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
  assert.deepEqual(ger149.colour, { status: 'ready', value: 'rgb(51, 68, 85)' });
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
  assert.deepEqual(faction.colour, { status: 'ready', value: 'rgb(51, 68, 85)' });
  assert.equal(workerDemands.length, 0);

  assert.throws(() => view.who('country', 'GER', 201), FutureDataError);

  const factsHold = deferred<CountryFactsReply>();
  const transport: IdentityTransport = {
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
  const held = { current: { key: '', hour: null as number | null } };

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
  assert.deepEqual(mergeArtDemands([], [plain, asText, asRecorded, plain]), [plain, asText, asRecorded]);
  assert.equal(new Set([plain, asText, asRecorded].map(artHandleKey)).size, 3);
});
