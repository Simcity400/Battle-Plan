import type { PreparationRequest, PreparationReply, PreparationPreferences } from '../shell/preparation/contract';
import type { RecorderSettings as PanelRecorderSettings } from '../recorder/status';
import type { Compression } from '../recorder/compression';
import type { ContentFolders, MapBounds, MapOptions, ReplayMetadata } from '../shell/api';
import type { CampaignMapSettings, HistoryCacheReply, MapSettingsReply } from '../generated/dashboard';

export type { CampaignMapSettings, HistoryCacheReply, MapGraphics, MapSettingsReply } from '../generated/dashboard';

/** Runtime source identity. Saved layouts keep source roles, not these handles. */
export interface DashboardSource {
  id: string;
  path: string;
  revision: string;
  metadata: ReplayMetadata;
}

/** One data product for one open source. `params` are the product's own typed parameters. */
export interface ProductRequest<P = unknown> {
  sourceId: string;
  revision: string;
  product: string;
  params: P;
}

/** The native reply echoes the lease identity so it can never be attributed to another recording. */
export interface ProductReply<T> {
  sourceId: string;
  revision: string;
  product: string;
  data: T;
  /** A pending or partial prepared reply's gate: the preparation status entries it waits for,
   * `walk` for the status's `coverage` or a family prepared on its own, listed in its `families`. */
  gate?: string[];
}

/** The status entry of the hourly walk's coverage in a prepared reply's gate. */
export const WALK_GATE = 'walk';

/** Per-call options of a prepared query. `gate` receives the reply's gate (undefined when it waits for nothing). */
export interface PreparedQueryOptions {
  signal: AbortSignal;
  /** A refresh of data already shown; first views and user actions leave this false. */
  refresh?: boolean;
  gate?: (entries: readonly string[] | undefined) => void;
}

/** A validated prepared reply's data, after handing its gate to the caller. */
export function preparedReplyData<T>(reply: ProductReply<T>, options: PreparedQueryOptions): T {
  const gate: unknown = reply.gate;
  if (gate !== undefined && !(Array.isArray(gate) && gate.every(entry => typeof entry === 'string' && entry.length > 0)))
    throw new Error('The prepared reply gate is malformed.');
  options.gate?.(gate);
  return reply.data;
}

export function validateProductReply<T>(reply: ProductReply<T>, request: ProductRequest): ProductReply<T> {
  if (!reply || typeof reply !== 'object' || reply.sourceId !== request.sourceId || reply.revision !== request.revision || reply.product !== request.product) {
    throw new Error('The reply does not match this recording and product.');
  }
  return reply;
}

export type RecorderSettings = Omit<PanelRecorderSettings, 'effectiveSaveFolder'>;

export interface ReplayFile { path: string; name: string; folder: string; partial?: import('../generated/dashboard').PartialRecording | null }
export interface ReplayLibrary { folder: string | null; files: ReplayFile[] }

export interface DashboardMapView {
  options?: MapOptions;
  id: string;
  sourceId: string;
  path: string;
  revision: string;
  branchId: string;
  /** Playing: the map may answer with the newest hour built before `hour`. */
  playing?: boolean;
  hour: number;
  fraction: number;
  sequence: number;
  /** Changes only on a seek; omitted for pinned targets. */
  seekSequence?: number;
  bounds: MapBounds;
  clip: MapBounds;
  visible: boolean;
}

/** Acknowledged presentation; queueing a target is not a displayed frame. */
export interface DashboardMapState {
  id: string;
  sourceId: string | null;
  revision: string | null;
  branchId: string | null;
  hour: number | null;
  fraction: number | null;
  sequence: number;
  requestedSequence: number;
  loading: boolean;
  error: string | null;
}

export type MapSettingsTarget = { scope: 'global' } | { scope: 'campaign' } | { scope: 'widget'; id: string };

export interface DashboardClient {
  available: boolean;
  nativeMaps?: boolean;
  openSource(path: string, signal: AbortSignal): Promise<DashboardSource>;
  releaseSource(id: string): Promise<void>;
  preparationPreferences?(enabled: boolean | null, signal: AbortSignal): Promise<PreparationPreferences>;
  preparationStatus?(request: PreparationRequest, signal: AbortSignal): Promise<PreparationReply>;
  prepareSource?(request: PreparationRequest, signal: AbortSignal): Promise<PreparationReply>;
  cancelPreparation?(request: PreparationRequest, signal: AbortSignal): Promise<PreparationReply>;
  clearPreparation?(request: PreparationRequest, signal: AbortSignal): Promise<PreparationReply>;
  product<T>(request: ProductRequest, signal: AbortSignal): Promise<ProductReply<T>>;
  preparedQuery?<T>(source: DashboardSource, product: string, params: unknown, options: PreparedQueryOptions): Promise<T>;
  mapSettings?(sourceId: string, revision: string, settings: CampaignMapSettings | null, signal: AbortSignal, target?: MapSettingsTarget): Promise<MapSettingsReply>;
  /** Flips a setting of the target's stored choices on the service, under the lock its saves take: the opposite of
   * its value when the flip runs, every other choice untouched. */
  flipMapSetting?(sourceId: string, revision: string, setting: 'dayNight' | 'alliedPlans', signal: AbortSignal, target: MapSettingsTarget): Promise<MapSettingsReply>;
  /** The widget history cache's size, after clearing it when asked. */
  historyCache?(clear: boolean, signal: AbortSignal): Promise<HistoryCacheReply>;
  contentFolders?(signal: AbortSignal): Promise<ContentFolders>;
  chooseContentFolder?(kind: 'game' | 'user', initial: string): Promise<string | null>;
  setContentFolders?(game: string, user: string): Promise<ContentFolders>;
  replayLibrary?(signal: AbortSignal): Promise<ReplayLibrary>;
  /** Changes when the library's recordings change; reads no recording. */
  replayLibraryStamp?(): Promise<string>;
  setReplayFolder?(folder: string): Promise<ReplayLibrary>;
  chooseReplayFolder?(initial?: string): Promise<string | null>;
  chooseRecording?(): Promise<string | null>;
  /** Rename a recording's file to the name the user chose; resolves to its new path. */
  renameRecording?(path: string, name: string): Promise<string>;
  /** Where each recording is now, by its folder and campaign; null when not found alone. */
  locateRecordings?(recordings: { path: string; campaignKey: string }[]): Promise<(string | null)[]>;
  recorderSettings?(): Promise<RecorderSettings>;
  setRecordingCompression?(compression: Compression): Promise<RecorderSettings>;
  setMapViews(views: DashboardMapView[]): Promise<void>;
  onMapStates(callback: (states: DashboardMapState[]) => void): Promise<() => void>;
  /** The native map picker changed a view's options; later layouts must carry them. */
  onMapOptions?(callback: (event: MapOptionsEvent) => void): Promise<() => void>;
  /** The native map picker clicked a toggle whose setting the host owns. */
  onMapToggle?(callback: (event: MapToggleEvent) => void): Promise<() => void>;
}

export interface MapOptionsEvent { viewId: string; options: MapOptions }
/** A picker toggle click: the host flips its own value of the setting. */
export interface MapToggle { setting: 'dayNight' | 'alliedPlans' | 'fog' }
export interface MapToggleEvent { viewId: string; toggle: MapToggle }
