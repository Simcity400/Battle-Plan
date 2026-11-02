import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';

const tick = () => new Promise<void>(resolve => setImmediate(resolve));

test('opening and releasing native sources sends only dashboard commands, including failed or cancelled opens', async () => {
  const calls: [string, any][] = [];
  let failOpen = false;
  let pendingOpen: ((reply: unknown) => void) | undefined;
  let delayOpen = false;
  const fixture = {
    invoke: async (command: string, args: any) => {
      calls.push([command, args]);
      if (command === 'dashboard_open_source') {
        if (failOpen) throw { kind: 'failed', message: 'synthetic open failure' };
        if (delayOpen) return new Promise(resolve => { pendingOpen = resolve; });
        return { id: 'lease', path: args.path, revision: 'new', metadata: {} };
      }
    },
  };
  const globals = globalThis as typeof globalThis & { __nativeSourceFixture?: typeof fixture };
  globals.__nativeSourceFixture = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'native-source-fixture', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/shell/api.ts')) return 'export const desktopAvailable = true;';
      if (path.endsWith('/dashboard/nativeClient.ts')) return source.replace(
        "import { invoke } from '@tauri-apps/api/core';", 'const invoke = (command, args) => globalThis.__nativeSourceFixture.invoke(command, args);');
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
    assert.ok(calls.every(([command]) => command.startsWith('dashboard_')), JSON.stringify(calls));
  } finally { delete globals.__nativeSourceFixture; await vite.close(); }
});


test('campaign reconnect releases the old native source before opening its new revision', async () => {
  const state: any[] = [], refs: any[] = [];
  let slot = 0, ref = 0;
  const fixture = {
    useState(initial: any) { const at = slot++; if (!(at in state)) state[at] = initial; return [state[at], (next: any) => { state[at] = typeof next === 'function' ? next(state[at]) : next; }]; },
    useRef(initial: any) { const at = ref++; return refs[at] ??= { current: initial }; },
    useCallback(callback: unknown) { return callback; },
    useEffect(run: () => unknown) { run(); },
    rosters: [] as string[],
  };
  const globals = globalThis as typeof globalThis & { __sourceRolesFixture?: typeof fixture };
  globals.__sourceRolesFixture = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'source-roles-fixture', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/shell/api.ts')) return 'export const errorMessage = error => error.message ?? String(error);';
      if (path.endsWith('/dashboard/products.ts')) return `export const forgetProducts = () => {};
        export const loadProduct = async (_client, request) => { globalThis.__sourceRolesFixture.rosters.push(request.sourceId);
          return { countries: [{ id: 7, tag: 'LATER', name: 'Later country' }], warnings: [] }; };`;
      if (path.endsWith('/dashboard/sources.ts')) return source.replace(
        "import { useCallback, useEffect, useRef, useState } from 'react';", 'const { useCallback, useEffect, useRef, useState } = globalThis.__sourceRolesFixture;');
    } }] });
  try {
    const { useSourceRoles } = await vite.ssrLoadModule('/ui/src/dashboard/sources.ts');
    const calls: string[] = [];
    let allowRelease: (() => void) | undefined;
    let revision = 'old';
    const client = {
      openSource: async (path: string) => { calls.push(`open:${revision}`); return { id: revision, path, revision, metadata: { countries: [] } }; },
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
    // Each opened campaign's later countries join its source.
    await tick(); roles = render();
    assert.deepEqual(fixture.rosters, ['old', 'new']);
    assert.deepEqual(roles.bindings['same-slot'].source.metadata.countries.map((country: { tag: string }) => country.tag), ['LATER']);
  } finally { delete globals.__sourceRolesFixture; await vite.close(); }
});
