import { queryFailure } from './queryError.ts';
import type { QueryError } from '../generated/dashboard';
import { validatePreparationReply, validatePreparationPreferences } from '../shell/preparation/contract.ts';
import { preparedReplyData, validateProductReply } from './client.ts';
import type { PreparationRequest, PreparationReply, PreparationPreferences } from '../shell/preparation/contract';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { newId } from '../shell/ids.ts';
import { desktopAvailable } from '../shell/api';
import type { ContentFolders } from '../shell/api';
import type { SourceReply } from '../generated/dashboard';
import { MAX_PENDING_REQUESTS } from '../generated/dashboard.ts';
import type { DashboardClient, DashboardMapState, MapOptionsEvent, MapToggleEvent, DashboardSource, MapSettingsReply, PreparedQueryOptions, ProductReply, RecorderSettings, ReplayLibrary } from './client';
import { replyLengths } from './products.ts';
import { keepDigest } from './replyDigest.ts';
import { parseReplyBytes } from './replyParse.ts';
import { contentRevision } from './contentRevision.ts';
import { forgetReplySizes } from './replySizes.ts';
import { forgetReplyGaps } from './replyGaps.ts';
import type { Compression } from '../recorder/compression';
import { perfLog } from '../shell/perf.ts';
import type { QueryTiming } from '../shell/perf';

function cancelled(): DOMException {
  return new DOMException('Request cancelled', 'AbortError');
}

/** Data replies arrive as raw JSON bytes; the host never parses them. */
async function rawInvoke<T>(command: string, args: Record<string, unknown>, timing?: QueryTiming): Promise<T> {
  const result = await invoke<unknown>(command, args).catch((error: QueryError) => { throw queryFailure(error); });
  if (timing) timing.received = performance.now();
  if (!(result instanceof ArrayBuffer || result instanceof Uint8Array)) return result as T;
  const bytes = result.byteLength;
  const reply: unknown = await parseReplyBytes(result);
  if (timing) { timing.parsed = performance.now(); timing.bytes = bytes; }
  if (reply && typeof reply === 'object') {
    replyLengths.set(reply, bytes);
    // A prepared reply's data is kept without its envelope.
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

/** Requests the host is holding, and those waiting for one of them to end, in order: the host refuses a
 * request past its bound for want of room, so one is never sent until there is room. */
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
    // A slot passes straight to the next waiting request.
    const next = waiting.shift();
    if (next) next(); else outstanding--;
  };
}

async function request<T>(
  command: string,
  args: (requestId: string) => Record<string, unknown>,
  signal: AbortSignal,
  releaseLate?: (result: T) => Promise<void>,
): Promise<T> {
  if (!desktopAvailable) throw new Error('Open the desktop app to read recordings and display native maps.');
  if (signal.aborted) throw cancelled();
  const release = await admitted(signal);
  try {
    // Cancelled while waiting for room, after the wait's own abort listener was removed.
    if (signal.aborted) throw cancelled();
    return await sent(command, args, signal, releaseLate);
  } finally { release(); }
}

async function sent<T>(
  command: string,
  args: (requestId: string) => Record<string, unknown>,
  signal: AbortSignal,
  releaseLate?: (result: T) => Promise<void>,
): Promise<T> {
  const requestId = newId();
  const body = args(requestId);
  const abort = () => { void invoke('dashboard_cancel', { requestId }).catch(() => {}); };
  signal.addEventListener('abort', abort, { once: true });
  const perf = perfLog, started = perf ? performance.now() : 0, timing: QueryTiming | undefined = perf ? {} : undefined;
  let outcome: 'ok' | 'cancelled' | 'error' = 'error';
  try {
    const result = await rawInvoke<T>(command, { ...body, requestId }, timing);
    if (signal.aborted) {
      await releaseLate?.(result).catch(() => {});
      throw cancelled();
    }
    outcome = 'ok';
    return result;
  } catch (error) {
    if (signal.aborted) { outcome = 'cancelled'; throw cancelled(); }
    throw error;
  } finally {
    signal.removeEventListener('abort', abort);
    if (perf && timing) perf.query(command, body, requestId, started, timing, outcome);
  }
}

async function preparation(command: string, query: PreparationRequest, signal: AbortSignal): Promise<PreparationReply> {
  if (!desktopAvailable) throw new Error('Open the desktop app to prepare recordings.');
  if (signal.aborted) throw cancelled();
  const release = await admitted(signal);
  const abort = () => { void invoke('dashboard_cancel', { requestId: query.requestId }).catch(() => {}); };
  signal.addEventListener('abort', abort, { once: true });
  try {
    if (signal.aborted) throw cancelled();
    const reply = await rawInvoke<PreparationReply>(command, query as unknown as Record<string, unknown>);
    if (signal.aborted) throw cancelled();
    return validatePreparationReply(reply, query);
  } finally { signal.removeEventListener('abort', abort); release(); }
}

