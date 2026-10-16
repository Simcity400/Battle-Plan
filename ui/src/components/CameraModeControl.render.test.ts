import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, type ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { CameraMode } from '../generated/dashboard';

test('the camera control offers map, fly and walk and marks the current mode', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const module = await vite.ssrLoadModule('/ui/src/components/CameraModeControl.tsx') as {
      default: ComponentType<{ value: CameraMode; onChange: (mode: CameraMode) => void }>;
      CAMERA_MODES: readonly { value: CameraMode; label: string }[];
    };
    assert.deepEqual(module.CAMERA_MODES.map(mode => mode.value), ['map', 'fly', 'walk']);
    const markup = renderToStaticMarkup(createElement(module.default, { value: 'fly', onChange() {} }));
    assert.match(markup, /role="group" aria-label="Camera"/);
    const pressed = [...markup.matchAll(/aria-pressed="(true|false)"[^>]*>(?:<svg[\s\S]*?<\/svg>)?(Map|Fly|Walk)</g)]
      .map(([, state, label]) => `${label}:${state}`);
    assert.deepEqual(pressed, ['Map:false', 'Fly:true', 'Walk:false']);
  } finally {
    await vite.close();
  }
});
