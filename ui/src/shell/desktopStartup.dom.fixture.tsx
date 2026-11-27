import { createElement } from 'react';
import { flushSync } from 'react-dom';
import { createRoot } from 'react-dom/client';
import type { MapExportSnapshot } from '../export/videoExportApi';
import type { DashboardClient, DashboardSource, ReplayLibrary } from '../dashboard/client';
import type { WidgetRuntime } from '../dashboard/definitions';
import type { DashboardTemplate, WidgetSpec } from '../dashboard/model';

const root = createRoot(document.body.appendChild(document.createElement('div')));
const asked: string[] = [];
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

const snapshot = deferred<MapExportSnapshot>();
const recordings = deferred<ReplayLibrary>();
let reads = 0;
Object.assign(globalThis, { isTauri: true });
Object.assign(window, { __TAURI_INTERNALS__: { invoke(command: string) {
  asked.push(command);
  if (command === 'map_export_snapshot') return snapshot.promise;
  if (command === 'preview_map_video') return Promise.resolve(new Uint8Array());
  throw new Error(`Unexpected command ${command}`);
} } });

const { default: ExportVideoDialog } = await import('../export/ExportVideoDialog');
const { default: ReplayLibraryMenu } = await import('../dashboard/ReplayLibraryMenu');
const { useReplayLibrary } = await import('../dashboard/replayLibrary');
const { default: CampaignPicker } = await import('../dashboard/CampaignPicker');
const { default: ReplayFolderSetting } = await import('./ReplayFolderSetting');

const client: DashboardClient = {
  available: true,
  async openSource() { throw new Error('Unexpected recording open'); },
  async releaseSource() {},
  async product() { throw new Error('Unexpected product read'); },
  replayLibrary() { reads++; return recordings.promise; },
  async setMapViews() {},
  async onMapStates() { return () => {}; },
};

const opened = { id: 'opened', label: 'Example open campaign', path: 'open/Example open.battleplan' };
const source = { id: opened.id, path: opened.path, revision: '1', metadata: { campaignKey: 'synthetic', countries: [], branches: [] } } as unknown as DashboardSource;
const bindings = { [opened.id]: { source, loading: false, error: null } };
let selected = 0;

function Library({ shellReady, hasOpen = false }: { shellReady: boolean; hasOpen?: boolean }) {
  const library = useReplayLibrary(client, shellReady);
  return <ReplayLibraryMenu library={library} client={client} campaigns={hasOpen ? [opened] : []} bindings={hasOpen ? bindings : {}} defaultId={null} countryTags={{}}
    slots={{ empty: [] }} errors={{}} relocations={[]} onLoad={() => {}} onUnload={() => {}} onDefault={() => { selected++; }} onRenamed={() => {}} />;
}

function OtherLibraryReaders() {
  const library = useReplayLibrary(client, true);
  return <>
    <CampaignPicker widget={{ id: 'widget', campaign: 'default' } as WidgetSpec} template={{ tracks: [], roles: [] } as unknown as DashboardTemplate}
      runtime={{ campaigns: [], recordings: library.files, recordingsLoading: library.loading } as unknown as WidgetRuntime} />
    <ReplayFolderSetting library={library} client={client} />
  </>;
}

const paint = () => new Promise<void>(done => requestAnimationFrame(() => requestAnimationFrame(() => done())));
const exportState = () => ({
  asked: [...asked],
  status: document.querySelector('.export-video-controls [role="status"]')?.textContent ?? null,
  disabled: document.querySelector<HTMLButtonElement>('.export-video-footer .primary')?.disabled,
});
const libraryState = () => ({
  reads,
  text: document.querySelector('.sidebar-campaigns')?.textContent,
  listed: document.querySelector('[aria-label="Library"]')?.textContent ?? null,
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
      placement: { center: [0, 0], height: 1, pose: { kind: 'map' } }, viewport: [1920, 1080],
      uiScale: 1, mapMode: 'test', perspective: null, fog: false, dayNight: false,
      alliedPlans: false, counterColorsCountry: false, natoCounters: false, audioMuted: true,
      maxTextureDimension: 8192, stretches: [{ first: 24, last: 120, join: 'start' }], unheld: [],
    });
    await paint();
    return exportState();
  },
  openLibraryBeforePaint() {
    flushSync(() => root.render(<Library shellReady={false} />));
    const before = libraryState();
    flushSync(() => document.querySelector<HTMLInputElement>('input[aria-label="Find a campaign"]')!.focus());
    const focused = libraryState();
    flushSync(() => root.render(<Library shellReady />));
    return { before, focused, waiting: libraryState() };
  },
  async finishLibraryRead() {
    recordings.resolve({ folder: 'recordings', files: [{ path: 'recordings/Example.battleplan', name: 'Example', folder: '' }] });
    await paint();
    return libraryState();
  },
  async searchWithAnOpenCampaign() {
    flushSync(() => root.render(<Library shellReady hasOpen />));
    const field = document.querySelector<HTMLInputElement>('input[aria-label="Find a campaign"]')!;
    flushSync(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(field, 'Example');
      field.dispatchEvent(new Event('input', { bubbles: true }));
    });
    flushSync(() => field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
    const status = document.querySelector('[aria-label="Campaign search results"] [role="status"]')?.textContent;
    const before = { selected, shortcuts: field.getAttribute('aria-keyshortcuts'), status };
    recordings.resolve({ folder: 'recordings', files: [] });
    await paint();
    flushSync(() => field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
    return { before, after: selected };
  },
  async readOtherLibrarySurfaces() {
    flushSync(() => root.render(<OtherLibraryReaders />));
    document.querySelector<HTMLButtonElement>('button[aria-label^="Widget campaign:"]')!.click();
    const picker = document.querySelector('.campaign-picker')!;
    const folder = document.querySelector<HTMLInputElement>('input[aria-label="Recordings folder"]')!;
    const status = (element: Element) => element.querySelector('[role="status"]')?.textContent ?? null;
    const before = { open: picker.matches(':popover-open'), picker: status(picker), folder: status(folder.closest('.folder-setting-row')!), disabled: folder.disabled };
    recordings.resolve({ folder: 'recordings', files: [{ path: 'recordings/Example.battleplan', name: 'Example', folder: '' }] });
    await paint();
    return { before, after: { picker: picker.textContent, folder: folder.value, disabled: folder.disabled } };
  },
} });
