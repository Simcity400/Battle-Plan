import assert from 'node:assert/strict';
import test from 'node:test';
import { forgetFacts, SHARED_FACT_HOURS, sharedFacts, SharedFacts } from './sharedFacts.ts';
import type { Ask, Told } from './sharedFacts';
import type { DashboardClient } from './client';

/** An ask answering each country with its tag and hour, held until released; every request kept. */
function asks(fail = new Set<string>()) {
  const asked: { hour: number; tags: readonly string[]; signal: AbortSignal; release: () => void }[] = [];
  const ask: Ask<string> = (hour, tags, signal) => new Promise<Told<string>>(resolve => {
    const release = () => resolve(tags.some(tag => fail.has(tag)) ? { failed: true }
      : new Map(tags.filter(tag => tag !== 'NONE').map(tag => [tag, `${tag}@${hour}`])));
    signal.addEventListener('abort', () => resolve({ failed: true }), { once: true });
    asked.push({ hour, tags, signal, release });
  });
  return { asked, ask };
}
const settle = () => new Promise(resolve => setTimeout(resolve, 0));

test('a country wanted by many readers is asked once, and told ones are kept for the next', async () => {
  const { asked, ask } = asks();
  const facts = new SharedFacts(ask);
  const first = facts.read(10, ['GER', 'ITA'], new AbortController().signal);
  // The second asks only for what no read asks yet, and joins the rest.
  const second = facts.read(10, ['ITA', 'JAP', 'NONE'], new AbortController().signal);
  assert.deepEqual(asked.map(each => each.tags), [['GER', 'ITA'], ['JAP', 'NONE']]);
  asked.forEach(each => each.release());
  assert.deepEqual([...(await first)!.found], [['GER', 'GER@10'], ['ITA', 'ITA@10']]);
  const joined = (await second)!;
  assert.deepEqual([...joined.found], [['ITA', 'ITA@10'], ['JAP', 'JAP@10']]);
  assert.equal(joined.failed.size, 0);
  // Told before, with or without a value: nothing is asked again.
  const again = (await facts.read(10, ['NONE', 'GER'], new AbortController().signal))!;
  assert.deepEqual([...again.found], [['GER', 'GER@10']]);
  assert.equal(asked.length, 2);
  // Another hour is its own.
  void facts.read(11, ['GER'], new AbortController().signal);
  assert.deepEqual(asked.at(-1)!.tags, ['GER']);
});

test('a request is cancelled only once no read waits on it, and its countries are then asked again', async () => {
  const { asked, ask } = asks();
  const facts = new SharedFacts(ask);
  const leaving = new AbortController(), staying = new AbortController();
  const left = facts.read(10, ['GER'], leaving.signal);
  const stays = facts.read(10, ['GER'], staying.signal);
  leaving.abort();
  assert.equal(await left, null);
  assert.equal(asked[0].signal.aborted, false, 'another read still waits on it');
  asked[0].release();
  assert.deepEqual([...(await stays)!.found], [['GER', 'GER@10']]);
  // Once every read waiting on a request leaves, it is cancelled and its countries forgotten.
  const alone = new AbortController();
  const gone = facts.read(10, ['ITA'], alone.signal);
  alone.abort();
  assert.equal(await gone, null);
  assert.equal(asked[1].signal.aborted, true);
  await settle();
  void facts.read(10, ['ITA'], new AbortController().signal);
  assert.deepEqual(asked.map(each => each.tags), [['GER'], ['ITA'], ['ITA']]);
});

test('a failure is told to its readers but never kept', async () => {
  const fail = new Set(['GER']);
  const { asked, ask } = asks(fail);
  const facts = new SharedFacts(ask);
  const failing = facts.read(10, ['GER', 'ITA'], new AbortController().signal);
  const told = facts.read(10, ['ENG'], new AbortController().signal);
  asked.forEach(each => each.release());
  const read = (await failing)!;
  assert.deepEqual([...read.failed], [['GER', true], ['ITA', true]]);
  assert.equal((await told)!.failed.size, 0);
  // Asked again, and told this time.
  fail.clear();
  const retried = facts.read(10, ['GER', 'ITA', 'ENG'], new AbortController().signal);
  assert.deepEqual(asked.at(-1)!.tags, ['GER', 'ITA']);
  asked.at(-1)!.release();
  assert.deepEqual([...(await retried)!.found].map(([tag]) => tag), ['GER', 'ITA', 'ENG']);
});

test('only the hours used most recently are kept, never one still being asked', async () => {
  const { asked, ask } = asks();
  const facts = new SharedFacts(ask);
  const pending = facts.read(0, ['GER'], new AbortController().signal);
  for (let hour = 1; hour <= SHARED_FACT_HOURS + 1; hour++) {
    const read = facts.read(hour, ['GER'], new AbortController().signal);
    asked.at(-1)!.release();
    await read;
  }
  // Hour 1, used least recently and told, is dropped; hour 0, still asked, is kept.
  const count = asked.length;
  void facts.read(1, ['GER'], new AbortController().signal);
  assert.equal(asked.length, count + 1);
  void facts.read(0, ['GER'], new AbortController().signal);
  assert.equal(asked.length, count + 1);
  asked[0].release();
  assert.deepEqual([...(await pending)!.found], [['GER', 'GER@0']]);
});

test('facts are shared per client, source, spec and scope, and forgotten with their source', () => {
  const client = {} as DashboardClient, other = {} as DashboardClient;
  const spec = {}, otherSpec = {};
  let made = 0;
  const ask = () => { made++; return asks().ask; };
  const facts = sharedFacts(client, 's', 'v|1|0', spec, ask);
  assert.equal(sharedFacts(client, 's', 'v|1|0', spec, ask), facts);
  assert.notEqual(sharedFacts(client, 's', 'v|1|1', spec, ask), facts);
  assert.notEqual(sharedFacts(client, 's', 'v|1|0', otherSpec, ask), facts);
  assert.notEqual(sharedFacts(other, 's', 'v|1|0', spec, ask), facts);
  forgetFacts(client, 's');
  assert.notEqual(sharedFacts(client, 's', 'v|1|0', spec, ask), facts);
  assert.equal(made, 5);
});
