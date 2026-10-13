import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { Campaign } from './campaigns';

const tick = () => new Promise<void>(resolve => setImmediate(resolve));

test('manual compaction reloads open campaigns for either entry point and waits for pending opens', async () => {
  const state: unknown[] = [];
  const effects: { deps: unknown[]; cleanup?: () => void }[] = [];
  let slot = 0, effect = 0;
  let listener: ((path: string) => void) | undefined;
  let detached = 0;
  const fixture = {
    useState(initial: unknown) {
      const at = slot++;
      if (!(at in state)) state[at] = initial;
      return [state[at], (next: any) => { state[at] = typeof next === 'function' ? next(state[at]) : next; }];
    },
    useEffect(run: () => (() => void) | undefined, deps: unknown[]) {
      const at = effect++;
      if (effects[at] && deps.every((item, index) => effects[at].deps[index] === item)) return;
      effects[at]?.cleanup?.(); effects[at] = { deps, cleanup: run() };
    },
  };
  const globals = globalThis as typeof globalThis & { __compactedCampaignFixture?: typeof fixture };
  globals.__compactedCampaignFixture = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'compacted-campaign-fixture', enforce: 'pre', transform(source, id) {
      if (id.replaceAll('\\', '/').endsWith('/dashboard/compactedCampaigns.ts')) return source.replace(
        "import { useEffect, useState } from 'react';", 'const { useEffect, useState } = globalThis.__compactedCampaignFixture;');
    } }] });
  try {
    const { useCompactedCampaignReload } = await vite.ssrLoadModule('/ui/src/dashboard/compactedCampaigns.ts');
    const client = { onRecordingCompacted: async (callback: (path: string) => void) => { listener = callback; return () => { detached++; }; } };
    const reloaded: string[] = [];
    const reload = async (campaign: Campaign) => { reloaded.push(campaign.id); };
    let campaigns: Campaign[] = [{ id: 'shown', path: 'C:/replays/Shown.battleplan', label: 'Shown' },
      { id: 'other', path: 'C:/replays/Other.battleplan', label: 'Other' },
      { id: 'opening', path: 'C:/replays/Opening.battleplan', label: 'Opening', loading: true }];
    const render = () => { slot = 0; effect = 0; useCompactedCampaignReload(client, campaigns, reload); };
    render(); await tick();
    // Compact and Compact all use the same changed-recording notification.
    listener!('c:/REPLAYS/shown.battleplan'); render(); render();
    assert.deepEqual(reloaded, ['shown']);
    listener!('C:/replays/Other.battleplan'); listener!('C:/replays/Archive.battleplan');
    listener!('C:/replays/Opening.battleplan'); render(); render();
    assert.deepEqual(reloaded, ['shown', 'other'], 'archives stay closed; pending opens wait');
    campaigns = campaigns.map(campaign => ({ ...campaign, loading: false })); render(); render();
    assert.deepEqual(reloaded, ['shown', 'other', 'opening']);
    listener!('C:/replays/Other.battleplan');
    campaigns = campaigns.filter(campaign => campaign.id !== 'other'); render(); render();
    assert.deepEqual(reloaded, ['shown', 'other', 'opening'], 'an unloaded campaign is never reopened');
    render(); assert.equal(reloaded.length, 3, 'unchanged recordings produce no notification or reload');
    for (const entry of effects) entry.cleanup?.();
    assert.equal(detached, 1);
    listener!('C:/replays/Shown.battleplan'); render();
    assert.equal(reloaded.length, 3, 'late notifications after unmount are ignored');
  } finally { delete globals.__compactedCampaignFixture; await vite.close(); }
});

