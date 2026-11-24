import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';

const status = (patch = {}) => ({ running: true, ready: true, visible: true, viewer: { path: null, revision: null, branch: null, hour: null }, error: null, cameraMode: 'map' as const, cameraOverlays: false, ...patch });
const load = async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  return { vite, popout: await vite.ssrLoadModule('/ui/src/shell/popout.ts') as typeof import('./popout') };
};
const tick = () => new Promise<void>(resolve => setImmediate(resolve));

test('the pop-out tells its two state payloads apart and names why it shows nothing', async () => {
  const { vite, popout: { CLOSED, nextPopoutState, popoutProblem, popoutStopped } } = await load();
  try {
    assert.deepEqual(nextPopoutState(CLOSED, { generation: 0, sequence: 1, viewer: 0, status: status() }), CLOSED);
    const opened = nextPopoutState(CLOSED, { generation: 1, sequence: 2, viewer: 1, open: true, status: status({ ready: false }) });
    assert.equal(opened.open, true);
    const ready = nextPopoutState(opened, { generation: 1, sequence: 3, viewer: 1, status: status() });
    assert.equal(ready.status?.ready, true);
    assert.equal(ready.sequence, 3);
    assert.deepEqual(nextPopoutState({ ...ready, error: 'resize failed' }, { generation: 1, sequence: 4, viewer: 1, open: false, status: null }), { generation: 1, sequence: 4, viewer: 1, open: false, status: null, error: null });
    const crashed = nextPopoutState(ready, { generation: 1, sequence: 4, viewer: 1, status: status({ running: false, ready: false, error: 'Native viewer exited (1)' }) });
    assert.equal(popoutStopped(crashed), true);
    assert.equal(popoutProblem(crashed), 'The pop-out map stopped: Native viewer exited (1)');
    assert.equal(popoutProblem(nextPopoutState(ready, { generation: 1, sequence: 4, viewer: 1, status: status({ error: 'Export failed' }) })), 'Export failed');
    assert.equal(popoutProblem({ ...ready, error: 'resize failed' }), 'resize failed');
    assert.equal(popoutProblem(ready), null);
    assert.equal(popoutProblem(CLOSED), null);
    assert.equal(popoutStopped(ready), false);
  } finally { await vite.close(); }
});

test('events from a closed window never change the window opened after it', async () => {
  const { vite, popout: { CLOSED, nextPopoutState, popoutFailed } } = await load();
  try {
    const first = nextPopoutState(CLOSED, { generation: 1, sequence: 1, viewer: 1, open: true, status: status() });
    const second = nextPopoutState(first, { generation: 2, sequence: 3, viewer: 2, open: true, status: status({ ready: false }) });
    assert.deepEqual(nextPopoutState(second, { generation: 1, sequence: 4, viewer: 1, status: status({ running: false, error: 'exited' }) }), second);
    assert.deepEqual(popoutFailed(second, { generation: 1, sequence: 4, error: 'resize failed' }), second);
    assert.equal(nextPopoutState(second, { generation: 2, sequence: 4, viewer: 2, status: status() }).status?.ready, true);
    const failed = popoutFailed(second, { generation: 2, sequence: 4, error: 'resize failed' });
    assert.equal(failed.error, 'resize failed');
    assert.equal(failed.sequence, 4);
    assert.equal(nextPopoutState(second, { generation: 2, sequence: 4, viewer: 2, open: false, status: null }).open, false);
    assert.deepEqual(popoutFailed(second, { generation: 2, sequence: 3, error: 'resize failed' }), second);
  } finally { await vite.close(); }
});

test('an event older than the state shown never overwrites it', async () => {
  const { vite, popout: { CLOSED, nextPopoutState, popoutStopped } } = await load();
  try {
    const opened = nextPopoutState(CLOSED, { generation: 1, sequence: 3, viewer: 1, open: true, status: status() });
    const crashed = nextPopoutState(opened, { generation: 1, sequence: 5, viewer: 1, status: status({ running: false, ready: false, error: 'Native viewer exited (1)' }) });
    assert.equal(popoutStopped(crashed), true);
    const stale = nextPopoutState(crashed, { generation: 1, sequence: 4, viewer: 1, open: true, status: status() });
    assert.deepEqual(stale, crashed);
    assert.equal(popoutStopped(stale), true);
    assert.deepEqual(nextPopoutState(crashed, { generation: 1, sequence: 5, viewer: 1, status: status() }), crashed);
    const closed = nextPopoutState(crashed, { generation: 1, sequence: 7, viewer: 1, open: false, status: null });
    assert.deepEqual(nextPopoutState(closed, { generation: 1, sequence: 6, viewer: 1, open: true, status: status() }), closed);
    assert.deepEqual(nextPopoutState(closed, { generation: 2, sequence: 6, viewer: 2, open: true, status: status() }), closed);
    assert.equal(nextPopoutState(closed, { generation: 2, sequence: 8, viewer: 2, open: true, status: status() }).open, true);
  } finally { await vite.close(); }
});

