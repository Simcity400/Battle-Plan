import type { Span } from '../../charts/series';
import { isMember } from '../../dashboard/groups.ts';

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
  readonly tag: string | undefined;
  readonly extent: readonly [number, number];
  readonly membership: ReadonlyMap<string, readonly Span[]> | undefined;
  readonly hour: number;
  readonly subject?: string | null;
}

export interface BranchDrillOpen {
  readonly tag: string;
  readonly hour: number;
  readonly initialLevel: BranchFamilyLevel | null;
}

export function resolveBranchDrillOpen(input: BranchDrillOpenInput): BranchDrillOpen | null {
  const { active, tag, extent, membership, subject } = input;
  if (!active || !tag) return null;
  if (subject?.startsWith('gap:')) return null;
  if (!Number.isFinite(input.hour)) return null;
  const hour = Math.floor(input.hour);
  if (hour < extent[0] || hour > extent[1]) return null;
  if (!isMember(membership?.get(tag), hour)) return null;
  return { tag, hour, initialLevel: parseBranchFamily(subject) };
}
