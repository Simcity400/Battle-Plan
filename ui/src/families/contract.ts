import type { LiveValues } from '../charts/live';
import type { FollowedAnswer } from '../dashboard/follow/message';

export type FamilyId = 'table' | 'race' | 'series' | 'log' | 'timeline' | 'parts' | 'pairs' | 'meters';

export interface FamilySources {
  readonly answers: ReadonlyMap<string, FollowedAnswer | undefined>;
  readonly panels: ReadonlyMap<string, unknown>;
  readonly live: ReadonlyMap<string, LiveValues<number | bigint | string> | null>;
}

export const EMPTY_SOURCES: FamilySources = {
  answers: new Map(),
  panels: new Map(),
  live: new Map(),
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
