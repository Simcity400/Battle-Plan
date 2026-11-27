import { queryFailure, RefusedError } from './queryError.ts';
import type { QueryError } from '../generated/dashboard';
import { invoke } from '@tauri-apps/api/core';
import { invokeWrite, tracked } from '../shell/writes.ts';
import { listen } from '@tauri-apps/api/event';
import { newId } from '../shell/ids.ts';
import { desktopAvailable } from '../shell/api';
import type { ContentFolders } from '../shell/api';
import type { SourceReply } from '../generated/dashboard';
import { MAX_PENDING_REQUESTS } from '../generated/dashboard.ts';
import type { DashboardClient, DashboardMapState, GameWindowRender, GameWindowRequest, MapObserveEvent, MapOptionsEvent, MapToggleEvent, DashboardSource, MapSettingsReply, ProductReply, RecorderSettings, ReplayLibrary } from './client';
import { replyLengths } from './products.ts';
import { keepDigest } from './replyDigest.ts';
import { parseReplyBytes } from './replyParse.ts';
import { contentRevision } from './contentRevision.ts';
import { forgetReplySizes } from './replySizes.ts';
import type { Compression } from '../recorder/compression';
import { perfLog } from '../shell/perf.ts';
import { busy } from '../shell/busy.ts';
import { OpenedSources } from './openedSources.ts';
import type { QueryTiming } from '../shell/perf';

function cancelled(): DOMException {
  return new DOMException('Request cancelled', 'AbortError');
}

async function rawInvoke<T>(command: string, args: Record<string, unknown>, timing?: QueryTiming, dropped?: AbortSignal): Promise<T> {
  const result = await invoke<unknown>(command, args).catch((error: QueryError | string) => { throw queryFailure(error); });
  if (timing) timing.received = performance.now();
  if (dropped?.aborted) throw cancelled();
  if (!(result instanceof ArrayBuffer || result instanceof Uint8Array)) return result as T;
  const bytes = result.byteLength;
  const reply: unknown = await parseReplyBytes(result);
  if (timing) { timing.parsed = performance.now(); timing.bytes = bytes; }
  if (reply && typeof reply === 'object') {
    replyLengths.set(reply, bytes);
    const { data, digest } = reply as { data?: unknown; digest?: unknown };
    if (data && typeof data === 'object') {
      replyLengths.set(data, bytes);
      if (typeof digest === 'string') keepDigest(digest, data);
      const through = (data as { stableThrough?: unknown }).stableThrough;
      if (timing && typeof through === 'number') timing.through = through;
    }
  }
  return reply as T;
}

let outstanding = 0;
const waiting: (() => void)[] = [];
async function admitted(signal: AbortSignal): Promise<() => void> {
  if (outstanding >= MAX_PENDING_REQUESTS) {
    await new Promise<void>((resolve, reject) => {
      const turn = () => { signal.removeEventListener('abort', leave); resolve(); };
      const leave = () => { const at = waiting.indexOf(turn); if (at >= 0) waiting.splice(at, 1); reject(cancelled()); };
      waiting.push(turn);
      signal.addEventListener('abort', leave, { once: true });
    });
  } else outstanding++;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    const next = waiting.shift();
    if (next) next(); else outstanding--;
  };
}

async function request<T>(
  command: string,
  args: (requestId: string) => Record<string, unknown>,
  signal: AbortSignal,
  releaseLate?: (result: T) => Promise<void>,
  asksFrom?: number,
): Promise<T> {
  if (!desktopAvailable) throw new Error('Open BattlePlan in the desktop app to read recordings and show the map.');
  if (signal.aborted) throw cancelled();
  const done = busy('requests');
  try {
    const release = await admitted(signal);
    try {
      if (signal.aborted) throw cancelled();
      return await sent(command, args, signal, releaseLate, asksFrom);
    } finally { release(); }
  } finally { done(); }
}

