import { validateResearchReply } from './researchData';
import { validateFocusReply } from './focusData';
import type { FocusReply, ResearchReply } from './client';
import { newId } from '../ids.ts';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { desktopAvailable } from '../api';
import type { ReplayMetadata as GeneratedMetadata, QueryRequest, QueryResult } from '../generated/analytics';
import type { CountryFlagReply, MapSettingsReply, DashboardClient, DashboardMapState, DashboardSource, LandOwnersReply, ReplayLibrary } from './client';

interface OpenedSource {
  id: string;
  path: string;
  revision: string;
  metadata: GeneratedMetadata;
}

function cancelled(): DOMException {
  return new DOMException('Request cancelled', 'AbortError');
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
    const result = await invoke<T>(command, { ...args(requestId), requestId });
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

async function releaseSource(sourceId: string): Promise<void> {
  if (desktopAvailable) await invoke('dashboard_release_source', { sourceId });
}

export const nativeDashboardClient: DashboardClient = {
  async mapSettings(sourceId, revision, settings, signal, target = { scope: 'campaign' }) {
    const reply = await request<MapSettingsReply>('dashboard_map_settings', () => ({ sourceId, revision, settings, target }), signal);
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Map settings do not match this campaign');
    if (!reply.inheritedSettings) throw new Error('The app or local bridge needs updating to support global and widget settings.');
    return reply;
  },
  available: desktopAvailable,
  replayLibrary: () => invoke<ReplayLibrary>('replay_library'),
  setReplayFolder: folder => invoke<ReplayLibrary>('set_replay_folder', { folder }),
  chooseReplayFolder: initial => invoke<string | null>('choose_replay_folder', { initial: initial ?? null }),
  chooseRecording: () => invoke<string | null>('choose_recording'),
  async landOwners(sourceId, revision, signal) {
    const result = await request<LandOwnersReply>('dashboard_land_owners', () => ({ sourceId, revision }), signal);
    if (result.sourceId !== sourceId || result.revision !== revision) throw new Error('Countries do not match this recording');
    return result;
  },

  async focusHistory(query, signal) {
    const reply = await request<FocusReply>('dashboard_focus_history', () => ({ query }), signal);
    return validateFocusReply(reply, query);
  },

  async researchHistory(query, signal) {
    const reply = await request<ResearchReply>('dashboard_research_history', () => ({ query }), signal);
    return validateResearchReply(reply, query);
  },
  async countryFlags(query, signal) {
    const result = await request<CountryFlagReply>('dashboard_country_flags', () => ({ query }), signal);
    if (result.size !== query.size || result.sourceId !== query.sourceId || result.revision !== query.revision || result.branchId !== query.branchId || result.hour !== query.hour
      || result.flags.some((flag) => !query.tags.includes(flag.tag) || flag.width < 1 || flag.height < 1 || flag.width > 128 || flag.height > 128 || flag.pixels.length !== flag.width * flag.height * 4)) {
      throw new Error('Country flags do not match this recording.');
    }
    return { ...result, flags: result.flags.map((flag) => ({ ...flag, pixels: new Uint8ClampedArray(flag.pixels) })) };
  },

  async openSource(path, signal): Promise<DashboardSource> {
    const source = await request<OpenedSource>(
      'dashboard_open_source', () => ({ path }), signal, (late) => releaseSource(late.id),
    );
    return {
      id: source.id, path: source.path, revision: source.revision,
      metadata: { ...source.metadata, revision: source.revision },
    };
  },

  async query(query, signal, priority) {
    const kind = query.kind === 'exact' ? 'exactWindow' : 'history';
    const result = await request<QueryResult>('dashboard_query', (requestId) => {
      const wire: QueryRequest = {
        scopeId: requestId, generation: '1', sourceId: query.sourceId,
        revision: query.revision, branchId: query.branchId, countryIds: query.countryIds,
        firstHour: query.firstHour, lastHour: query.lastHour, kind,
        maxPoints: query.kind === 'exact' ? query.lastHour - query.firstHour + 1 : query.maxPoints,
        priority: priority ?? (query.kind === 'exact' ? 'current' : 'history'),
      };
      return { query: wire };
    }, signal);
    if (result.revision !== query.revision || result.branchId !== query.branchId
      || result.firstHour !== query.firstHour || result.lastHour !== query.lastHour || result.kind !== kind
      || result.maxPoints !== query.maxPoints
      || JSON.stringify([...result.countryIds].sort()) !== JSON.stringify([...new Set(query.countryIds)].sort())) {
      throw new Error('The analytics result does not match this recording and time selection.');
    }
    return {
      // The native cache key is shared across leases; keep the widget's binding.
      sourceId: query.sourceId, revision: result.revision,
      series: result.series.map((series) => ({ ...series, revision: result.revision })),
    };
  },

  releaseSource,

  async setMapViews(views) {
    if (desktopAvailable) await invoke('set_dashboard_maps', { views });
  },

  async onMapStates(callback) {
    if (!desktopAvailable) return () => {};
    return listen<DashboardMapState[]>('dashboard-map-state', (event) => callback(event.payload));
  },
};
