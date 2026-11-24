import { categoricalPalette } from '../../charts/palette.ts';
import type { HourSpan } from '../../generated/dashboard';
import { flagArtHandle } from '../art/provider.ts';
import type { ArtHandle, BoundedReadView, DisplayedHour } from '../plan.ts';
import type { TimelineCountryChip } from './model.ts';

export function urlArtHandle(url: string | null | undefined, scope = 'url'): ArtHandle | null {
  if (!url) return null;
  return { scope, choice: url, path: url };
}

export function resolveCountryChip(
  readView: BoundedReadView,
  tag: string | null | undefined,
  atHour?: number,
  options?: { readonly dimmed?: boolean; readonly fallbackName?: string },
): TimelineCountryChip {
  const h = atHour !== undefined ? Math.min(atHour, readView.hour) : readView.hour;
  if (!tag) {
    return {
      tag: '',
      name: options?.fallbackName ?? 'Country not recorded',
      flag: null,
      flagMissing: true,
      color: null,
      dimmed: options?.dimmed,
    };
  }
  const id = readView.who('country', tag, h);
  const name = id.name.status === 'ready'
    ? id.name.value
    : id.name.status === 'pending'
      ? tag
      : options?.fallbackName ?? tag;
  const flag = id.flagChoice?.status === 'ready' ? flagArtHandle(id.flagChoice.value, 'panel') : null;
  const flagMissing = id.flagChoice?.status === 'absent';
  const color = id.colour?.status === 'ready' ? id.colour.value : null;
  return {
    tag,
    name,
    flag,
    flagMissing,
    color,
    dimmed: options?.dimmed,
  };
}

export function paletteAt(surface: string, index: number): string {
  const colors = categoricalPalette(surface);
  return colors[((index % colors.length) + colors.length) % colors.length]!;
}

export function clipSpansToHour(spans: readonly HourSpan[] | undefined, hour: DisplayedHour): HourSpan[] {
  if (!spans?.length) return [];
  const out: HourSpan[] = [];
  for (const s of spans) {
    if (s.firstHour > hour) continue;
    out.push({ firstHour: s.firstHour, lastHour: Math.min(s.lastHour, hour) });
  }
  return out;
}

export function inHourSpans(spans: readonly HourSpan[] | undefined, hour: number): boolean {
  if (!spans?.length) return false;
  for (const s of spans) {
    if (s.firstHour <= hour && hour <= s.lastHour) return true;
  }
  return false;
}

export function formatDurationShort(hours: number): string {
  const days = Math.max(0, Math.floor(hours / 24));
  if (days < 1) return '< 1 day';
  if (days < 60) return `${days} ${days === 1 ? 'day' : 'days'}`;
  const months = Math.floor(days / 30);
  if (months < 24) return `${months} ${months === 1 ? 'month' : 'months'}`;
  const years = Math.floor(days / 365);
  const remMonths = Math.floor((days - years * 365) / 30);
  return remMonths > 0 ? `${years}y ${remMonths}m` : `${years} ${years === 1 ? 'year' : 'years'}`;
}

export function nextBoundaryAfter(hours: readonly number[], current: number): number {
  let lo = 0;
  let hi = hours.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (hours[mid]! <= current) lo = mid + 1;
    else hi = mid;
  }
  return lo < hours.length ? hours[lo]! : Infinity;
}