async function releaseSource(sourceId: string): Promise<void> {
  forgetReplySizes(nativeDashboardClient, sourceId);
  forgetReplyGaps(sourceId);
  if (!desktopAvailable) return;
  await invoke('dashboard_release_source', { sourceId });
}

export const nativeDashboardClient: DashboardClient = {
  available: desktopAvailable,
  // Outside the desktop app there is no host to list or pick files.
  ...(desktopAvailable ? {
    replayLibrary: () => invoke<ReplayLibrary>('replay_library'),
    replayLibraryStamp: () => invoke<string>('replay_library_stamp'),
    setReplayFolder: (folder: string) => invoke<ReplayLibrary>('set_replay_folder', { folder }),
    chooseReplayFolder: (initial?: string) => invoke<string | null>('choose_replay_folder', { initial: initial ?? null }),
    chooseRecording: () => invoke<string | null>('choose_recording'),
    renameRecording: (path: string, name: string) => invoke<string>('rename_recording', { path, name }),
    locateRecordings: (recordings: { path: string; campaignKey: string }[]) => invoke<(string | null)[]>('locate_recordings', { recordings }),
    recorderSettings: () => invoke<RecorderSettings>('recorder_settings'),
    setRecordingCompression: (compression: Compression) => invoke<RecorderSettings>('set_recording_compression', { compression }),
    // The host owns the content folders and sends them to its native viewer.
    contentFolders: () => invoke<ContentFolders>('content_folders'),
    chooseContentFolder: (kind: 'game' | 'user', initial: string) => invoke<string | null>('choose_content_folder', { kind, initial }),
    setContentFolders: (game: string, user: string) => invoke<ContentFolders>('set_content_folders', { game, user }),
  } : {}),

  async mapSettings(sourceId, revision, settings, signal, target = { scope: 'campaign' }) {
    const reply = await request<MapSettingsReply>('dashboard_map_settings', () => ({ sourceId, revision, settings, flip: null, target }), signal);
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Map settings do not match this campaign');
    contentRevision(nativeDashboardClient).observePresentation(reply.presentationGeneration);
    return reply;
  },
  async flipMapSetting(sourceId, revision, setting, signal, target) {
    const reply = await request<MapSettingsReply>('dashboard_map_settings', () => ({ sourceId, revision, settings: null, flip: setting, target }), signal);
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Map settings do not match this campaign');
    contentRevision(nativeDashboardClient).observePresentation(reply.presentationGeneration);
    return reply;
  },

  preparationPreferences: async (enabled, signal) => validatePreparationPreferences(await request<PreparationPreferences>('dashboard_preparation_preferences', () => ({ enabled }), signal)),
  chooseHistory: async (path, segmentId, choice, signal) => { await request<unknown>('dashboard_history_choice', () => ({ path, segmentId, choice }), signal); },
  preparationStatus: (query, signal) => preparation('dashboard_preparation_status', query, signal),
  async preparedQuery<T>(source: DashboardSource, product: string, params: unknown, options: PreparedQueryOptions): Promise<T> {
    const query = { sourceId: source.id, revision: source.revision, product, params };
    const reply = await request<ProductReply<T>>('dashboard_prepared_query', () => ({ query, refresh: options.refresh ?? false }), options.signal);
    return preparedReplyData(validateProductReply(reply, query), options, params);
  },
  prepareSource: (query, signal) => preparation('dashboard_prepare_source', query, signal),
  cancelPreparation: (query, signal) => preparation('dashboard_cancel_preparation', query, signal),
  clearPreparation: (query, signal) => preparation('dashboard_clear_preparation', query, signal),

  async openSource(path, signal): Promise<DashboardSource> {
    if (signal.aborted) throw cancelled();
    const source = await request<SourceReply>('dashboard_open_source', () => ({ path }), signal,
      late => invoke('dashboard_release_source', { sourceId: late.id }));
    contentRevision(nativeDashboardClient).observePresentation(source.presentationGeneration);
    return { id: source.id, path: source.path, revision: source.revision, metadata: { ...source.metadata, revision: source.revision } };
  },

  product<T>(query: import('./client').ProductRequest, signal: AbortSignal) {
    return request<ProductReply<T>>('dashboard_product', () => ({ query }), signal);
  },

  releaseSource,

  async setMapViews(views) {
    if (desktopAvailable) await invoke('set_dashboard_maps', { views });
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
};
