import type { FamilyHostAdapter, ParsedFamilyHit } from './adapter.ts';
import { hoveredSubject, type InstalledRecord } from './installed.ts';

export type TooltipRowSource = 'pointer' | 'focus';

export function rowPublisher<T>(resolve: (input: T) => string | null, target: () => ((subject: string | null) => void) | undefined): (input: T) => void {
  let last: string | null = null;
  return input => {
    const subject = resolve(input);
    if (subject === last) return;
    last = subject;
    target()?.(subject);
  };
}

export function rowHoverPublisher(
  adapter: () => FamilyHostAdapter<any, any, any, any>,
  record: () => InstalledRecord | null,
  target: () => ((subject: string | null) => void) | undefined,
): (hit: ParsedFamilyHit | null) => void {
  return rowPublisher(hit => hoveredSubject(adapter(), record(), hit), target);
}
