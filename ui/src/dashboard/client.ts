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
}

export function validateProductReply<T>(reply: ProductReply<T>, request: ProductRequest): ProductReply<T> {
  if (!reply || typeof reply !== 'object' || reply.sourceId !== request.sourceId || reply.revision !== request.revision || reply.product !== request.product) {
    throw new Error('The reply does not match this recording and product.');
  }
  return reply;
}

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
  product<T>(request: ProductRequest, signal: AbortSignal): Promise<ProductReply<T>>;
  mapSettings?(sourceId: string, revision: string, settings: CampaignMapSettings | null, signal: AbortSignal, target?: MapSettingsTarget): Promise<MapSettingsReply>;
  /** The widget history cache's size, after clearing it when asked. */
  historyCache?(clear: boolean, signal: AbortSignal): Promise<HistoryCacheReply>;
  contentFolders?(signal: AbortSignal): Promise<ContentFolders>;
  chooseContentFolder?(kind: 'game' | 'user', initial: string): Promise<string | null>;
  setContentFolders?(game: string, user: string): Promise<ContentFolders>;
  replayLibrary?(signal: AbortSignal): Promise<ReplayLibrary>;
  setReplayFolder?(folder: string): Promise<ReplayLibrary>;
  chooseReplayFolder?(initial?: string): Promise<string | null>;
  chooseRecording?(): Promise<string | null>;
  setMapViews(views: DashboardMapView[]): Promise<void>;
  onMapStates(callback: (states: DashboardMapState[]) => void): Promise<() => void>;
}
