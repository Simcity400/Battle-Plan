import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { drawFlags, drawnFlag, FLAG_BATCH, flagReadKey, forgetFlags, retryFlags, useFlagReads } from './countryFlags.ts';
import type { FlagValue } from './countryFlags';
import { FAILED_FACT, retryDelay } from './countryFacts.ts';
import { heldStyles } from './countryTimeline.ts';
import type { CountryFactsReply, CountryFlagChoice, CountryFlagParams } from '../generated/dashboard';
import type { DashboardClient, DashboardSource, ProductRequest } from './client';

const recording = { id: 'flags', revision: 'r1' } as DashboardSource;
const settle = () => new Promise(resolve => setImmediate(resolve));
const choice = (key: string | null, reason: string | null = null): CountryFlagChoice => ({ keys: [key, key, key], reasons: [reason, reason, reason] });
const url = (index: number) => `http://artwork.localhost/${String(index).padStart(32, '0')}.png`;

/** A client drawing every choice it is asked for, but refusing `refused` keys and answering nothing for `unanswered`. */
function drawing(refused = new Set<string>(), unanswered = new Set<string>(), fail?: () => Error) {
  const asked: CountryFlagChoice[][] = [];
  const client = {
    product: async (request: ProductRequest<CountryFlagParams>) => {
      if (request.product !== 'country-flags') throw new Error(`unexpected ${request.product}`);
      asked.push(request.params.flags);
      if (fail) throw fail();
      const flags = request.params.flags.flatMap((flag, index) => flag.keys[0] && !refused.has(flag.keys[0]) && !unanswered.has(flag.keys[0])
        ? [{ flag: index, asset: `gfx/flags/${flag.keys[0]}.tga`, key: `${flag.keys[0]}#1`, width: 82, height: 52, png: url(index) }] : []);
      const unavailable = request.params.flags.flatMap((flag, index) => flag.keys[0] && refused.has(flag.keys[0])
        ? [{ flag: index, reason: `${flag.keys[0]} could not be decoded` }] : !flag.keys[0] ? [{ flag: index, reason: flag.reasons[0] ?? 'none' }] : []);
      return { ...request, data: { size: request.params.size, flags, unavailable, warnings: [] } };
    },
  } as unknown as DashboardClient;
  return { client, asked };
}

test('each distinct flag is drawn once, in requests of at most the batch, whoever asks for it', async () => {
  const { client, asked } = drawing();
  const choices = Array.from({ length: 40 }, (_, index) => choice(`T${index}`));
  drawFlags(client, recording, 0, 'panel', choices);
  // The same flags again, as other readers would ask for them, and one equal by value only.
  drawFlags(client, recording, 0, 'panel', [...choices.slice(0, 5), choice('T7')]);
  await settle(); await settle();
  assert.deepEqual(asked.map(batch => batch.length), [FLAG_BATCH, 40 - FLAG_BATCH]);
  const drawn = drawnFlag(client, recording, 0, 'panel', choice('T39'));
  assert.ok(drawn && drawn !== FAILED_FACT);
  assert.equal(drawn.url, drawn.image?.png);
  assert.equal(drawn.image?.asset, 'gfx/flags/T39.tga');
  drawFlags(client, recording, 0, 'panel', choices);
  await settle();
  assert.equal(asked.length, 2, 'a drawn flag is never asked for again');
  assert.equal(drawnFlag(client, recording, 0, 'small', choices[0]), undefined, 'another size is another drawing');
  assert.equal(drawnFlag(client, recording, 1, 'panel', choices[0]), undefined, 'another content revision is another drawing');
});

test('a refused flag keeps its reason, one with no key the reason preparation gave, and an unanswered one is drawn with nothing', async () => {
  const { client } = drawing(new Set(['BAD']), new Set(['GONE']));
  const none = choice(null, 'flag artwork unavailable in the recorded content');
  drawFlags(client, recording, 0, 'panel', [choice('BAD'), none, choice('GONE')]);
  await settle(); await settle();
  assert.deepEqual(drawnFlag(client, recording, 0, 'panel', choice('BAD')), { reason: 'BAD could not be decoded' });
  assert.deepEqual(drawnFlag(client, recording, 0, 'panel', none), { reason: 'flag artwork unavailable in the recorded content' });
  assert.equal(drawnFlag(client, recording, 0, 'panel', choice('GONE')), null);
});

