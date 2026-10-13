import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { ReactElement } from 'react';
import type { CompactionProgress, DashboardClient, RecorderSettings } from '../dashboard/client';

type Element = ReactElement<Record<string, any>>;
function elements(value: any): Element[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  return value && typeof value === 'object' && 'props' in value ? [value, ...elements(value.props.children)] : [];
}
const tick = () => new Promise<void>(resolve => setImmediate(resolve));

test('compression settings persist through the native client and manual folder compaction reports progress, savings and every unchanged reason', async () => {
  const slots: any[] = [];
  let cursor = 0, effect = 0;
  let timer: (() => void) | undefined;
  const effects: { deps: unknown[]; cleanup?: () => void }[] = [];
  const fixture = {
    useState(initial: any) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = initial;
      return [slots[index], (next: any) => { slots[index] = typeof next === 'function' ? next(slots[index]) : next; }];
    },
    useEffect(run: () => (() => void), deps: unknown[]) {
      const index = effect++;
      if (effects[index] && deps.every((value, at) => effects[index].deps[at] === value)) return;
      effects[index]?.cleanup?.();
      effects[index] = { deps, cleanup: run() };
    },
    timeout(next: () => void) { timer = next; return 1; },
    clear() { timer = undefined; },
  };
  let stored: RecorderSettings = { compression: 'fast' };
  let refuseSave = false;
  const calls: [string, unknown][] = [];
  let progress: CompactionProgress = { running: false, gameRunning: true, total: 0, completed: 0, compacted: 0,
    skipped: 0, bytesSaved: 0, current: null, blocksTotal: 0, blocksCompleted: 0, unchanged: [] };
  const client = {
    recorderSettings: async () => stored,
    setRecordingCompression: async (compression: RecorderSettings['compression']) => {
      calls.push(['preference', compression]);
      if (refuseSave) throw new Error('Could not save recorder settings');
      return stored = { compression };
    },
    compactionProgress: async () => progress,
    compactReplayFolder: async () => {
      calls.push(['compact-all', stored.compression]);
      return progress = { ...progress, running: true, total: 3, current: 'C:/replays/one.battleplan', blocksTotal: 4 };
    },
  } as DashboardClient;
  const globalFixture = globalThis as typeof globalThis & { __compressionFixture?: typeof fixture; __compressionInvoke?: (command: string, args?: unknown) => Promise<unknown> };
  globalFixture.__compressionFixture = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'compression-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/shell/api.ts')) return `export const desktopAvailable = true;
          export const errorMessage = error => error.message ?? String(error);
          export const recordingName = path => path.split(/[\\\\/]/).at(-1) ?? path;`;
        if (path.endsWith('/shell/RecordingCompressionSetting.tsx')) return source.replace(
          "import { useEffect, useState } from 'react';",
          'const { useEffect, useState, timeout: setTimeout, clear: clearTimeout } = globalThis.__compressionFixture;');
        if (path.endsWith('/dashboard/nativeClient.ts')) return source.replace(
          "import { invoke } from '@tauri-apps/api/core';", 'const invoke = (command, args) => globalThis.__compressionInvoke(command, args);');
      },
    }] });
  try {
    const { default: Setting, compactionSummary } = await vite.ssrLoadModule('/ui/src/shell/RecordingCompressionSetting.tsx');
    let nodes: Element[] = [];
    const render = (folder: string | null = 'C:/replays') => { cursor = 0; effect = 0; nodes = elements(Setting({ client, folder })); };
    const find = (predicate: (element: Element) => boolean) => { const found = nodes.find(predicate); assert.ok(found); return found; };
    const select = () => find(element => element.props.label === 'Recording compression');
    const compact = () => find(element => element.type === 'button' && element.props.children === 'Compact all');
    render(); await tick(); render();
    assert.equal(select().props.value, 'fast');
    assert.deepEqual(select().props.options.map((option: any) => option.label), ['Fast (larger files)', 'Small (slower)']);
    select().props.onChange('small'); await tick(); render();
    assert.equal(stored.compression, 'small');
    assert.equal(select().props.value, 'small');
    refuseSave = true;
    select().props.onChange('fast'); await tick(); render();
    assert.equal(select().props.value, 'small', 'a failed native save does not pretend the setting changed');
    assert.ok(nodes.some(element => element.props.role === 'alert' && element.props.children === 'Could not save recorder settings'));
    refuseSave = false;
    select().props.onChange('fast'); await tick(); render();
    render(null); assert.equal(compact().props.disabled, true, 'a replay folder must be configured');
    render(); compact().props.onClick(); await tick(); render();
    assert.deepEqual(calls.at(-1), ['compact-all', 'fast'], 'manual compaction remains available with Fast selected');
    assert.equal(compact().props.disabled, true, 'a running job cannot be duplicated');
    assert.ok(nodes.some(element => element.type === 'progress'));
    progress = { ...progress, running: false, completed: 5, total: 5, compacted: 1, skipped: 4, bytesSaved: 2 * 2 ** 20,
      current: null, unchanged: [
        { path: 'C:/replays/busy.battleplan', reason: 'Busy; try again later' },
        { path: 'C:/replays/live.battleplan', reason: 'Still being recorded' },
        { path: 'C:/replays/small.battleplan', reason: 'Already as small as it gets' },
        { path: 'C:/replays/changed.battleplan', reason: 'Changed while compacting; original kept' },
      ] };
    timer!(); await tick(); render();
    assert.equal(compact().props.disabled, false);
    assert.equal(compactionSummary(progress), 'Checked 5 of 5 recordings · Saved 2.0 MB · 4 left unchanged');

    for (const reason of ['Busy; try again later', 'Still being recorded', 'Already as small as it gets', 'Changed while compacting; original kept']) {
      assert.ok(nodes.some(element => element.type === 'li' && element.props.children.join('').includes(reason)));
    }
    assert.ok(nodes.some(element => element.props.children === 'The game is running. Compacting may slow it briefly.'));
    progress = { ...progress, gameRunning: false };
    timer!(); await tick(); render();
    assert.ok(!nodes.some(element => element.props.children === 'The game is running. Compacting may slow it briefly.'));
    assert.ok(!nodes.some(element => typeof element.props.children === 'string' && element.props.children.includes('Close open campaigns')));

    const nativeCalls: [string, unknown][] = [];
    globalFixture.__compressionInvoke = async (command, args) => { nativeCalls.push([command, args]); return command === 'recording_compaction_status' ? true : command.includes('compression') || command === 'recorder_settings' ? stored : progress; };
    const { nativeDashboardClient: native } = await vite.ssrLoadModule('/ui/src/dashboard/nativeClient.ts');
    await native.recorderSettings();
    await native.setRecordingCompression('small');
    await native.compactRecording('C:/replays/fast.battleplan');
    await native.compactReplayFolder();
    await native.compactionProgress();
    assert.equal(await native.recordingCompacted('C:/replays/fast.battleplan'), true);
    assert.deepEqual(nativeCalls, [
      ['recorder_settings', undefined], ['set_recording_compression', { compression: 'small' }],
      ['compact_recording', { path: 'C:/replays/fast.battleplan' }], ['compact_replay_folder', undefined],
      ['compaction_progress', undefined], ['recording_compaction_status', { path: 'C:/replays/fast.battleplan' }],
    ]);
  } finally {
    for (const entry of effects) entry.cleanup?.();
    delete globalFixture.__compressionFixture; delete globalFixture.__compressionInvoke;
    await vite.close();
  }
});
