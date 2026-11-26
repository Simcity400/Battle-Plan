import { isSlotKey } from '../../dashboard/countryKey.ts';
import type { DatedCountrySelection } from '../../dashboard/datedCountry.ts';
import type { Holder } from '../../dashboard/regionGate.ts';

export type BranchFamilyLevel = 'family:0' | 'family:1' | 'family:2';

export function parseBranchFamily(subject: string | null | undefined): BranchFamilyLevel | null {
  if (!subject) return null;
  if (subject === '0' || subject === '1' || subject === '2') return `family:${subject}`;
  const sep = subject.lastIndexOf('\u0000');
  if (sep >= 0) {
    const part = subject.slice(sep + 1);
    if (part === '0' || part === '1' || part === '2') return `family:${part}`;
  }
  return null;
}

export interface BranchDrillOpenInput {
  readonly active: boolean;
  readonly holder: Holder | null;
  readonly extent: readonly [number, number];
  readonly hour: number;
  readonly subject?: string | null;
}

export interface BranchDrillOpen {
  readonly holder: Holder;
  readonly hour: number;
  readonly initialLevel: BranchFamilyLevel | null;
}

export function resolveBranchDrillOpen(input: BranchDrillOpenInput): BranchDrillOpen | null {
  const { active, holder, extent, subject } = input;
  if (!active || !holder) return null;
  if (!Number.isFinite(input.hour)) return null;
  const hour = Math.floor(input.hour);
  if (hour < extent[0] || hour > extent[1]) return null;
  return { holder, hour, initialLevel: parseBranchFamily(subject) };
}

export interface BranchDrillSelectionInput {
  readonly sourceId: string | undefined;
  readonly revision: string | number | undefined;
  readonly branchId: string | undefined;
  readonly chosen: string | null;
  readonly savedSelection: DatedCountrySelection | null;
  readonly branchFirstHour: number | undefined;
  readonly resolvedHolder: Holder | null;
  readonly drillHolder: Holder | null;
  readonly extent: readonly [number, number];
}

export function branchDrillSelectionKey(input: BranchDrillSelectionInput): string {
  const {
    sourceId,
    revision,
    branchId,
    chosen,
    savedSelection,
    branchFirstHour,
    resolvedHolder,
    drillHolder,
    extent,
  } = input;
  const holderKey = !chosen
    ? null
    : isSlotKey(chosen)
      ? chosen
      : savedSelection && branchFirstHour !== undefined
        ? `${savedSelection.tag}@${branchFirstHour + savedSelection.originOffset}`
        : resolvedHolder
          ? `${resolvedHolder.tag}@${resolvedHolder.origin}`
          : drillHolder?.tag === chosen
            ? `${drillHolder.tag}@${drillHolder.origin}`
            : chosen;
  return JSON.stringify([sourceId, revision, branchId, holderKey, extent]);
}

export function branchDrillDetailSubject(
  drillTag: string | null,
  drillHolder: Holder | null | undefined,
  open: boolean,
): string | null {
  if (!open) return null;
  if (drillHolder) return `${drillHolder.tag}@${drillHolder.origin}`;
  return drillTag;
}

export function branchDrillCanFollow(
  playbackCovered: boolean,
  drillHolder: Holder | null | undefined,
  drillTag: string | null,
): boolean {
  return playbackCovered && (drillHolder != null || drillTag !== null);
}
