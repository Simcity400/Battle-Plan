import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { newId } from '../shell/ids.ts';
import { desktopAvailable } from '../shell/api';
import type { SourceReply } from '../generated/dashboard';
import type { CompactionProgress, DashboardClient, DashboardMapState, DashboardSource, HistoryCacheReply, MapSettingsReply, ProductReply, RecorderSettings, RecordingCompression, ReplayLibrary } from './client';
import { replyLengths } from './products.ts';

function cancelled(): DOMException {
  return new DOMException('Request cancelled', 'AbortError');
}

/** Data replies arrive as raw JSON bytes; the host never parses them. */
async function rawInvoke<T>(command: string, args: Record<string, unknown>): Promise<T> {
  const result = await invoke<unknown>(command, args);
  if (!(result instanceof ArrayBuffer || result instanceof Uint8Array)) return result as T;
  const reply: unknown = JSON.parse(new TextDecoder().decode(result));
  if (reply && typeof reply === 'object') replyLengths.set(reply, result.byteLength);
  return reply as T;
}

async function request<T>(
  command: string,
  args: (requestId: string) => Record<string, unknown>,
  signal: AbortSignal,
  releaseLate?: (result: T) => Promise<void>,
): Promise<T> {
  if (!desktopAvailable) throw new Error('Open the desktop app to read recordings and display native maps.');
  if (signal.aborted) throw cancelled();
  const requestId = newId();
  const abort = () => { void invoke('dashboard_cancel', { requestId }).catch(() => {}); };
  signal.addEventListener('abort', abort, { once: true });
  try {
    const result = await rawInvoke<T>(command, { ...args(requestId), requestId });
    if (signal.aborted) {
      await releaseLate?.(result).catch(() => {});
      throw cancelled();
    }
    return result;
  } catch (error) {
    if (signal.aborted) throw cancelled();
    throw error;
  } finally {
    signal.removeEventListener('abort', abort);
  }
}

const retainedPaths = new Map<string, string>();
async function releaseSource(sourceId: string): Promise<void> {
  if (!desktopAvailable) return;
  await invoke('dashboard_release_source', { sourceId });
  const path = retainedPaths.get(sourceId);
  if (path !== undefined) {
    retainedPaths.delete(sourceId);
    try { await invoke('set_compaction_recording_open', { path, open: false }); }
    catch (error) { retainedPaths.set(sourceId, path); throw error; }
  }
}

export const nativeDashboardClient: DashboardClient = {
  available: desktopAvailable,
  // Outside the desktop app there is no host to list or pick files.
  ...(desktopAvailable ? {
    replayLibrary: () => invoke<ReplayLibrary>('replay_library'),
    setReplayFolder: (folder: string) => invoke<ReplayLibrary>('set_replay_folder', { folder }),
    chooseReplayFolder: (initial?: string) => invoke<string | null>('choose_replay_folder', { initial: initial ?? null }),
    chooseRecording: () => invoke<string | null>('choose_recording'),
    recorderSettings: () => invoke<RecorderSettings>('recorder_settings'),
    setRecordingCompression: (compression: RecordingCompression) => invoke<RecorderSettings>('set_recording_compression', { compression }),
    compactRecording: (path: string) => invoke<CompactionProgress>('compact_recording', { path }),
    compactReplayFolder: () => invoke<CompactionProgress>('compact_replay_folder'),
    compactionProgress: () => invoke<CompactionProgress>('compaction_progress'),
    recordingCompacted: (path: string) => invoke<boolean>('recording_compaction_status', { path }),
    onRecordingCompacted: (callback: (path: string) => void) => listen<string>('recording-compacted', event => callback(event.payload)),
  } : {}),

  async mapSettings(sourceId, revision, settings, signal, target = { scope: 'campaign' }) {
    const reply = await request<MapSettingsReply>('dashboard_map_settings', () => ({ sourceId, revision, settings, target }), signal);
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Map settings do not match this campaign');
    return reply;
  },

  historyCache: (clear, signal) => request<HistoryCacheReply>('dashboard_history_cache', () => ({ clear }), signal),

  async openSource(path, signal): Promise<DashboardSource> {
    if (signal.aborted) throw cancelled();
    // Retain before opening, including pending opens, so automatic work cannot
    // change the revision between opening metadata and installing the binding.
    await invoke('set_compaction_recording_open', { path, open: true });
    try {
      const source = await request<SourceReply>('dashboard_open_source', () => ({ path }), signal,
        late => invoke('dashboard_release_source', { sourceId: late.id }));
      retainedPaths.set(source.id, path);
      return { id: source.id, path: source.path, revision: source.revision, metadata: { ...source.metadata, revision: source.revision } };
    } catch (error) {
      await invoke('set_compaction_recording_open', { path, open: false });
      throw error;
    }
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
};
