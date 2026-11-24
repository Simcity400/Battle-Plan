import assert from 'node:assert/strict';
import test from 'node:test';
import { askTimeline, flagIn, forgetTimelines, heldFacts, heldTimeline, identitySince, stylesIn, validateCountryFacts, watchTimeline } from './countryTimeline.ts';
import type { CountryTimeline } from './countryTimeline';
import { answerHeld } from './countryNamesAt.ts';
import { FactRequests, HourStore, MAX_NAME_HOURS, nameKey, readFacts } from './countryFacts.ts';
import type { FactSpec } from './countryFacts';
import type { CountryFactsReply, CountryStyle, CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient, DashboardSource, ProductRequest } from './client';

const heldStyles = (client: DashboardClient, src: { sourceId: string; revision: string; branchId: string; content: number },
  rec: DashboardSource | undefined, hour: number, tags: readonly string[]) => {
  const timeline = rec && askTimeline(client, src, rec, hour);
  return timeline ? stylesIn(timeline, hour, tags) : undefined;
};

const REPLY: CountryFactsReply = {
  branchId: '1', known: [[10, 20], [30, 40]], names: ['Germany', 'German Reich'], reasons: ['not a country at this hour', 'no recorded map colour'],
  flags: [{ keys: ['GER_democratic', 'GER_democratic', null], reasons: [null, null, 'flag artwork unavailable in the recorded content'] },
    { keys: ['GER_fascism', 'GER_fascism', 'GER_fascism'], reasons: [null, null, null] }],
  countries: [
    { tag: 'GER', runs: [[10, 0, 0x336699, -1, 0], [15, 1, 0x336699, -1, 1], [35, 1, -1, 1, 1]] },
    { tag: 'D01', runs: [[10, -1, -1, 0, -1], [32, 0, 0x112233, -1, 0]] },
  ],
  warnings: ['content'],
};

const replyTo = (request: ProductRequest, data: unknown) => ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });

function facts(reply: CountryFactsReply = REPLY) {
  const asked: unknown[] = [];
  const client = {
    product: async (request: ProductRequest) => {
      asked.push([request.product, request.params]);
      return replyTo(request, reply);
    },
  } as unknown as DashboardClient;
  return { client, asked };
}
const source = { sourceId: 'timeline', revision: 'r1', branchId: '1', content: 0 };
const recording = { id: 'timeline', revision: 'r1' } as DashboardSource;
const settle = () => new Promise(resolve => setTimeout(resolve, 0));

test('a held timeline answers every hour it holds, as country-styles would, and leaves the rest to be asked', async () => {
  const { client, asked } = facts();
  const at = (hour: number, tags: string[]) => heldStyles(client, source, recording, hour, tags);
  assert.equal(at(12, ['GER']), undefined, 'asked for while it is read');
  await settle();
  assert.deepEqual(asked, [['country-facts', { branchId: '1' }]]);
  assert.deepEqual(at(12, ['GER'])?.get('GER'), { tag: 'GER', name: 'Germany', color: [0x33, 0x66, 0x99], warnings: ['content'] });
  assert.equal(at(15, ['GER'])?.get('GER')?.name, 'German Reich');
  assert.deepEqual(at(36, ['GER'])?.get('GER'),
    { tag: 'GER', name: 'German Reich', color: null, warnings: ['content', 'GER: no recorded map colour'] });
  assert.deepEqual(at(31, ['D01', 'ENG'])?.get('D01'), { tag: 'D01', name: null, color: null,
    warnings: ['content', 'D01: not a country at this hour'] });
  assert.deepEqual(at(31, ['D01', 'ENG'])?.get('ENG')?.warnings, ['content', 'ENG: not a country at this hour']);
  assert.equal(at(32, ['D01'])?.get('D01')?.name, 'Germany');
  for (const hour of [9, 21, 29, 41]) assert.equal(at(hour, ['GER']), undefined, String(hour));
  assert.equal(asked.length, 1, 'the timeline is read once');
  assert.deepEqual([...heldFacts(style => style.name)(heldTimeline(client, source)!, 31, ['GER', 'D01'])!], [['GER', 'German Reich']]);
  assert.equal(heldStyles(client, source, undefined, 12, ['GER']), undefined);
  forgetTimelines(client, 'timeline');
  assert.equal(at(12, ['GER']), undefined, 'a forgotten timeline is read again');
});

test('a malformed country facts reply is refused', () => {
  assert.equal(validateCountryFacts(REPLY), REPLY);
  const broken = (change: (reply: CountryFactsReply) => void) => {
    const reply = structuredClone(REPLY);
    change(reply);
    assert.throws(() => validateCountryFacts(reply), /malformed/);
  };
  broken(reply => { reply.countries[0].runs[1][1] = 2; });
  broken(reply => { reply.countries[0].runs[1][0] = 10; });
  broken(reply => { reply.known[0] = [20, 10]; });
  broken(reply => { (reply.countries[1].runs[0] as number[]).pop(); });
  broken(reply => { reply.countries[0].runs[0][4] = 2; });
  broken(reply => { (reply.flags[0].keys as unknown[]).pop(); });
  broken(reply => { (reply.flags[1].reasons as unknown[])[0] = 1; });
});