async function sent<T>(
  command: string,
  args: (requestId: string) => Record<string, unknown>,
  signal: AbortSignal,
  releaseLate?: (result: T) => Promise<void>,
  asksFrom?: number,
): Promise<T> {
  const requestId = newId();
  const body = args(requestId);
  const abort = () => { void invoke('dashboard_cancel', { requestId }).catch(() => {}); };
  signal.addEventListener('abort', abort, { once: true });
  const perf = perfLog, started = perf ? performance.now() : 0, timing: QueryTiming | undefined = perf ? {} : undefined, head = perf?.head() ?? null;
  let outcome: 'ok' | 'cancelled' | 'error' = 'error', failure: string | undefined;
  try {
    const result = await rawInvoke<T>(command, { ...body, requestId }, timing, releaseLate ? undefined : signal);
    if (signal.aborted) {
      await releaseLate?.(result).catch(() => {});
      throw cancelled();
    }
    outcome = 'ok';
    return result;
  } catch (error) {
    if (signal.aborted) { outcome = 'cancelled'; throw cancelled(); }
    if (perf) failure = error instanceof RefusedError ? `refused: ${error.text}` : `failed: ${error instanceof Error ? error.message : String(error)}`;
    throw error;
  } finally {
    signal.removeEventListener('abort', abort);
    if (perf && timing) perf.query(command, body, requestId, started, timing, outcome, failure, { from: asksFrom ?? null, head });
  }
}

const opened = new OpenedSources();
const IS_EXPORT_WINDOW = typeof window !== 'undefined' && new URLSearchParams(window.location.search).get('export') === '1';
const FOLLOW_PAGE_ID = `${IS_EXPORT_WINDOW ? 'export:' : ''}${crypto.randomUUID()}`;
let nextFollowSession = 0;

async function releaseSource(sourceId: string): Promise<void> {
  opened.delete(sourceId);
  forgetReplySizes(nativeDashboardClient, sourceId);
  if (!desktopAvailable) return;
  await invoke('dashboard_release_source', { sourceId });
}

