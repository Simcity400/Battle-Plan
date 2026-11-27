import type { ExactEntry } from '../../families/exactScene.ts';
import type { ProductRequest } from '../request.ts';
import { coverageAt, type CoveredBranch, type Outside } from '../regionTime.ts';
import type { StripDensity } from './lattice.ts';

export interface StripTrack {
  readonly branch: CoveredBranch;
  readonly axisOffset: number;
}

export function stripCoverageAt(track: StripTrack, hour: number) {
  return coverageAt(track, hour + track.axisOffset);
}

export interface StripFrame {
  readonly page: number;
  readonly frame: number;
  readonly hour: number;
  readonly density: StripDensity;
  readonly gap: Outside | null;
  readonly measure?: true;
}

export type StripScenePlan =
  | { readonly recorded: number; readonly parts: readonly (ExactEntry & { readonly slots: readonly string[] } | null)[]; readonly allowEmpty: boolean; readonly options?: unknown }
  | { readonly outside: Outside };

export interface StripKeyframe {
  readonly asks: readonly { slot: string; request: ProductRequest | null }[];
  readonly scene?: StripScenePlan;
}

export interface StripWork {
  leaves: number;
  bytes: number;
}
