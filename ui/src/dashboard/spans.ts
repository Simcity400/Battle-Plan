import type { Span } from '../charts/series';

/** Recorded spans are whole hours, ordered and disjoint; missing hours stay gaps. */
export function validSpans(value: unknown): value is Span[] {
  return Array.isArray(value) && value.every((span, index) => span !== null && typeof span === 'object'
    && Number.isSafeInteger(span.firstHour) && Number.isSafeInteger(span.lastHour) && span.firstHour <= span.lastHour
    && (index === 0 || value[index - 1].lastHour < span.firstHour));
}
