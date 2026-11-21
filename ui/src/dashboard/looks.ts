import type { Failure } from './queryError';
import { FAILED_FACT, isFailedFact } from './countryFacts.ts';

export interface Look { name: string | null; color: [number, number, number] | null; flag: string | null; nameFailure?: Failure; flagFailure?: Failure }

export interface LookRead {
  style: { name: string | null; color: [number, number, number] | null } | null | undefined | typeof FAILED_FACT;
  flag: string | null | undefined | typeof FAILED_FACT;
}

const NOTHING: Look = { name: null, color: null, flag: null };

export function nextLooks(shown: ReadonlyMap<string, Look>, tags: readonly string[], read: (tag: string) => LookRead): Map<string, Look> {
  const next = new Map(shown);
  for (const tag of tags) {
    const { style, flag } = read(tag);
    if (style === undefined && flag === undefined) continue;
    const look = { ...(shown.get(tag) ?? NOTHING) };
    if (style !== undefined) {
      look.name = isFailedFact(style) ? null : style?.name ?? null;
      look.color = isFailedFact(style) ? null : style?.color ?? null;
      if (isFailedFact(style)) look.nameFailure = style; else delete look.nameFailure;
    }
    if (flag !== undefined) {
      look.flag = isFailedFact(flag) ? null : flag;
      if (isFailedFact(flag)) look.flagFailure = flag; else delete look.flagFailure;
    }
    next.set(tag, look);
  }
  return next;
}

export function retainLooks(shown: ReadonlyMap<string, Look>, before: ReadonlyMap<string, number>, at: ReadonlyMap<string, number>): Map<string, Look> {
  return new Map([...shown].filter(([tag]) => at.has(tag) && before.get(tag) === at.get(tag)));
}

export function matchesLookSegment(segments: ReadonlyMap<string, number>, tag: string, start: number | null): boolean {
  return start !== null && segments.get(tag) === start;
}

export function sampledLooks(before: { looks: ReadonlyMap<string, Look>; segments: ReadonlyMap<string, number> },
  samples: readonly { tag: string; hour: number; stretch: number }[], read: (sample: { tag: string; hour: number }) => LookRead,
  segments = new Map(samples.map(sample => [sample.tag, sample.stretch]))) {
  const reads = new Map(samples.map(sample => [sample.tag, read(sample)]));
  const looks = nextLooks(retainLooks(before.looks, before.segments, segments), [...reads.keys()], tag => reads.get(tag)!);
  return { looks, segments };
}
