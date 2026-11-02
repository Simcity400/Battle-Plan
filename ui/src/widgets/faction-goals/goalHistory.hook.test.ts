import assert from 'node:assert/strict';
import test from 'node:test';
import { act, createElement } from 'react';
import { createRoot } from 'react-dom/client';
import type { FactionGoalsReply } from '../../generated/dashboard';
import { mountModule } from '../../dashboard/pendingGroups.fixture.ts';
import type { GoalPart } from './goalParts.ts';

type Asked = { params: { faction: string; firstHour: number | null; lastHour: number | null }; signal: AbortSignal;
  resolve: (data: unknown) => void; reject: (error: Error) => void; publish: (data: unknown) => void };
type History = { parts: readonly GoalPart[]; error?: Error; retry: () => void };
type Host = typeof globalThis & { __history?: { requests: Asked[]; latest?: History }; IS_REACT_ACT_ENVIRONMENT?: boolean };

const OVER = () => new Error('This product reply exceeds the transport reply budget of 25165824 bytes');
const reply = (name: string) => ({ warnings: [name] }) as unknown as FactionGoalsReply;
const ranges = (parts: readonly GoalPart[]) => parts.map(part => [part.firstHour, part.lastHour, part.data?.warnings[0] ?? null, part.error ?? null]);

test('the real hook replaces only a refused range, keeps its siblings\' requests and data, and cancels on a switch', async () => {
  const host = globalThis as Host;
  const state: { requests: Asked[]; latest?: History } = { requests: [] };
  host.__history = state;
  const { module, close } = await mountModule<typeof import('./goalHistory.ts')>('/ui/src/widgets/faction-goals/goalHistory.ts', {
    '/dashboard/products.ts': `export const loadProduct = (_client, request, signal, _validate, _source, _restart, publish) => new Promise((resolve, reject) =>
      globalThis.__history.requests.push({ params: request.params, signal, resolve, reject, publish }));`,
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
  });
  // React renders a component that draws nothing into this container; the hook's effects run as in the app.
  const listeners = { addEventListener() {}, removeEventListener() {} };
  const globals = host as unknown as Record<string, unknown>;
  globals.window ??= globalThis;
  globals.HTMLIFrameElement ??= class {};
  globals.document ??= { nodeType: 9, ...listeners, defaultView: globalThis, activeElement: null };
  host.IS_REACT_ACT_ENVIRONMENT = true;
  const container = { nodeType: 1, nodeName: 'DIV', tagName: 'DIV', namespaceURI: 'http://www.w3.org/1999/xhtml', ownerDocument: globals.document,
    ...listeners, childNodes: [], firstChild: null, removeChild() {}, appendChild() {}, insertBefore() {} };
  const track = { source: { id: 'source', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 100 } };
  const client = {};
  function Probe({ faction }: { faction: string }) {
    state.latest = module.useGoalHistory(client as never, track as never, faction, true);
    return null;
  }
  const root = createRoot(container as never);
  const show = (faction: string) => act(async () => root.render(createElement(Probe, { faction })));
  const answer = (run: () => void) => act(async () => run());
  const { requests } = state;
  try {
    await show('9');
    assert.equal(requests.length, 1);
    assert.deepEqual([requests[0].params.firstHour, requests[0].params.lastHour], [null, null]);
    await answer(() => requests[0].reject(OVER()));
    assert.deepEqual(requests.slice(1).map(asked => [asked.params.firstHour, asked.params.lastHour]), [[null, 50], [51, null]]);
    // Staggered: the later half is refused while the earlier one is still running.
    await answer(() => requests[2].reject(OVER()));
    assert.equal(requests.length, 5);
    assert.deepEqual(requests.slice(3).map(asked => [asked.params.firstHour, asked.params.lastHour]), [[51, 75], [76, null]]);
    assert.equal(requests[1].signal.aborted, false, 'a sibling keeps its request through a split');
    await answer(() => requests[4].resolve(reply('late')));
    await answer(() => requests[1].publish(reply('early, still preparing')));
    // A split after a sibling answered keeps that sibling's data and asks it nothing again.
    assert.deepEqual(ranges(state.latest!.parts), [[null, 50, 'early, still preparing', null], [51, 75, null, null], [76, null, 'late', null]]);
    await answer(() => requests[3].resolve(reply('middle')));
    await answer(() => requests[1].resolve(reply('early')));
    assert.equal(requests.length, 5, 'no range was asked twice');
    assert.ok(requests.every(asked => !asked.signal.aborted));
    assert.deepEqual(ranges(state.latest!.parts), [[null, 50, 'early', null], [51, 75, 'middle', null], [76, null, 'late', null]]);
    await show('9');
    assert.equal(requests.length, 5, 'rendering again asks nothing');
    // A switch cancels every unfinished request and starts the new faction whole, without the last faction's data.
    await show('10');
    assert.equal(requests.length, 6);
    assert.equal(requests[5].params.faction, '10');
    assert.deepEqual(ranges(state.latest!.parts), [[null, null, null, null]]);
    await show('11');
    assert.equal(requests[5].signal.aborted, true);
    assert.equal(requests.length, 7);
    await answer(() => requests[5].resolve(reply('stale')));
    assert.deepEqual(ranges(state.latest!.parts), [[null, null, null, null]], 'a cancelled answer is never shown');
    // A failure that is not the budget is held, and its retry asks the whole history again.
    await answer(() => requests[6].reject(new Error('Background preparation is off.')));
    assert.equal(state.latest!.error?.message, 'Background preparation is off.');
    assert.equal(requests.length, 7, 'an ordinary failure is not split');
    await answer(() => state.latest!.retry());
    assert.equal(requests.length, 8);
    assert.deepEqual([requests[7].params.firstHour, requests[7].params.lastHour], [null, null]);
    await act(async () => root.unmount());
    assert.equal(requests[7].signal.aborted, true, 'unmounting cancels what is still running');
  } finally { delete host.__history; await close(); }
});