test('thousands of held hours are told through the scheduler without asking, and without growing the stack', async () => {
  const hours = 5000;
  const reply: CountryFactsReply = { branchId: '1', known: [[0, hours]], names: ['Germany'], reasons: [],
    flags: [], countries: [{ tag: 'GER', runs: [[0, 0, 0x336699, -1, -1]] }], warnings: [] };
  const asked: unknown[] = [];
  const client = {
    product: async (request: ProductRequest) => {
      if (request.product === 'country-facts') return replyTo(request, reply);
      asked.push(request);
      throw new Error('asked for an hour the timeline holds');
    },
  } as unknown as DashboardClient;
  const source = { sourceId: 'many', revision: 'r1', branchId: '1', content: 0 };
  const recording = { id: 'many', revision: 'r1' } as DashboardSource;
  heldStyles(client, source, recording, 0, ['GER']);
  await settle();
  for (const shared of [true, false]) {
    const spec: FactSpec<CountryStyle> = { product: 'country-styles', perRequest: 64, shared,
      params: (branchId, hour, tags) => ({ branchId, hour, tags }),
      extract: data => new Map((data as CountryStyleReply).styles.map(style => [style.tag, style] as const)),
      held: heldFacts(({ tag, name, color }) => ({ tag, name, color })) };
    const store = new HourStore<CountryStyle>(MAX_NAME_HOURS);
    const requests = new FactRequests<CountryStyle>(store,
      (plan, signal, onChunk) => readFacts(client, source, spec, plan, signal, onChunk, recording), () => {},
      { now: () => 0, after: () => () => {} });
    const wanted = Array.from({ length: hours }, (_, hour) => ({ hour, tag: 'GER' }));
    requests.want(wanted, Infinity, spec.perRequest);
    await settle();
    const read = requests.pickRead(wanted);
    assert.equal(read.size, hours, `${shared}`);
    assert.deepEqual(read.get(nameKey(hours - 1, 'GER')), { tag: 'GER', name: 'Germany', color: [0x33, 0x66, 0x99] });
    assert.equal(requests.running, 0);
  }
  assert.deepEqual(asked, []);
});

test('a timeline still being read survives forgetting what settled, and another content revision is read anew', async () => {
  const replies: (() => void)[] = [];
  let asked = 0;
  const client = {
    product: (request: ProductRequest) => { asked++; return new Promise(resolve => replies.push(() => resolve(replyTo(request, REPLY)))); },
  } as unknown as DashboardClient;
  const source = { sourceId: 'settled', revision: 'r1', branchId: '1', content: 0 };
  const recording = { id: 'settled', revision: 'r1' } as DashboardSource;
  assert.equal(heldStyles(client, source, recording, 12, ['GER']), undefined);
  forgetTimelines(client, 'settled', true);
  assert.equal(heldStyles(client, source, recording, 12, ['GER']), undefined);
  assert.equal(asked, 1, 'the read in flight is kept, not asked again');
  replies.shift()!();
  await settle();
  assert.equal(heldStyles(client, source, recording, 12, ['GER'])?.get('GER')?.name, 'Germany');
  assert.equal(heldStyles(client, { ...source, content: 1 }, recording, 12, ['GER']), undefined, 'new content is read anew');
  assert.equal(asked, 2);
  replies.shift()!();
  await settle();
  assert.equal(heldStyles(client, { ...source, content: 1 }, recording, 12, ['GER'])?.get('GER')?.name, 'Germany');
  assert.equal(heldStyles(client, source, recording, 12, ['GER']), undefined, 'the old content is read anew too');
  assert.equal(asked, 3);
  replies.shift()!();
  await settle();
  assert.equal(heldStyles(client, source, recording, 12, ['GER'])?.get('GER')?.name, 'Germany');
  forgetTimelines(client, 'settled', true);
  assert.equal(heldStyles(client, source, recording, 12, ['GER']), undefined, 'a settled timeline is forgotten');
  assert.equal(asked, 4);
});