test('the pop-out asks for its state only after both listeners are registered', async () => {
  const { vite, popout: { CLOSED, subscribePopout } } = await load();
  try {
    const log: string[] = [];
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    const registered = new Map<string, () => void>();
    const listen = <T,>(event: string, handler: (event: { payload: T }) => void) => {
      log.push(`listen ${event}`);
      handlers.set(event, handler as (event: { payload: unknown }) => void);
      return new Promise<() => void>(resolve => registered.set(event, () => resolve(() => { log.push(`unlisten ${event}`); })));
    };
    const invoke = async (command: string) => { log.push(`invoke ${command}`); };
    let state = CLOSED;
    const apply = (update: (previous: typeof CLOSED) => typeof CLOSED) => { state = update(state); };

    const stop = subscribePopout(listen, invoke, apply);
    await tick();
    assert.deepEqual(log, ['listen popout-state', 'listen popout-error']);
    registered.get('popout-state')!(); await tick();
    assert.deepEqual(log, ['listen popout-state', 'listen popout-error'], 'one listener registered is not enough');
    registered.get('popout-error')!(); await tick();
    assert.deepEqual(log, ['listen popout-state', 'listen popout-error', 'invoke popout_publish']);
    handlers.get('popout-state')!({ payload: { generation: 1, sequence: 9, viewer: 1, open: true, status: status() } });
    handlers.get('popout-state')!({ payload: { generation: 0, sequence: 8, viewer: 0, open: false, status: null } });
    assert.equal(state.open, true);
    assert.equal(state.sequence, 9);
    handlers.get('popout-error')!({ payload: { generation: 1, sequence: 10, error: 'resize failed' } });
    assert.equal(state.error, 'resize failed');
    stop(); await tick();
    assert.deepEqual(log.slice(3).sort(), ['unlisten popout-error', 'unlisten popout-state']);
    handlers.get('popout-state')!({ payload: { generation: 1, sequence: 11, viewer: 1, open: false, status: null } });
    assert.equal(state.open, true);
  } finally { await vite.close(); }
});

test('a pop-out stopped before its listeners register removes them and never asks for its state', async () => {
  const { vite, popout: { subscribePopout } } = await load();
  try {
    const log: string[] = [];
    const registered: (() => void)[] = [];
    const listen = (event: string) => new Promise<() => void>(resolve => registered.push(() => resolve(() => { log.push(`unlisten ${event}`); })));
    const invoke = async (command: string) => { log.push(`invoke ${command}`); };
    const stop = subscribePopout(listen, invoke, () => assert.fail('no update after stopping'));
    stop();
    for (const resolve of registered) resolve();
    await tick();
    assert.deepEqual(log.sort(), ['unlisten popout-error', 'unlisten popout-state']);
  } finally { await vite.close(); }
});

test('every new pop-out window is sent the options and the clock again', async () => {
  const { vite, popout: { CLOSED, nextPopoutState, popoutClockKey, popoutConfigKey } } = await load();
  try {
    const first = nextPopoutState(CLOSED, { generation: 1, sequence: 1, viewer: 1, open: true, status: status() });
    assert.equal(popoutClockKey(first), '1:1');
    assert.equal(popoutConfigKey(first), '1:1');
    const closed = nextPopoutState(first, { generation: 1, sequence: 2, viewer: 1, open: false, status: null });
    const second = nextPopoutState(nextPopoutState(closed, { generation: 2, sequence: 3, viewer: 2, open: true, status: status({ ready: false }) }), { generation: 2, sequence: 4, viewer: 2, status: status() });
    assert.deepEqual([second.open, second.status?.ready, second.status?.running], [first.open, first.status?.ready, first.status?.running]);
    assert.equal(popoutClockKey(second), '2:2');
    assert.equal(popoutConfigKey(second), '2:2');
    assert.equal(popoutClockKey(closed), null);
    assert.equal(popoutConfigKey(closed), null);
    assert.equal(popoutClockKey(CLOSED), null);
    const starting = nextPopoutState(second, { generation: 2, sequence: 5, viewer: 2, status: status({ ready: false }) });
    assert.equal(popoutClockKey(starting), '2:2');
    assert.equal(popoutConfigKey(starting), null);
    const stopped = nextPopoutState(second, { generation: 2, sequence: 5, viewer: 2, status: status({ running: false, ready: false, error: 'exited' }) });
    assert.equal(popoutClockKey(stopped), null);
    assert.equal(popoutConfigKey(stopped), null);
    const restarted = nextPopoutState(second, { generation: 2, sequence: 6, viewer: 3, status: status() });
    assert.deepEqual([restarted.status?.running, restarted.status?.ready], [second.status?.running, second.status?.ready]);
    assert.equal(popoutClockKey(restarted), '2:3');
    assert.equal(popoutConfigKey(restarted), '2:3');
  } finally { await vite.close(); }
});

