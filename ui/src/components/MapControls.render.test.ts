import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, type ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('the pop-out map settings show its camera mode and overlay choice while its viewer runs', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'desktop-fixture', enforce: 'pre', transform(source, id) {
      if (id.replaceAll('\\', '/').endsWith('/shell/api.ts')) {
        return source.replace('export const desktopAvailable = isTauri();', 'export const desktopAvailable = true;');
      }
    } }] });
  try {
    const module = await vite.ssrLoadModule('/ui/src/components/MapControls.tsx') as { default: ComponentType<Record<string, unknown>> };
    const render = (popoutCameraMode: string | null, poppedOut = true, popoutCameraOverlays = false) => renderToStaticMarkup(createElement(module.default, {
      source: null, client: {}, metadata: null, branchId: null, onBranch() {}, options: { country: null, fog: true }, onOptions() {},
      poppedOut, popoutGeneration: 2, popoutCameraMode, popoutCameraOverlays,
    }));
    const shown = render('walk');
    assert.match(shown, /Pop-out camera/);
    const pressed = [...shown.matchAll(/aria-pressed="(true|false)"[^>]*>(?:<svg[\s\S]*?<\/svg>)?(Map|Fly|Walk)</g)]
      .map(([, state, label]) => `${label}:${state}`);
    assert.deepEqual(pressed, ['Map:false', 'Fly:false', 'Walk:true']);
    // Walking, the pop-out's own overlay choice shows beside its mode.
    const overlays = (markup: string) => markup.match(/aria-pressed="(true|false)"[^>]*>(?:(?!<\/button>)[\s\S])*Overlays<\/button>/)?.[1];
    assert.equal(overlays(shown), 'false');
    assert.equal(overlays(render('walk', true, true)), 'true');
    // No running pop-out viewer, or no pop-out, has no camera to show.
    assert.doesNotMatch(render(null), /Pop-out camera/);
    assert.doesNotMatch(render('fly', false), /Pop-out camera/);
  } finally {
    await vite.close();
  }
});