test('a failed drawing stays failed until retried, and one that failed for want of room is drawn again after its wait', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let failure: (() => Error) | undefined = () => new Error('Invalid country flag selection');
  const asked: number[] = [];
  const { client: answering } = drawing();
  const client = { product: async (request: ProductRequest) => {
    asked.push(1);
    if (failure) throw failure();
    return (answering as unknown as { product: (request: ProductRequest) => Promise<unknown> }).product(request);
  } } as unknown as DashboardClient;
  drawFlags(client, recording, 0, 'panel', [choice('A')]);
  await settle(); await settle();
  assert.equal(drawnFlag(client, recording, 0, 'panel', choice('A')), FAILED_FACT);
  t.mock.timers.tick(60_000);
  await settle();
  assert.equal(drawnFlag(client, recording, 0, 'panel', choice('A')), FAILED_FACT, 'a refusal waits for a retry');
  drawFlags(client, recording, 0, 'panel', [choice('A')]);
  await settle();
  assert.equal(asked.length, 1, 'nor is it asked again unbidden');
  retryFlags(client, recording.id);
  assert.equal(drawnFlag(client, recording, 0, 'panel', choice('A')), undefined);
  failure = () => new TypeError('Failed to fetch');
  drawFlags(client, recording, 0, 'panel', [choice('A')]);
  await settle(); await settle();
  assert.equal(drawnFlag(client, recording, 0, 'panel', choice('A')), FAILED_FACT);
  t.mock.timers.tick(retryDelay(1));
  assert.equal(drawnFlag(client, recording, 0, 'panel', choice('A')), undefined, 'a transient failure is let go after its wait');
  failure = undefined;
  drawFlags(client, recording, 0, 'panel', [choice('A')]);
  await settle(); await settle();
  assert.equal((drawnFlag(client, recording, 0, 'panel', choice('A')) as { url?: string }).url, url(0));
});

test('forgetting a recording drops its drawings and stops those being drawn, and another content revision starts afresh', async () => {
  const replies: (() => void)[] = [];
  const asked: string[] = [];
  const { client: answering } = drawing();
  const client = { product: (request: ProductRequest, signal: AbortSignal) => {
    asked.push(request.sourceId);
    return new Promise((resolve, reject) => {
      signal.addEventListener('abort', () => reject(new DOMException('Request cancelled', 'AbortError')));
      replies.push(() => resolve((answering as unknown as { product: (request: ProductRequest) => Promise<unknown> }).product(request)));
    });
  } } as unknown as DashboardClient;
  const other = { id: 'other', revision: 'r1' } as DashboardSource;
  drawFlags(client, recording, 0, 'panel', [choice('A')]);
  drawFlags(client, other, 0, 'panel', [choice('B')]);
  forgetFlags(client, recording.id);
  for (const reply of replies.splice(0)) reply();
  await settle(); await settle();
  assert.equal(drawnFlag(client, recording, 0, 'panel', choice('A')), undefined, 'a forgotten draw keeps nothing');
  assert.ok(drawnFlag(client, other, 0, 'panel', choice('B')), 'another recording keeps its own');
  drawFlags(client, recording, 0, 'panel', [choice('A')]);
  await settle();
  for (const reply of replies.splice(0)) reply();
  await settle(); await settle();
  assert.ok(drawnFlag(client, recording, 0, 'panel', choice('A')));
  forgetFlags(client, recording.id);
  assert.equal(drawnFlag(client, recording, 0, 'panel', choice('A')), undefined);
  drawFlags(client, recording, 1, 'panel', [choice('A')]);
  assert.equal(drawnFlag(client, other, 0, 'panel', choice('B')), undefined, 'another content revision keeps nothing drawn under the last');
  assert.deepEqual(asked, ['flags', 'other', 'flags', 'flags']);
});

