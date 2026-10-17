import assert from 'node:assert/strict';
import test from 'node:test';
import { PAUSED_POLL_MS, PreparationController } from './controller.ts';
import { progressPercent, validatePreparationReply, validatePreparationPreferences } from './contract.ts';
import type { PreparationReply, PreparationRequest } from './contract.ts';
import type { DashboardClient, DashboardSource } from '../../dashboard/client.ts';
const source = { id: 'source-1', revision: 'r1', path: 'modded.battleplan', metadata: {} } as DashboardSource;
const reply = (query: PreparationRequest, state: PreparationReply['state'], done = '0'): PreparationReply => ({ ...query, state, backgroundEnabled: true, bytes: '9007199254740993', error: null, sections: [{ id: 'mod-section', label: 'Mod section', state, done, total: '9007199254740993', error: null }] });
const settle = () => new Promise(resolve => setImmediate(resolve));
function fixture() {
  const calls: string[] = [];
  let state: PreparationReply['state'] = 'idle';
  let completed = '0';
  const client = {
    openSource: async () => { calls.push('open'); return source; }, releaseSource: async (id: string) => { calls.push(`release:${id}`); },
    preparationStatus: async (query: PreparationRequest) => { calls.push('status'); return reply(query, state, completed); },
    prepareSource: async (query: PreparationRequest) => { calls.push('start'); state = 'preparing'; return reply(query, state, completed); },
    cancelPreparation: async (query: PreparationRequest) => { calls.push('cancel'); state = 'cancelled'; return reply(query, state, completed); },
    clearPreparation: async (query: PreparationRequest) => { calls.push('clear'); state = 'idle'; return { ...reply(query, state), backgroundEnabled: true, bytes: '0' }; },
  } as unknown as DashboardClient;
  return { client, calls, controller: new PreparationController(client), complete() { state = 'ready'; completed = '9007199254740993'; } };
}
test('visible preparation reads backend progress, cancellation and Clear, without invented progress', async () => {
  const f = fixture();
  try {
    await f.controller.start(source.path, source);
    assert.equal(f.controller.getSnapshot().reply?.state, 'idle');
    assert.deepEqual(f.calls, ['status']);
    await f.controller.command('start');
    assert.equal(f.controller.getSnapshot().reply?.sections[0].done, '0');
    f.complete(); await f.controller.command('status');
    assert.equal(f.controller.getSnapshot().reply?.state, 'ready');
    assert.equal(f.controller.getSnapshot().reply?.sections[0].done, '9007199254740993');
    await f.controller.command('start'); await f.controller.command('cancel');
    assert.equal(f.controller.getSnapshot().reply?.state, 'cancelled');
    await f.controller.command('clear');
    assert.equal(f.controller.getSnapshot().reply?.bytes, '0');
    assert.equal(f.controller.getSnapshot().reply?.state, 'idle');
    assert.deepEqual(f.calls, ['status', 'start', 'status', 'start', 'cancel', 'clear']);
  } finally { f.controller.stop(); }
  assert.ok(!f.calls.some(call => call.startsWith('release')));
});
test('temporary library source is released on close without cancelling the backend job', async () => {
  const f = fixture(); await f.controller.start(source.path, null); await f.controller.command('start'); f.controller.stop(); await settle();
  assert.deepEqual(f.calls, ['open', 'status', 'start', 'release:source-1']);
});
test('late status cannot undo Clear or cross source/request identity', async () => {
  const f = fixture(); await f.controller.start(source.path, source);
  let resolve!: (value: PreparationReply) => void;
  let staleQuery!: PreparationRequest;
  f.client.preparationStatus = async query => { staleQuery = query; return new Promise(done => { resolve = done; }); };
  const old = f.controller.command('status');
  await f.controller.command('clear'); resolve(reply(staleQuery, 'ready')); await old;
  assert.equal(f.controller.getSnapshot().reply?.state, 'idle');
  f.client.preparationStatus = async query => ({ ...reply(query, 'ready'), revision: 'wrong' });
  await f.controller.command('status');
  assert.match(f.controller.getSnapshot().error!, /does not match/);
  assert.equal(f.controller.getSnapshot().reply?.state, 'idle'); f.controller.stop();
});
test('source opened after panel closes is released and never queried', async () => {
  const f = fixture(); let resolve!: (value: DashboardSource) => void;
  f.client.openSource = async () => new Promise(done => { resolve = done; });
  const opening = f.controller.start(source.path, null); f.controller.stop(); resolve(source); await opening;
  assert.deepEqual(f.calls, ['release:source-1']);
});
test('progress uses exact decimal counts and validates all echoed identities', () => {
  const query = { sourceId: 's', revision: 'r', requestId: 'q' };
  assert.equal(progressPercent('9007199254740993', '18014398509481986'), 50);
  assert.equal(progressPercent('1', null), undefined); assert.equal(progressPercent('1', '0'), undefined);
  assert.equal(progressPercent('8', '4'), 100);
  for (const field of ['sourceId', 'revision', 'requestId']) assert.throws(() => validatePreparationReply({ ...reply(query, 'ready'), [field]: 'other' }, query), /does not match/);
  assert.throws(() => validatePreparationReply({ ...reply(query, 'ready'), backgroundEnabled: true, bytes: '-1' }, query), /Invalid/);
});

