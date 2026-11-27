import { act } from 'react';
import { createRoot } from 'react-dom/client';
import type { SourceReply } from '../generated/dashboard';

const opened: string[] = [], mapOpens: string[] = [], mapSeeks: string[] = [];
const released: string[] = [], mapVisibility: boolean[] = [];
const listeners = new Map<string, number>();
const callbacks: Array<(event: { payload: unknown }) => void> = [];
let sendSnapshot!: (snapshot: unknown) => void;
const snapshot = new Promise<unknown>(resolve => { sendSnapshot = resolve; });
let sendChosen!: (source: SourceReply) => void;
let refuseChosen!: (error: unknown) => void;
const chosen = new Promise<SourceReply>((resolve, reject) => { sendChosen = resolve; refuseChosen = reject; });
const waiting = new Promise<never>(() => {});
const status = (ready: boolean, error: string | null = null, path: string | null = null, hour = path ? 0 : null) => ({
  running: true, ready, visible: false, error, cameraMode: 'map', cameraOverlays: false,
  viewer: { path, revision: path ? '1' : null, branch: path ? '1' : null, hour },
});

function source(path: string): SourceReply {
  if (path === 'launch.battleplan') path = 'C:/recordings/launch.battleplan';
  return { id: path, path, revision: '1', presentationGeneration: 1, metadata: {
    path, name: path, format: 154, captureMode: 'full', campaignKey: path, countries: [], playerCountryId: null,
    contentVerified: true, partial: null, warnings: [], buildWarnings: [],
    branches: [{ id: '1', sessionId: '1', firstHour: 0, lastHour: 100, main: true, fork: null, joins: [], sampleBoundary: '', gaps: [] }],
  } };
}

Object.assign(window, {
  isTauri: true,
  IS_REACT_ACT_ENVIRONMENT: true,
  __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
  __TAURI_INTERNALS__: {
    transformCallback(callback: (event: { payload: unknown }) => void) { return callbacks.push(callback) - 1; },
    async invoke(command: string, args: { event: string; handler: number; path: string; sourceId: string; visible: boolean }) {
      switch (command) {
        case 'plugin:event|listen': listeners.set(args.event, args.handler); return args.handler;
        case 'desktop_state': return snapshot;
        case 'content_folders': return { game: '', user: '', contentReady: true };
        case 'replay_library': return { folder: null, files: [] };
        case 'choose_recording': return 'chosen.battleplan';
        case 'dashboard_open_source':
          opened.push(args.path);
          return args.path === 'chosen.battleplan' ? chosen : source(args.path);
        case 'open_replay': mapOpens.push(args.path); return;
        case 'seek_map': mapSeeks.push(args.path); return;
        case 'set_map_view': mapVisibility.push(args.visible); return;
        case 'dashboard_release_source': released.push(args.sourceId); return;
        case 'dashboard_product': return waiting;
        default: return null;
      }
    },
  },
});

const { default: App } = await import('./App');
await import('../dashboard/Dashboard');
const root = createRoot(document.body.appendChild(document.createElement('div')));
function click(selector: string) {
  const element = document.querySelector<HTMLButtonElement>(selector);
  if (!element) throw new Error(`Missing button ${selector}`);
  element.click();
}

Object.assign(window, { startupRecording: {
  async mount() { await act(async () => root.render(<App />)); },
  async launch() {
    await act(async () => sendSnapshot({ ...status(false), launchRecording: 'launch.battleplan' }));
    return opened;
  },
  async viewer(ready: boolean, error: string | null = null, path: string | null = null, hour?: number | null) {
    await act(async () => callbacks[listeners.get('viewer-state')!]({ payload: status(ready, error, path, hour) }));
    return mapVisibility.at(-1);
  },
  async choose() {
    const button = await new Promise<HTMLButtonElement>(resolve => {
      const check = () => {
        const candidates = document.querySelectorAll<HTMLButtonElement>('.sidebar-campaigns-empty button, button[aria-label="Open a recording"]');
        const available = Array.from(candidates).find(button => !button.matches(':disabled') && !button.closest('[hidden], [inert]'));
        if (!available) return;
        observer.disconnect();
        resolve(available);
      };
      const observer = new MutationObserver(check);
      observer.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['disabled', 'hidden', 'inert'] });
      check();
    });
    await act(async () => button.click());
    return opened;
  },
  async deliverChoice() { await act(async () => sendChosen(source('chosen.battleplan'))); },
  async refuseChoice() {
    await act(async () => refuseChosen({ kind: 'refused', text: 'This recording could not be opened' }));
    return document.querySelector('.library-row[data-detail]')?.getAttribute('data-detail');
  },
  async play() {
    await act(async () => click('.header-play[aria-label="Play"]'));
    return { playing: !!document.querySelector('.header-play[aria-label="Pause"]'), opened, mapOpens, mapSeeks: [...new Set(mapSeeks)] };
  },
  async retryMap() {
    await act(async () => click('#panel-map [role="alert"] button.primary'));
    return { opened, mapOpens, released, playing: !!document.querySelector('.header-play[aria-label="Pause"]') };
  },
} });