test('watchers are told as a timeline arrives and as it is forgotten, and reading it never asks for it', async () => {
  const { client, asked } = facts();
  let told = 0;
  const stop = watchTimeline(client, source, () => told++);
  assert.equal(heldTimeline(client, source), undefined);
  assert.equal(asked.length, 0, 'reading what is held asks for nothing');
  heldStyles(client, source, recording, 12, ['GER']);
  assert.equal(told, 0, 'nothing held changed while it is read');
  await settle();
  assert.equal(told, 1);
  assert.equal(heldTimeline(client, source)?.reply.names[0], 'Germany');
  assert.equal(heldTimeline(client, { ...source, content: 1 }), undefined, 'another content holds nothing yet');
  heldStyles(client, source, recording, 12, ['GER']);
  assert.equal(told, 1, 'an hour it holds changes nothing');
  forgetTimelines(client, 'timeline');
  assert.equal(told, 2);
  stop();
  heldStyles(client, source, recording, 12, ['GER']);
  await settle();
  assert.equal(told, 2, 'a watcher that stopped is told nothing');
});

test('the hours a held timeline holds are answered at once, each keeping its identity, and only the rest are left to read', async () => {
  const { client } = facts();
  const wanted = [{ hour: 12, tag: 'GER' }, { hour: 12, tag: 'ENG' }, { hour: 25, tag: 'GER' }, { hour: 36, tag: 'GER' }];
  const pick = heldFacts(({ tag, name, color }) => ({ tag, name, color }));
  const none = answerHeld(pick, undefined, wanted, undefined);
  assert.equal(none.rest, wanted, 'without a timeline everything is read');
  assert.equal(none.answered.size, 0);
  heldStyles(client, source, recording, 12, ['GER']);
  await settle();
  const timeline = heldTimeline(client, source)!;
  const first = answerHeld(pick, timeline, wanted, undefined);
  assert.deepEqual(first.rest, [{ hour: 25, tag: 'GER' }], 'an hour no known span holds is read');
  assert.deepEqual(first.answered.get(nameKey(12, 'GER')), { tag: 'GER', name: 'Germany', color: [0x33, 0x66, 0x99] });
  const names = answerHeld(heldFacts(style => style.name), timeline, [{ hour: 12, tag: 'ENG' }], undefined);
  assert.equal(names.answered.get(nameKey(12, 'ENG')), null, 'a country the timeline gives nothing for is answered with nothing');
  assert.equal(first.answered.get(nameKey(36, 'GER'))?.color, null);
  const again = answerHeld(pick, timeline, [...wanted].reverse(), first.answered);
  assert.equal(again.answered.get(nameKey(12, 'GER')), first.answered.get(nameKey(12, 'GER')), 'an answer keeps its identity');
  assert.equal(answerHeld(pick, timeline, [{ hour: 13, tag: 'GER' }], first.answered).rest.length, 0, 'nothing left to read');
  assert.equal(answerHeld(undefined, timeline, wanted, undefined).rest, wanted, 'a spec without a lookup reads everything');
});

test('each run names the flag preparation chose for it, and a tag that is not a country has none, saying why', async () => {
  const { client } = facts();
  heldStyles(client, source, recording, 12, ['GER']);
  await settle();
  const timeline = heldTimeline(client, source)!;
  assert.deepEqual(flagIn(timeline, 12, 'GER'), { choice: REPLY.flags[0] });
  assert.deepEqual(flagIn(timeline, 15, 'GER'), { choice: REPLY.flags[1] });
  assert.deepEqual(flagIn(timeline, 36, 'GER'), { choice: REPLY.flags[1] }, 'a missing colour leaves the flag');
  assert.deepEqual(flagIn(timeline, 31, 'D01'), { reason: 'not a country at this hour' });
  assert.deepEqual(flagIn(timeline, 32, 'D01'), { choice: REPLY.flags[0] });
  assert.deepEqual(flagIn(timeline, 12, 'ENG'), { reason: 'not a country at this hour' });
  assert.deepEqual(flagIn(timeline, 25, 'GER'), { reason: 'country facts were not recorded at this hour' },
    'an hour a finished preparation holds no facts of says so');
});

test('identities at the head are read at the hour one last changed, which reads the same as the head', () => {
  const timeline = { reply: REPLY, runs: new Map(REPLY.countries.map(country => [country.tag, country.runs as [number, number, number, number, number][]])),
    final: true, colours: new Map() } as CountryTimeline;
  for (const tags of [['GER'], ['D01'], ['GER', 'D01']]) {
    const seen = new Set<number>();
    for (let hour = 0; hour <= 50; hour++) {
      const since = identitySince(timeline, hour, tags);
      if (since === undefined) { assert.equal(stylesIn(timeline, hour, tags), undefined, `${hour} is not held`); continue; }
      seen.add(since);
      assert.ok(since <= hour);
      assert.deepEqual(stylesIn(timeline, since, tags), stylesIn(timeline, hour, tags), `${tags} at ${hour} and at ${since}`);
      for (const tag of tags) assert.deepEqual(flagIn(timeline, since, tag), flagIn(timeline, hour, tag));
    }
    assert.deepEqual([...seen], { GER: [10, 15, 30, 35], D01: [10, 30, 32], 'GER,D01': [10, 15, 30, 32, 35] }[tags.join()], `${tags} moves only when one changes`);
  }
});
