import type { RecorderSettings as PanelRecorderSettings } from '../recorder/status';
import type { Compression } from '../recorder/compression';
import type { ContentFolders, MapBounds, MapOptions, ReplayMetadata } from '../shell/api';
import type { CampaignMapSettings, CounterFilter, MapAudio, MapGraphics, MapSettingsReply } from '../generated/dashboard';

export type { CampaignMapSettings, MapGraphics, MapSettingsReply } from '../generated/dashboard';

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
  /** The first track hour of the rows asked, where the params do not name it (a window or a page by its index). Never
   * sent: the perf log tells by it a request for a block ahead of the head from one at it. */
  asksFrom?: number;
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
  hoursPerSecond: number;
  hour: number;
  fraction: number;
  sequence: number;
  /** Changes only on a seek; omitted for pinned targets. */
  seekSequence?: number;
  /** When the page composed this target, on its own monotonic clock: the map places its anchor
   * by that time, so delivery delay moves no clock. */
  composedAt: number;
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
  /** Playing, how many hours the rendered frame trails the map's clock. */
  behind?: number | null;
  /** Playing, why a newer hour the clock passed is not shown: it failed or waits for memory. */
  behindReason?: string | null;
  error: string | null;
  /** The map mode token of the hour on screen; null until a frame is drawn. */
  mapMode?: string | null;
  /** When the viewer drew the first frame showing this, in milliseconds since the Unix epoch. */
  drawnAt?: number | null;
  /** What the pointer highlights in that frame: `button:…`, `division:<id>`, `railway_gun:<id>` or `task_force:<id>`. */
  hover?: string | null;
  /** The map tooltip that frame drew, as `province:<id>`. */
  tooltip?: string | null;
  /** The game's own delay before a tooltip shows on a still pointer. */
  tooltipDelayMs?: number | null;
  /** The camera's eye height in that frame, its zoom. */
  zoom?: number | null;
  /** The camera's look-at point in that frame, world (x, z). */
  center?: [number, number] | null;
  /** The view's options that frame drew with. */
  options?: MapOptions | null;
  /** The graphics, sound, counters and visual mods (ids, sorted) that frame drew and played with. */
  applied?: { graphics: MapGraphics; audio: MapAudio; counters: CounterFilter; visualMods: string[] } | null;
}

export type MapSettingsTarget = { scope: 'global' } | { scope: 'campaign' } | { scope: 'widget'; id: string };

export interface DashboardClient {
  available: boolean;
  nativeMaps?: boolean;
  openSource(path: string, signal: AbortSignal): Promise<DashboardSource>;
  releaseSource(id: string): Promise<void>;
  /** Joins a recorded stretch of play to the history chosen for it: the segment id it goes on from, or null to let
   * the recording decide again. The recording then opens again under the choice. */
  chooseHistory?(path: string, segmentId: string, choice: string | null, signal: AbortSignal): Promise<void>;
  product<T>(request: ProductRequest, signal: AbortSignal): Promise<ProductReply<T>>;
  mapSettings?(sourceId: string, revision: string, settings: CampaignMapSettings | null, signal: AbortSignal, target?: MapSettingsTarget): Promise<MapSettingsReply>;
  /** Flips a setting of the target's stored choices on the service, under the lock its saves take: the opposite of
   * its value when the flip runs, every other choice untouched. */
  flipMapSetting?(sourceId: string, revision: string, setting: 'dayNight' | 'alliedPlans', signal: AbortSignal, target: MapSettingsTarget): Promise<MapSettingsReply>;
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
  setAutoCompress?(enabled: boolean): Promise<RecorderSettings>;
  setMapViews(views: DashboardMapView[]): Promise<void>;
  onMapStates(callback: (states: DashboardMapState[]) => void): Promise<() => void>;
  /** The native map picker changed a view's options; later layouts must carry them. */
  onMapOptions?(callback: (event: MapOptionsEvent) => void): Promise<() => void>;
  /** The native map picker clicked a toggle whose setting the host owns. */
  onMapToggle?(callback: (event: MapToggleEvent) => void): Promise<() => void>;
  /** A Ctrl+click on a native map chose a viewing country, as the game's observer switches the observed one. */
  onMapObserve?(callback: (event: MapObserveEvent) => void): Promise<() => void>;
  /** Opens the dated stream into the drawing worker. */
  openFollow?(port: MessagePort): Promise<void>;
}

export interface MapOptionsEvent { viewId: string; options: MapOptions }
/** A picker toggle click: the host flips its own value of the setting. */
export interface MapToggle { setting: 'dayNight' | 'alliedPlans' | 'fog' }
export interface MapToggleEvent { viewId: string; toggle: MapToggle }
/** The country a Ctrl+click chose: its canonical slot, which the choice follows, its tag at the click's hour, and the
 * switch's sequence, which the view's options echo once the choice applies it. */
export interface MapObserveEvent { viewId: string; country: { slot: number; tag: string; sequence: number } }