test('a whole state keeps a window error the window has not recovered from', async () => {
  const { vite, popout: { CLOSED, nextPopoutState, popoutFailed, popoutProblem } } = await load();
  try {
    const opened = nextPopoutState(CLOSED, { generation: 1, sequence: 1, viewer: 1, open: true, status: status() });
    const failed = popoutFailed(opened, { generation: 1, sequence: 2, error: 'resize failed' });
    const republished = nextPopoutState(failed, { generation: 1, sequence: 3, viewer: 1, open: true, status: status(), error: 'resize failed' });
    assert.equal(popoutProblem(republished), 'resize failed');
    const recovered = nextPopoutState(republished, { generation: 1, sequence: 4, viewer: 1, open: true, status: status(), error: null });
    assert.equal(popoutProblem(recovered), null);
  } finally { await vite.close(); }
});

test('a restarted or reopened pop-out is sent the hour and playback chosen meanwhile', async () => {
  const { vite, popout: { CLOSED, nextPopoutState, popoutClockKey } } = await load();
  try {
    const { followMapPlayback } = await vite.ssrLoadModule('/ui/src/dashboard/mapPlayback.ts') as typeof import('../dashboard/mapPlayback');
    const track = { source: { path: 'campaign.battleplan', revision: 'r' }, branch: { id: '1', firstHour: 0, lastHour: 100 }, axisOffset: 0 } as Parameters<typeof followMapPlayback>[1];
    let snapshot = { hour: 10, fraction: 0.25, hoursPerSecond: 1440, playing: true, seekSequence: 0 };
    const listeners = new Set<() => void>();
    const clock = { getSnapshot: () => snapshot, composedAt: () => 0, clockId: 'page-clock-1', subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; }, subscribeFrame: () => () => {} };
    const move = (next: typeof snapshot) => { snapshot = next; for (const listener of listeners) listener(); };
    let running: number | null = 1;
    const shown: { generation: number; hour: number; playing: boolean; seekSequence: number }[] = [];
    const seekPopout = async (generation: number, _path: string, _revision: string, _branch: string, hour: number, playback: import('../dashboard/mapPlayback').MapPlayback) => {
      if (running !== generation) throw new Error('The pop-out map stopped');
      shown.push({ generation, hour, ...playback });
    };
    let key: string | null = null;
    let stop: (() => void) | undefined;
    const render = (state: Parameters<typeof popoutClockKey>[0]) => {
      const next = popoutClockKey(state);
      if (next === key) return;
      stop?.(); stop = undefined; key = next;
      const generation = state.generation;
      if (next != null) stop = followMapPlayback(clock, track, (...args) => seekPopout(generation, ...args), () => {});
    };
    let state = nextPopoutState(CLOSED, { generation: 1, sequence: 1, viewer: 1, open: true, status: status() });
    render(state); await tick();
    assert.deepEqual(shown.at(-1), { generation: 1, hour: 10, playing: true, seekSequence: 0, fraction: 0.25, hoursPerSecond: 1440, composedAt: 0, clockId: 'page-clock-1' });
    running = null;
    state = nextPopoutState(state, { generation: 1, sequence: 2, viewer: 1, status: status({ running: false, ready: false, error: 'Native viewer exited (1)' }) });
    render(state);
    move({ ...snapshot, hour: 40, playing: false, seekSequence: 1, fraction: 0.25, hoursPerSecond: 1440 }); await tick();
    running = 1;
    state = nextPopoutState(state, { generation: 1, sequence: 3, viewer: 2, open: true, status: status({ ready: false }) });
    render(state); await tick();
    assert.deepEqual(shown.at(-1), { generation: 1, hour: 40, playing: false, seekSequence: 1, fraction: 0.25, hoursPerSecond: 1440, composedAt: 0, clockId: 'page-clock-1' });
    running = 2;
    state = nextPopoutState(nextPopoutState(state, { generation: 1, sequence: 4, viewer: 1, open: false, status: null }), { generation: 2, sequence: 5, viewer: 2, open: true, status: status() });
    render(state); await tick();
    assert.deepEqual(shown.at(-1), { generation: 2, hour: 40, playing: false, seekSequence: 1, fraction: 0.25, hoursPerSecond: 1440, composedAt: 0, clockId: 'page-clock-1' });
    stop?.();
  } finally { await vite.close(); }
});

