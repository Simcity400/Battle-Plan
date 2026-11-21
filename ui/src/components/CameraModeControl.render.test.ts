import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, type ComponentType, type ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { CameraMode } from '../generated/dashboard';

type Props = { value: CameraMode; onChange: (mode: CameraMode) => void; overlays: boolean; onOverlaysChange: (shown: boolean) => void };

test('the camera control offers map, fly and walk, and the overlay switch flying and walking', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const module = await vite.ssrLoadModule('/ui/src/components/CameraModeControl.tsx') as {
      default: ComponentType<Props>;
      CAMERA_MODES: readonly { value: CameraMode; label: string }[];
    };
    assert.deepEqual(module.CAMERA_MODES.map(mode => mode.value), ['map', 'fly', 'walk']);
    const render = (props: Partial<Props>) => renderToStaticMarkup(createElement(module.default,
      { value: 'fly', onChange() {}, overlays: false, onOverlaysChange() {}, ...props }));
    const markup = render({});
    assert.match(markup, /role="group" aria-label="Camera"/);
    const pressed = [...markup.matchAll(/aria-pressed="(true|false)"[^>]*>(?:<svg[\s\S]*?<\/svg>)?(Map|Fly|Walk|Overlays)</g)]
      .map(([, state, label]) => `${label}:${state}`);
    assert.deepEqual(pressed, ['Map:false', 'Fly:true', 'Walk:false', 'Overlays:false']);
    assert.doesNotMatch(render({ value: 'map' }), /Overlays/);
    assert.match(render({ value: 'walk', overlays: true }), /aria-pressed="true"[^>]*>(?:<svg[\s\S]*?<\/svg>)?Overlays</);
    const asked: boolean[] = [];
    for (const overlays of [false, true]) {
      const control = module.default as (props: Props) => ReactElement;
      const tree = control({ value: 'walk', onChange() {}, overlays, onOverlaysChange: shown => asked.push(shown) });
      const [, group] = (tree.props as { children: ReactElement[] }).children;
      const button = (group.props as { children: ReactElement<{ onClick: () => void }> }).children;
      button.props.onClick();
    }
    assert.deepEqual(asked, [true, false]);
  } finally {
    await vite.close();
  }
});
