import type { LiveValues } from '../charts/live';
import type { DashboardSource } from '../dashboard/client';
import type { Channel } from '../dashboard/follow/facts';
import type { FollowedAnswer } from '../dashboard/follow/message';
import type { FollowQuery } from '../dashboard/follow/pipeline.ts';
import type { CountryLooksMap } from './looks.ts';

export type FamilyId = 'table' | 'race' | 'series' | 'log' | 'timeline' | 'parts' | 'pairs' | 'meters';

export interface FamilySourceSubscription {
  readonly name: string;
  readonly source: DashboardSource;
  readonly branch: string;
  readonly offset: number;
  readonly query: FollowQuery;
  readonly channel: Channel;
  readonly product?: string;
}

export interface FamilyTheme {
  surface: string;
  raised?: string;
  dark?: boolean;
  text: string;
  muted: string;
  line: string;
  lineSoft: string;
  hairline?: string;
  hairlineStrong?: string;
  bandDivider?: string;
  headerBand?: string;
  rowHover?: string;
  rowSelected?: string;
  childTint?: string;
  accent: string;
  blue: string;
  red: string;
  shortage?: string;
  amber: string;
  green: string;
  grey: string;
  barMuted?: string;
  barTrack?: string;
  gridLine?: string;
  zeroLine?: string;
  tailLine?: string;
  playhead?: string;
  crosshair?: string;
  dateTabBg?: string;
  dateTabText?: string;
  hoverTabBg?: string;
  hoverTabText?: string;
  hatch: string;
  hatchEdge?: string;
  font: string;
  fontSize: { small: number; body: number };
  lineWidth: number;
  hatchSpacing: number;
  country?: Readonly<Record<string, string>>;
  countryLooks?: CountryLooksMap;
}

export interface FamilySources {
  readonly answers: ReadonlyMap<string, FollowedAnswer | undefined>;
  readonly panels: ReadonlyMap<string, unknown>;
  readonly live: ReadonlyMap<string, LiveValues<number | bigint | string> | null>;
  readonly refused?: ReadonlyMap<string, string>;
}

export const EMPTY_SOURCES: FamilySources = {
  answers: new Map(),
  panels: new Map(),
  live: new Map(),
  refused: new Map(),
};

export interface FamilyProjection<TModel, TOptions> {
  readonly kind: string;
  readonly family: FamilyId;
  project(sources: FamilySources, hour: number, options: TOptions): TModel;
  nextChange?(sources: FamilySources, hour: number, options: TOptions): number;
}

export interface MirrorCell {
  readonly column: string;
  readonly spoken: string;
}

export interface MirrorRow {
  readonly key: string;
  readonly name: string;
  readonly rowIndex: number;
  readonly rank?: number;
  readonly level?: number;
  readonly group?: boolean;
  readonly open?: boolean;
  readonly groupHeader?: boolean;
  readonly cells: readonly MirrorCell[];
}

export interface MirrorModel {
  readonly first: number;
  readonly total: number;
  readonly rows: readonly MirrorRow[];
}