test('a flag is read from the held timeline at its hour and shown once drawn; an hour not held is pending', async () => {
  const reply: CountryFactsReply = { branchId: '1', known: [[10, 20]], names: ['Germany'], reasons: ['not a country at this hour'],
    flags: [choice('GER_democratic'), choice('GER_fascism')],
    countries: [{ tag: 'GER', runs: [[10, 0, 0x336699, -1, 0], [15, 0, 0x336699, -1, 1]] }, { tag: 'D01', runs: [[10, -1, -1, 0, -1]] }], warnings: [] };
  const { client: answering, asked } = drawing();
  const client = Object.assign(answering, {
    preparedQuery: async () => ({ ...reply, coverage: { firstHour: 10, lastHour: 20, complete: true }, pending: [] }),
  });
  const source = { sourceId: recording.id, revision: recording.revision, branchId: '1', content: 0 };
  heldStyles(client, source, recording, 12, ['GER']);
  await settle();
  drawFlags(client, recording, 0, 'panel', [reply.flags[1]]);
  await settle(); await settle();
  const reads = [{ tag: 'GER', hour: 12 }, { tag: 'GER', hour: 16 }, { tag: 'D01', hour: 12 }, { tag: 'GER', hour: 30 }].map(read => ({ ...read, branchId: '1' }));
  let shown: ReadonlyMap<string, FlagValue> = new Map();
  const Probe = () => { shown = useFlagReads(client, recording, reads, true, 'panel'); return null; };
  renderToStaticMarkup(createElement(Probe));
  assert.equal(shown.get(flagReadKey(reads[0])), undefined, 'a flag not yet drawn is pending');
  assert.equal((shown.get(flagReadKey(reads[1])) as { url?: string }).url, url(0), 'the later run draws its own flag');
  assert.deepEqual(shown.get(flagReadKey(reads[2])), { reason: 'not a country at this hour' });
  assert.equal(shown.get(flagReadKey(reads[3])), undefined, 'an hour the timeline does not hold is pending');
  assert.equal(asked.length, 1, 'rendering asks for nothing');
});

test('lane looks show each country as the timeline names it and its flag once drawn: drawn, refused, failed or pending, and none for sorting-only names', async () => {
  const { useCountriesAt } = await import('./useCountriesAt.ts');
  const reply: CountryFactsReply = { branchId: '1', known: [[10, 20]], names: ['Germany', 'England', 'Italy', 'France', 'Soviet Union'], reasons: [],
    flags: ['GER', 'ENG', 'ITA', 'FRA', 'SOV'].map(tag => choice(tag)),
    countries: ['GER', 'ENG', 'ITA', 'FRA', 'SOV'].map((tag, index) => ({ tag, runs: [[10, index, 0x336699, -1, index]] })), warnings: [] };
  const asked: string[] = [];
  const { client: answering } = drawing(new Set(['ENG']));
  const client = {
    preparedQuery: async () => ({ ...reply, coverage: { firstHour: 10, lastHour: 20, complete: true }, pending: [] }),
    product: async (request: ProductRequest<CountryFlagParams>) => {
      asked.push(request.product);
      if (request.params.flags?.some(flag => flag.keys[0] === 'ITA')) throw new Error('Invalid country flag key');
      return (answering as unknown as { product: (request: ProductRequest) => Promise<unknown> }).product(request);
    },
  } as unknown as DashboardClient;
  const lanes = { id: 'lanes', revision: 'r1' } as DashboardSource;
  heldStyles(client, { sourceId: lanes.id, revision: lanes.revision, branchId: '1', content: 0 }, lanes, 12, ['GER']);
  await settle();
  for (const index of [0, 1, 2]) drawFlags(client, lanes, 0, 'panel', [reply.flags[index]]);
  await settle(); await settle();
  const track = { source: lanes, branch: { id: '1', firstHour: 10, lastHour: 20, gaps: [] } } as never;
  let lane: ReturnType<typeof useCountriesAt> | undefined;
  const Probe = () => { lane = useCountriesAt(client, track, ['GER', 'ENG', 'ITA', 'FRA'], 12, true, ['GER', 'ENG', 'ITA', 'FRA', 'SOV']); return null; };
  renderToStaticMarkup(createElement(Probe));
  const country = lane!.country;
  assert.equal(country('GER').name, 'Germany');
  assert.equal(country('GER').flag, url(0));
  assert.equal(country('GER').flagPending, false);
  assert.equal(country('ENG').flag, undefined);
  assert.equal(country('ENG').flagReason, 'ENG could not be decoded');
  assert.equal(country('ITA').flagReason, 'The flag request failed.');
  assert.equal(country('FRA').flagPending, true, 'a flag not yet drawn is pending');
  assert.equal(country('SOV').name, 'Soviet Union');
  assert.equal(country('SOV').flagPending, false, 'a country named only for sorting asks for no flag');
  assert.deepEqual(asked, ['country-flags', 'country-flags', 'country-flags'], 'names the timeline holds are never read hour by hour');
});
