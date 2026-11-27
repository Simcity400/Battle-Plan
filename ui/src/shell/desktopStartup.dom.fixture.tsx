import { createElement } from 'react';
import { flushSync } from 'react-dom';
import { createRoot } from 'react-dom/client';
import type { MapExportSnapshot } from '../export/videoExportApi';

const root = createRoot(document.body.appendChild(document.createElement('div')));
const asked: string[] = [];
const snapshot = Promise.withResolvers<MapExportSnapshot>();
Object.assign(globalThis, { isTauri: true });
Object.assign(window, { __TAURI_INTERNALS__: { invoke(command: string) {
  asked.push(command);
  if (command === 'map_export_snapshot') return snapshot.promise;
  if (command === 'preview_map_video') return Promise.resolve(new Uint8Array());
  throw new Error(`Unexpected command ${command}`);
} } });

const { default: ExportVideoDialog } = await import('../export/ExportVideoDialog');
const paint = () => new Promise<void>(done => requestAnimationFrame(() => requestAnimationFrame(() => done())));
const exportState = () => ({
  asked: [...asked],
  status: document.querySelector('.export-video-controls [role="status"]')?.textContent ?? null,
  disabled: document.querySelector<HTMLButtonElement>('.export-video-footer .primary')?.disabled,
});

Object.assign(window, { desktopStartup: {
  async openExport() {
    flushSync(() => root.render(createElement(ExportVideoDialog, {
      target: { kind: 'map', label: 'Campaign map' },
      busyExport: false,
      onClose() {},
      async onStart() {},
    })));
    await paint();
    return exportState();
  },
  async finishExportSetup() {
    snapshot.resolve({
      path: 'synthetic.battleplan', revision: null, branch: 1,
      currentHour: 48, firstHour: 24, lastHour: 120,
      placement: {} as MapExportSnapshot['placement'], viewport: [1920, 1080],
      uiScale: 1, mapMode: 'test', perspective: null, fog: false, dayNight: false,
      alliedPlans: false, counterColorsCountry: false, natoCounters: false, audioMuted: true,
      maxTextureDimension: 8192, stretches: [{ first: 24, last: 120, join: 'start' }], unheld: [],
    });
    await paint();
    return exportState();
  },
} });
