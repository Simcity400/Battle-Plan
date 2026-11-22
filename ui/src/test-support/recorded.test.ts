import assert from 'node:assert/strict';
import test from 'node:test';
import { RECORDED_KIND, RECORDED_VERSION, RecordedClient, recordedRefusal, remapMessage } from './recorded.ts';
import type { RecordedEntry, RecordedFormat } from './recorded.ts';
import { datedPanel, message } from '../dashboard/follow/messages.fixture.ts';
import { parseFollowed, parseDatedPanels } from '../dashboard/follow/message.ts';
import { RefusedError } from '../dashboard/queryError.ts';
import { CELLS } from '../dashboard/follow/liveCells.ts';

const TABLES = ['0 Clock 0 hour:I32', '2 Countries 32 canonical_slot:U32,tag:Bytes', '514 GarrisonStrengths 64 value:I64'];
const tree = (lines: string[], digest: string): RecordedFormat => ({ file: 'replies.jsonl.gz', formatFile: 'crates/replay-data/src/rows/registry.golden', digest, lines });
const header = (lines: string[], digest: string) => ({ kind: RECORDED_KIND, version: RECORDED_VERSION, formatDigest: digest, format: lines, app: { revision: 'fed926180f1c59ac' }, recordedAt: '' });

test('replies recorded with another recording format are refused, naming the tables added, removed and changed', () => {
  assert.equal(recordedRefusal('replies.jsonl.gz', header(TABLES, 'a'), tree(TABLES, 'a')), null, 'the same format is served');
  const now = ['0 Clock 0 hour:I32', '2 Countries 32 canonical_slot:U32,tag:Bytes,stability:I64', '515 SupplyHealth 32 value:I64'];
  const refused = recordedRefusal('replies.jsonl.gz', header(TABLES, 'a'), tree(now, 'b'));
  assert.ok(refused, 'another format is refused');
  assert.match(refused, /replies\.jsonl\.gz was recorded by app fed926180f/);
  assert.match(refused, /added 515 SupplyHealth; removed 514 GarrisonStrengths; changed 2 Countries/);
  assert.match(refused, /record the replies again/);
  const previous = { ...header(TABLES, 'a'), version: 2 };
  assert.match(recordedRefusal('old.json', previous, tree(TABLES, 'a')) ?? '', /not recorded replies of version 4/);
});

test('a recorded message is played under the ids this page follows the same queries by: others left out, live cells kept', () => {
  const answer = { keys: [1n], columns: 1, values: [7n] };
  const bytes = message(500, [{ id: 3, answer }, { id: 9, answer }, { id: CELLS + 4, answer }]);
  const mapped = remapMessage(bytes, id => id === 3 ? 41 : undefined);
  assert.ok(mapped);
  const read = parseFollowed(mapped);
  assert.equal(read.hour, 500);
  assert.deepEqual(read.answers.map(item => item.id), [41, CELLS + 4], 'the app\'s 3 is the page\'s 41; 9 is followed by no widget here');
  assert.equal(remapMessage(message(500, [{ id: 9, answer }]), () => undefined), null, 'a message of nothing followed here is not delivered');
  const panels = datedPanel(3, 500, { rows: [] });
  assert.deepEqual(parseDatedPanels(remapMessage(panels, id => id === 3 ? 41 : undefined)!).panels.map(panel => panel.id), [41]);
});

test('the recorded client answers a request the app made, whatever order its params were built in, and fails one it never made by name', async () => {
  const query = (product: string, params: unknown) => ({ query: { sourceId: 's1', revision: 'r1', product, params } });
  const entries: RecordedEntry[] = [
    { t: 'product', at: 1, ms: 1, head: 0, args: query('series-state', { hour: 10, family: { kind: 'country', measure: 'pp' } }), ok: true,
      reply: { sourceId: 's1', revision: 'r1', product: 'series-state', data: { states: [1] } } },
    { t: 'product', at: 2, ms: 1, head: 0, args: query('wars', { hour: 10 }), ok: false, reply: { kind: 'refused', text: 'Wars are not recorded' } },
    { t: 'product', at: 3, ms: 1, head: 0, args: query('follow', { id: 3, branchId: '1', clock: 0, query: { product: 'army-size' } }), ok: true,
      reply: { sourceId: 's1', revision: 'r1', product: 'follow', data: { available: true } } },
  ];
  const anchor = (at: number, clock: number, epoch: number, playing: boolean): RecordedEntry => ({
    t: 'product', at, ms: 1, head: 12, args: query('follow-clock', { branchId: '1', clock, playing }), ok: true,
    reply: { product: 'follow-clock', data: { route: clock + 2, epoch, generation: epoch } },
  });
  entries.push(anchor(5, 0, 7, false), anchor(8, 0, 8, true), anchor(9, 1, 11, true), anchor(20, 0, 15, false),
    { t: 'product', at: 22, ms: 1, head: 12, args: query('unfollow', { id: 3 }), ok: true, reply: { data: { available: true } } });
  const client = new RecordedClient(entries);
  client.at = 6;
  client.windowEnd = 12;
  const signal = new AbortController().signal;
  const ask = (product: string, params: unknown) => client.product<unknown>({ sourceId: 'here', revision: 'now', product, params }, signal);
  const reply = await ask('series-state', { family: { measure: 'pp', kind: 'country' }, hour: 10 });
  assert.deepEqual(reply, { sourceId: 'here', revision: 'now', product: 'series-state', data: { states: [1] } }, 'answered under this page\'s source');
  await assert.rejects(ask('wars', { hour: 10 }), (error: unknown) => error instanceof RefusedError && /not recorded/.test(error.message), 'a recorded refusal is refused again');
  await assert.rejects(ask('series-state', { hour: 11, family: { kind: 'country', measure: 'pp' } }), /Not recorded: series-state/);
  assert.deepEqual(client.misses.map(miss => miss.product), ['series-state'], 'the request the app never made is counted');
  assert.deepEqual((await ask('follow', { id: 41, branchId: '1', clock: 0, query: { product: 'army-size' } })).data, { available: true });
  const clock = (playing: boolean) => ask('follow-clock', { branchId: '1', clock: 0, playing });
  assert.deepEqual((await clock(false)).data, { route: 2, epoch: 7, generation: 7 });
  assert.deepEqual((await clock(true)).data, { route: 2, epoch: 8, generation: 8 });
  const { port1, port2 } = new MessageChannel();
  await client.openFollow([port1]);
  const delivered = new Promise<number[]>(resolve => {
    port2.onmessage = ({ data }: MessageEvent<{ kind: string; bytes: Uint8Array }>) => resolve(parseFollowed(data.bytes).answers.map(answer => answer.id));
  });
  assert.ok(client.deliver(message(12, [{ id: 3, answer: { keys: [1n], columns: 1, values: [2n] } }])));
  try {
    assert.deepEqual(await delivered, [41]);
    await ask('unfollow', { id: 41 });
    assert.equal(client.deliver(message(13, [{ id: 3, answer: { notRecorded: 'Unavailable' } }])), false);
  }
  finally { port1.close(); port2.close(); }
});
