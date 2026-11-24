import type { RecorderSettings as PanelRecorderSettings } from '../recorder/status';
import type { Compression } from '../recorder/compression';
import type { ContentFolders, MapBounds, MapOptions, ReplayMetadata } from '../shell/api';
import type { CampaignMapSettings, CounterFilter, MapAudio, MapGraphics, MapSettingsReply } from '../generated/dashboard';

export type { CampaignMapSettings, MapGraphics, MapSettingsReply } from '../generated/dashboard';

export interface DashboardSource {
  id: string;
  path: string;
  revision: string;
  metadata: ReplayMetadata;
}

export type { ProductRequest } from './request.ts';
import type { ProductRequest } from './request.ts';

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

export type RecorderSettings = Omit<PanelRecorderSettings, 'effectiveSaveFolder'>;

export interface ReplayFile { path: string; name: string; folder: string; partial?: import('../generated/dashboard').PartialRecording | null }
export interface ReplayLibrary { folder: string | null; files: ReplayFile[]; warning?: string | null }

export interface DashboardMapView {
  options?: MapOptions;
  id: string;
  sourceId: string;
  path: string;
  revision: string;
  branchId: string;
  playing?: boolean;
  hoursPerSecond: number;
  hour: number;
  fraction: number;
  sequence: number;
  seekSequence?: number;
  composedAt: number;
  clockId: string;
  bounds: MapBounds;
  clip: MapBounds;
  visible: boolean;
}

export interface DashboardMapState {
  id: string;
  clockId: string;
  sourceId: string | null;
  revision: string | null;
  branchId: string | null;
  hour: number | null;
  fraction: number | null;
  sequence: number;
  requestedSequence: number;
  loading: boolean;
  behind?: number | null;
  behindReason?: string | null;
  error: string | null;
  mapMode?: string | null;
  drawnAt?: number | null;
  hover?: string | null;
  tooltip?: string | null;
  tooltipDelayMs?: number | null;
  zoom?: number | null;
  center?: [number, number] | null;
  options?: MapOptions | null;
  applied?: { graphics: MapGraphics; audio: MapAudio; counters: CounterFilter; visualMods: string[] } | null;
}

export type MapSettingsTarget = { scope: 'global' } | { scope: 'campaign' } | { scope: 'widget'; id: string };

export interface GameWindowRequest {
  sourceId: string;
  revision: string;
  branchId: string;
  hour: number;
  kind: string;
  target?: unknown;
  scale: number;
  subView?: string | null;
}

export interface GameWindowTooltip {
  title: string;
  body: string | null;
  detail: string | null;
}

export interface GameWindowAlphaMask {
  readonly width: number;
  readonly height: number;
  readonly bitsBase64: string;
}

export interface GameWindowRegion {
  path: string;
  rect: readonly [number, number, number, number];
  clip: readonly [number, number, number, number] | null;
  disabled: boolean;
  close: boolean;
  passthrough?: boolean;
  view?: string | null;
  tooltip: GameWindowTooltip | null;
  alphaMask?: GameWindowAlphaMask | null;
}

export interface GameWindowRender {
  title?: string | null;
  pngBase64: string;
  logicalSize: readonly [number, number];
  pixelSize: readonly [number, number];
  scale: number;
  readHour: number | null;
  artFromCurrentInstall: boolean;
  artLabel: string | null;
  regions: readonly GameWindowRegion[];
  warnings: readonly string[];
}

export interface DashboardClient {
  available: boolean;
  nativeMaps?: boolean;
  openSource(path: string, signal: AbortSignal): Promise<DashboardSource>;
  releaseSource(id: string): Promise<void>;
  chooseHistory?(path: string, segmentId: string, choice: string | null, signal: AbortSignal): Promise<void>;
  product<T>(request: ProductRequest, signal: AbortSignal): Promise<ProductReply<T>>;
  renderGameWindow?(request: GameWindowRequest, signal: AbortSignal): Promise<GameWindowRender>;
  mapSettings?(sourceId: string, revision: string, settings: CampaignMapSettings | null, signal: AbortSignal, target?: MapSettingsTarget): Promise<MapSettingsReply>;
  flipMapSetting?(sourceId: string, revision: string, setting: 'dayNight' | 'alliedPlans', signal: AbortSignal, target: MapSettingsTarget): Promise<MapSettingsReply>;
  contentFolders?(signal: AbortSignal): Promise<ContentFolders>;
  chooseContentFolder?(kind: 'game' | 'user', initial: string): Promise<string | null>;
  setContentFolders?(game: string, user: string): Promise<ContentFolders>;
  replayLibrary?(signal: AbortSignal): Promise<ReplayLibrary>;
  replayLibraryStamp?(): Promise<string>;
  setReplayFolder?(folder: string): Promise<ReplayLibrary>;
  chooseReplayFolder?(initial?: string): Promise<string | null>;
  chooseRecording?(): Promise<string | null>;
  renameRecording?(path: string, name: string): Promise<{ path: string; warning: string | null }>;
  locateRecordings?(recordings: { path: string; campaignKey: string }[]): Promise<(string | null)[]>;
  recorderSettings?(): Promise<RecorderSettings>;
  setRecordingCompression?(compression: Compression): Promise<RecorderSettings>;
  setAutoCompress?(enabled: boolean): Promise<RecorderSettings>;
  setMapViews(clockId: string, views: DashboardMapView[]): Promise<void>;
  onMapStates(callback: (states: DashboardMapState[]) => void): Promise<() => void>;
  onMapOptions?(callback: (event: MapOptionsEvent) => void): Promise<() => void>;
  onMapToggle?(callback: (event: MapToggleEvent) => void): Promise<() => void>;
  onMapObserve?(callback: (event: MapObserveEvent) => void): Promise<() => void>;
  openFollow?(ports: readonly MessagePort[]): FollowOpen;
}

export type OpenedFollow = string | void | { readonly session?: string; close?(): void };
export type FollowOpen = Promise<OpenedFollow> | { readonly ready: Promise<OpenedFollow>; close(): void };

export interface MapOptionsEvent { viewId: string; options: MapOptions }
export interface MapToggle { setting: 'dayNight' | 'alliedPlans' | 'fog' }
export interface MapToggleEvent { viewId: string; toggle: MapToggle }
export interface MapObserveEvent { viewId: string; country: { slot: number; tag: string; sequence: number } }
