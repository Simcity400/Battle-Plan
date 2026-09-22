import type { ContentFolders, CountrySeries, MapBounds, ReplayMetadata } from '../api';
import type { LandOwners } from '../generated/analytics';
import type { FocusHistory, ResearchHistory } from '../generated/analytics';

export interface FocusRequest { sourceId: string; revision: string; branchId: string; tag: string }
export interface FocusReply extends FocusRequest { history: FocusHistory; artwork: { token: string; name: string | null; description: string | null; baseDurationDays: number | null; icon: string | null }[]; warnings: string[] }

export interface ResearchReply extends FocusRequest { history: ResearchHistory; artwork: FocusReply['artwork']; warnings: string[] }

/** Runtime source identity. Saved layouts keep source roles, not these handles. */
export interface DashboardSource {
  id: string;
  path: string;
  revision: string;
  metadata: ReplayMetadata;
}

export interface CountryFlagRequest { sourceId: string; revision: string; branchId: string; hour: number; tags: string[]; size: 'full' | 'small' }
export interface CountryFlagImage { tag: string; asset: string; width: number; height: number; pixels: ArrayLike<number> }
export interface CountryFlagReply { sourceId: string; revision: string; branchId: string; hour: number; flags: CountryFlagImage[]; warnings: string[]; size: 'full' | 'small' }
export interface LandOwnersReply extends LandOwners { sourceId: string; revision: string }
export interface ReplayFile { path: string; name: string; folder: string }
export interface ReplayLibrary { folder: string | null; files: ReplayFile[] }

/** Explicit data demand; the client's transport does not own widget time modes. */
export interface DashboardQuery {
  sourceId: string;
  revision: string;
  branchId: string;
  countryIds: string[];
  firstHour: number;
  lastHour: number;
  kind: 'exact' | 'history';
  maxPoints: number;
}

export interface DashboardResult {
  sourceId: string;
  revision: string;
  series: CountrySeries[];
}

export interface DashboardMapView {
  options?: import("../api").MapOptions;
  id: string;
  sourceId: string;
  path: string;
  revision: string;
  branchId: string;
  hour: number;
  fraction: number;
  sequence: number;
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

export interface DashboardClient {
  researchHistory?(request: FocusRequest, signal: AbortSignal): Promise<ResearchReply>;
  focusHistory?(request: FocusRequest, signal: AbortSignal): Promise<FocusReply>;
  mapSettings?(sourceId: string, revision: string, settings: CampaignMapSettings | null, signal: AbortSignal, target?: MapSettingsTarget): Promise<MapSettingsReply>;
  contentFolders?(signal: AbortSignal): Promise<ContentFolders>;
  chooseContentFolder?(kind: 'game' | 'user', initial: string): Promise<string | null>;
  setContentFolders?(game: string, user: string): Promise<ContentFolders>;
  available: boolean;
  nativeMaps?: boolean;
  replayLibrary?(signal: AbortSignal): Promise<ReplayLibrary>;
  setReplayFolder?(folder: string): Promise<ReplayLibrary>;
  chooseReplayFolder?(initial?: string): Promise<string | null>;
  chooseRecording?(): Promise<string | null>;
  countryFlags?(request: CountryFlagRequest, signal: AbortSignal): Promise<CountryFlagReply>;
  landOwners?(sourceId: string, revision: string, signal: AbortSignal): Promise<LandOwnersReply>;
  openSource(path: string, signal: AbortSignal): Promise<DashboardSource>;
  query(request: DashboardQuery, signal: AbortSignal, priority?: 'current' | 'history' | 'prefetch'): Promise<DashboardResult>;
  releaseSource(id: string): Promise<void>;
  setMapViews(views: DashboardMapView[]): Promise<void>;
  onMapStates(callback: (states: DashboardMapState[]) => void): Promise<() => void>;
}

export interface MapGraphics { textureQuality: number; samples: number; trees: boolean; shadows: boolean; rivers: boolean; buildings: boolean; units: boolean; citySprawl: boolean; weather: boolean }
export type MapSettingsTarget = { scope: 'global' } | { scope: 'campaign' } | { scope: 'widget'; id: string };
export interface CampaignMapSettings { useGameGraphics: boolean; graphics: MapGraphics; visualMods: string[] | null }
export interface MapSettingsReply { inheritedSettings: CampaignMapSettings; gameGraphics: MapGraphics | null; gameGraphicsError: string | null; sampleCounts: number[]; sourceId: string; revision: string; settings: CampaignMapSettings; recordedMods: string[]; mods: { id: string; name: string; recordedId: string; reason: string | null }[] }