test('opening and releasing native sources sends only dashboard commands, including failed or cancelled opens', async () => {
  const calls: [string, any][] = [];
  let failOpen = false;
  let pendingOpen: ((reply: unknown) => void) | undefined;
  let delayOpen = false;
  const fixture = {
    invoke: async (command: string, args: any) => {
      calls.push([command, args]);
      if (command === 'dashboard_open_source') {
        if (failOpen) throw new Error('synthetic open failure');
        if (delayOpen) return new Promise(resolve => { pendingOpen = resolve; });
        return { id: 'lease', path: args.path, revision: 'new', metadata: {} };
      }
    },
  };
  const globals = globalThis as typeof globalThis & { __compactionNativeFixture?: typeof fixture };
  globals.__compactionNativeFixture = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'compaction-native-fixture', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/shell/api.ts')) return 'export const desktopAvailable = true;';
      if (path.endsWith('/dashboard/nativeClient.ts')) return source.replace(
        "import { invoke } from '@tauri-apps/api/core';", 'const invoke = (command, args) => globalThis.__compactionNativeFixture.invoke(command, args);');
    } }] });
  const path = 'C:/replays/loaded.battleplan';
  try {
    const { nativeDashboardClient: client } = await vite.ssrLoadModule('/ui/src/dashboard/nativeClient.ts');
    const source = await client.openSource(path, new AbortController().signal);
    assert.deepEqual(calls.map(([command]) => command), ['dashboard_open_source']);
    await client.releaseSource(source.id);
    assert.deepEqual(calls.at(-1), ['dashboard_release_source', { sourceId: 'lease' }]);
    failOpen = true;
    await assert.rejects(client.openSource(path, new AbortController().signal), /synthetic open failure/);
    failOpen = false; delayOpen = true;
    const controller = new AbortController();
    const opening = client.openSource(path, controller.signal);
    await tick(); controller.abort();
    pendingOpen!({ id: 'late', path, revision: 'new', metadata: {} });
    await assert.rejects(opening, { name: 'AbortError' });
    assert.deepEqual(calls.at(-1), ['dashboard_release_source', { sourceId: 'late' }]);
    // The recorder compacts automatically; the app keeps no open-recording retention for it.
    assert.ok(calls.every(([command]) => command.startsWith('dashboard_')), JSON.stringify(calls));
  } finally { delete globals.__compactionNativeFixture; await vite.close(); }
});


test('campaign reconnect releases the old native source before opening its new revision', async () => {
  const state: any[] = [], refs: any[] = [];
  let slot = 0, ref = 0;
  const fixture = {
    useState(initial: any) { const at = slot++; if (!(at in state)) state[at] = initial; return [state[at], (next: any) => { state[at] = typeof next === 'function' ? next(state[at]) : next; }]; },
    useRef(initial: any) { const at = ref++; return refs[at] ??= { current: initial }; },
    useCallback(callback: unknown) { return callback; },
    useEffect(run: () => unknown) { run(); },
  };
  const globals = globalThis as typeof globalThis & { __compactionSourcesFixture?: typeof fixture };
  globals.__compactionSourcesFixture = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'compaction-sources-fixture', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/shell/api.ts')) return 'export const errorMessage = error => error.message ?? String(error);';
      if (path.endsWith('/dashboard/products.ts')) return 'export const forgetProducts = () => {};';
      if (path.endsWith('/dashboard/sources.ts')) return source.replace(
        "import { useCallback, useEffect, useRef, useState } from 'react';", 'const { useCallback, useEffect, useRef, useState } = globalThis.__compactionSourcesFixture;');
    } }] });
  try {
    const { useSourceRoles } = await vite.ssrLoadModule('/ui/src/dashboard/sources.ts');
    const calls: string[] = [];
    let allowRelease: (() => void) | undefined;
    let revision = 'old';
    const client = {
      openSource: async (path: string) => { calls.push(`open:${revision}`); return { id: revision, path, revision, metadata: {} }; },
      releaseSource: async (id: string) => { calls.push(`release:${id}`); await new Promise<void>(resolve => { allowRelease = resolve; }); },
    };
    const render = () => { slot = 0; ref = 0; return useSourceRoles(client); };
    let roles = render();
    await roles.open('same-slot', 'C:/replays/loaded.battleplan'); roles = render();
    assert.equal(roles.bindings['same-slot'].source.revision, 'old');
    revision = 'new';
    const reopening = roles.open('same-slot', 'C:/replays/loaded.battleplan');
    await tick();
    assert.deepEqual(calls, ['open:old', 'release:old'], 'the new open must wait until the old lease is released');
    allowRelease!(); await reopening; roles = render();
    assert.deepEqual(calls, ['open:old', 'release:old', 'open:new']);
    assert.equal(roles.bindings['same-slot'].source.revision, 'new');
  } finally { delete globals.__compactionSourcesFixture; await vite.close(); }
});