export const nativeDashboardClient: DashboardClient = {
  available: desktopAvailable,
  ...(desktopAvailable ? {
    openFollow(ports: readonly MessagePort[]) {
      const reader = new Worker(new URL('./pull.worker.ts', import.meta.url), { type: 'module' });
      const session = `${FOLLOW_PAGE_ID}:${++nextFollowSession}`;
      let terminated = false;
      const close = () => {
        if (terminated) return;
        terminated = true;
        reader.onmessage = null;
        reader.onerror = null;
        reader.terminate();
      };
      const retireLane = (lane: number) => {
        if (!terminated) reader.postMessage({ type: 'retire-lane', lane });
      };
      const ready = new Promise<string>((resolve, reject) => {
        const fail = (reason: string) => { close(); reject(new Error(reason)); };
        reader.onmessage = ({ data }) => {
          if (data === 'ready') { reader.onmessage = null; reader.onerror = null; resolve(session); }
          else if (data.reason) fail(data.reason);
        };
        reader.onerror = event => fail(event.message);
        reader.postMessage({ ports, session }, [...ports]);
      });
      const done = busy('follow');
      return { ready: ready.finally(done), close: () => { done(); close(); }, retireLane };
    },
    replayLibrary: () => invoke<ReplayLibrary>('replay_library'),
    replayLibraryStamp: () => invoke<string>('replay_library_stamp'),
    setReplayFolder: (folder: string) => invokeWrite<ReplayLibrary>('set_replay_folder', { folder }),
    chooseReplayFolder: (initial?: string) => invoke<string | null>('choose_replay_folder', { initial: initial ?? null }),
    chooseRecording: () => invoke<string | null>('choose_recording'),
    renameRecording: (path: string, name: string) => invokeWrite<{ path: string; warning: string | null }>('rename_recording', { path, name }),
    locateRecordings: (recordings: { path: string; campaignKey: string }[]) => invoke<(string | null)[]>('locate_recordings', { recordings }),
    recorderSettings: () => invoke<RecorderSettings>('recorder_settings'),
    setRecordingCompression: (compression: Compression) => invokeWrite<RecorderSettings>('set_recording_compression', { compression }),
    setAutoCompress: (enabled: boolean) => invokeWrite<RecorderSettings>('set_auto_compress', { enabled }),
    contentFolders: () => invoke<ContentFolders>('content_folders'),
    chooseContentFolder: (kind: 'game' | 'user', initial: string) => invoke<string | null>('choose_content_folder', { kind, initial }),
    setContentFolders: (game: string, user: string) => invokeWrite<ContentFolders>('set_content_folders', { game, user }),
  } : {}),

  async mapSettings(sourceId, revision, settings, signal, target = { scope: 'campaign' }) {
    const asked = () => request<MapSettingsReply>('dashboard_map_settings', () => ({ sourceId, revision, settings, flip: null, target }), signal);
    const reply = await (settings ? tracked(asked) : asked());
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Map settings do not match this campaign');
    contentRevision(nativeDashboardClient).observePresentation(reply.presentationGeneration);
    return reply;
  },
  async flipMapSetting(sourceId, revision, setting, signal, target) {
    const reply = await tracked(() => request<MapSettingsReply>('dashboard_map_settings', () => ({ sourceId, revision, settings: null, flip: setting, target }), signal));
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Map settings do not match this campaign');
    contentRevision(nativeDashboardClient).observePresentation(reply.presentationGeneration);
    return reply;
  },

  chooseHistory: async (path, segmentId, choice, signal) => { await tracked(() => request<unknown>('dashboard_history_choice', () => ({ path, segmentId, choice }), signal)); },
  async openSource(path, signal): Promise<DashboardSource> {
    if (signal.aborted) throw cancelled();
    const source = await request<SourceReply>('dashboard_open_source', () => ({ path }), signal,
      late => invoke('dashboard_release_source', { sourceId: late.id }));
    contentRevision(nativeDashboardClient).observePresentation(source.presentationGeneration);
    return opened.add({ id: source.id, path: source.path, revision: source.revision, metadata: { ...source.metadata, revision: source.revision } });
  },

  product<T>({ asksFrom, ...query }: import('./client').ProductRequest, signal: AbortSignal) {
    const refused = opened.refusal(query);
    if (refused) return Promise.reject(new RefusedError(refused));
    return request<ProductReply<T>>('dashboard_product', () => ({ query }), signal, undefined, asksFrom);
  },

  async renderGameWindow(windowRequest: GameWindowRequest, signal: AbortSignal): Promise<GameWindowRender> {
    if (!desktopAvailable) throw new Error('Open BattlePlan in the desktop app to display game windows.');
    if (signal.aborted) throw cancelled();
    const requestId = `gw-${newId()}`;
    const done = busy('requests');
    try {
      const release = await admitted(signal);
      let cancelling: Promise<unknown> | undefined;
      const abort = () => { cancelling = invoke('cancel_game_window', { requestId }).catch(() => {}); };
      signal.addEventListener('abort', abort, { once: true });
      try {
        if (signal.aborted) throw cancelled();
        return await rawInvoke<GameWindowRender>(
          'render_game_window',
          { request: windowRequest, requestId },
          undefined,
          signal,
        );
      } finally {
        signal.removeEventListener('abort', abort);
        if (cancelling) {
          await cancelling;
          await invoke('forget_game_window_cancel', { requestId }).catch(() => {});
        }
        release();
      }
    } finally { done(); }
  },

  releaseSource,

  async setMapViews(clockId, views) {
    if (desktopAvailable) await invoke('set_dashboard_maps', { clockId, views });
  },

  async onMapStates(callback) {
    if (!desktopAvailable) return () => {};
    return listen<DashboardMapState[]>('dashboard-map-state', event => callback(event.payload));
  },

  async onMapOptions(callback) {
    if (!desktopAvailable) return () => {};
    return listen<MapOptionsEvent>('dashboard-map-options', event => callback(event.payload));
  },

  async onMapToggle(callback) {
    if (!desktopAvailable) return () => {};
    return listen<MapToggleEvent>('dashboard-map-toggle', event => callback(event.payload));
  },

  async onMapObserve(callback) {
    if (!desktopAvailable) return () => {};
    return listen<MapObserveEvent>('dashboard-map-observe', event => callback(event.payload));
  },
};
