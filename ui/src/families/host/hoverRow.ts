import type { FamilyHostAdapter, ParsedFamilyHit } from './adapter.ts';
import { hoveredSubject, type InstalledRecord } from './installed.ts';

export type TooltipRowSource = 'pointer' | 'focus';

export function rowHoverPublisher(
  adapter: () => FamilyHostAdapter<any, any, any, any>,
  record: () => InstalledRecord | null,
  target: () => ((subject: string | null) => void) | undefined,
): (hit: ParsedFamilyHit | null) => void {
  let last: string | null = null;
  return hit => {
    const subject = hoveredSubject(adapter(), record(), hit);
    if (subject === last) return;
    last = subject;
    target()?.(subject);
  };
}