test('only backend-active work is polled and terminal work sleeps', async context => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const f = fixture();
  try {
    await f.controller.start(source.path, source); context.mock.timers.tick(5000); await settle();
    assert.deepEqual(f.calls, ['status']);
    await f.controller.command('start'); context.mock.timers.tick(500); await settle();
    assert.deepEqual(f.calls, ['status', 'start', 'status']);
    f.complete(); context.mock.timers.tick(500); await settle();
    assert.equal(f.controller.getSnapshot().reply?.state, 'ready');
    const reads = f.calls.length; context.mock.timers.tick(5000); await settle(); assert.equal(f.calls.length, reads);
  } finally { f.controller.stop(); }
});

test('a paused panel checks back slowly and follows the automatic resume', async context => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const f = fixture();
  let state: PreparationReply['state'] = 'paused';
  f.client.preparationStatus = async query => { f.calls.push('status'); return { ...reply(query, state), pauseReason: state === 'paused' ? 'game' : null }; };
  try {
    await f.controller.start(source.path, source);
    assert.equal(f.controller.getSnapshot().reply?.state, 'paused');
    context.mock.timers.tick(PAUSED_POLL_MS - 1); await settle();
    assert.deepEqual(f.calls, ['status'], 'no fast polling while paused');
    context.mock.timers.tick(1); await settle();
    assert.deepEqual(f.calls, ['status', 'status']);
    // The game exits and the library scan resumes the job.
    state = 'preparing';
    context.mock.timers.tick(PAUSED_POLL_MS); await settle();
    assert.equal(f.controller.getSnapshot().reply?.state, 'preparing');
    context.mock.timers.tick(500); await settle();
    assert.equal(f.calls.length, 4, 'active work is polled again');
  } finally { f.controller.stop(); }
});

test('background preference is explicit in preferences and progress replies', () => {
  assert.deepEqual(validatePreparationPreferences({ backgroundEnabled: false }), { backgroundEnabled: false });
  assert.throws(() => validatePreparationPreferences({} as any), /Invalid background/);
  const request = { sourceId: 's', revision: 'r', requestId: 'q' };
  assert.equal(validatePreparationReply({ ...reply(request, 'idle'), backgroundEnabled: false }, request).backgroundEnabled, false);
  const incomplete = reply(request, 'idle') as any;
  delete incomplete.backgroundEnabled;
  assert.throws(() => validatePreparationReply(incomplete, request), /Invalid preparation/);
});

test('paused preparation retains family coverage and validates both fields', () => {
  const request = { sourceId: 's', revision: 'r', requestId: 'q' };
  const paused = { ...reply(request, 'paused'), pauseReason: 'game' as const,
    families: { wars: { '1': { firstHour: 10, lastHour: 11, complete: false } } } };
  assert.equal(validatePreparationReply(paused, request), paused);
  assert.throws(() => validatePreparationReply({ ...paused, pauseReason: 'unknown' } as any, request), /Invalid/);
  assert.throws(() => validatePreparationReply({ ...paused, families: { wars: { '1': { firstHour: 11, lastHour: 10, complete: false } } } }, request), /Invalid/);
});
