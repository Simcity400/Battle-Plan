import type { LiveValues } from '../charts/live';
import type { TooltipContent } from '../charts/tooltip';
import type { FollowedAnswer } from '../dashboard/follow/message';
import type { AuditRow } from '../draw/notifications';

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

export interface FamilyDefinition<TModel, TOptions> {
  readonly kind: FamilyId;
  audit(model: TModel, options: TOptions): readonly AuditRow[];
  mirror(model: TModel, options: TOptions, window?: { readonly first: number; readonly count: number }): MirrorModel;
  tooltip(model: TModel, itemKey: string, hour: number, options: TOptions): TooltipContent | null;
}

export interface WidgetDeclaration<TFamily extends FamilyId, TModel, TOptions> extends FamilyProjection<TModel, TOptions> {
  readonly family: TFamily;
  readonly subject: string;
  readonly things: string;
}
