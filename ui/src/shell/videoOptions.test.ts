import test from 'node:test';
import assert from 'node:assert/strict';
import { FRAME_LIMIT_HINT, frameLimitInput, sameDisplay } from './videoOptions.ts';

test('a frame limit outside 1–1000 or not a number is refused, never read as no limit', () => {
  assert.deepEqual(frameLimitInput(''), { value: null });
  assert.deepEqual(frameLimitInput(' 60 '), { value: 60 });
  assert.deepEqual(frameLimitInput('59.6'), { value: 60 });
  assert.deepEqual(frameLimitInput('1'), { value: 1 });
  assert.deepEqual(frameLimitInput('1000'), { value: 1000 });
  for (const typo of ['0', '1001', '-5', 'abc'])
    assert.equal(frameLimitInput(typo), null, typo);
  assert.equal(frameLimitInput('', true), null, 'an unparseable entry the browser reports as empty is refused');
  assert.equal(FRAME_LIMIT_HINT, 'Use 1–1000, or leave empty for no limit.');
});

test('the maps display alike whatever their diagnostics switch, so Revert stays hidden at the game settings', () => {
  const game = { useGameSettings: true, uiScale: 100, vsync: true, mapUiScale: null, mapFrameLimit: null };
  const saved = { ...game, mapDiagnostics: true };
  assert.equal(sameDisplay(saved, game), true);
  for (const changed of [{ ...saved, uiScale: 125 }, { ...saved, vsync: false }, { ...saved, mapUiScale: 90 }, { ...saved, mapFrameLimit: 60 }, { ...saved, useGameSettings: false }])
    assert.equal(sameDisplay(changed, game), false, JSON.stringify(changed));
  assert.equal(sameDisplay(null, game), false);
});

test('a display change the desktop couldn’t save stays chosen, and Try again sends that same change', async () => {
  const { createServer } = await import('vite');
  const slots: unknown[] = [];
  let cursor = 0;
  const effects: (() => void)[] = [];
  const hooks = {
    useState(initial: unknown) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === 'function' ? initial() : initial;
      return [slots[index], (next: unknown) => { slots[index] = typeof next === 'function' ? next(slots[index]) : next; }];
    },
    useRef: (current: unknown) => { const index = cursor++; return (slots[index] ??= { current }); },
    useEffect(effect: () => void) { const index = cursor++; if (!(index in slots)) { slots[index] = true; effects.push(effect); } },
    useId: () => 'id',
  };
  const settings = { useGameSettings: true, uiScale: 100, vsync: true, mapUiScale: null, mapFrameLimit: null };
  const sent: unknown[] = [];
  let refuse = true;
  const host = globalThis as typeof globalThis & { __videoFixture?: unknown };
  host.__videoFixture = { hooks, invoke: async (command: string, args?: { settings: typeof settings }) => {
    if (command === 'video_settings') return { settings, gameSettings: settings, gameSettingsError: null, gameMapUiScale: null, mapUiScaleChoices: [] };
    sent.push(args!.settings);
    if (refuse) throw 'The display settings file is locked';
    return { settings: args!.settings, gameSettings: settings, gameSettingsError: null, gameMapUiScale: null, mapUiScaleChoices: [] };
  } };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'video-fixture', enforce: 'pre', transform(source, id) {
      const name = id.replaceAll('\\', '/');
      if (name.endsWith('/shell/writes.ts')) return source.replace("import { invoke } from '@tauri-apps/api/core';", 'const invoke = (...args) => globalThis.__videoFixture.invoke(...args);');
      if (name.endsWith('/shell/VideoSettings.tsx')) return source.replace(/import \{[^\n]+\} from 'react';/, 'const { useEffect, useId, useRef, useState } = globalThis.__videoFixture.hooks;')
        .replace("import { invoke } from '@tauri-apps/api/core';", 'const invoke = (...args) => globalThis.__videoFixture.invoke(...args);')
        .replace("import { desktopAvailable, errorMessage } from './api';", 'const desktopAvailable = true, errorMessage = String;');
    } }] });
  try {
    const { useVideoSettings } = await vite.ssrLoadModule('/ui/src/shell/VideoSettings.tsx') as typeof import('./VideoSettings');
    const settle = () => new Promise(resolve => setImmediate(resolve));
    const video = () => { cursor = 0; return useVideoSettings(true); };
    video(); for (const effect of effects.splice(0)) effect(); await settle();
    video().update({ vsync: false }); await settle();
    assert.equal(video().draft?.vsync, false, 'the switch keeps the attempted value instead of snapping back');
    assert.equal(video().writeFailed, true);
    assert.equal(video().error, 'The display settings file is locked');
    refuse = false;
    video().retry(); await settle();
    assert.deepEqual(sent.map(value => (value as typeof settings).vsync), [false, false], 'Try again sends the same change again');
    assert.equal(video().error, null, 'the failure clears once the change is saved');
    assert.equal(video().draft?.vsync, false);
  } finally { delete host.__videoFixture; await vite.close(); }
});
