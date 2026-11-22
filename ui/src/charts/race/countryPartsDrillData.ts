import type { GatedTime } from '../../dashboard/regionTime';
import type { HeldCountry } from '../../dashboard/regionGate';
import type { PinnedSpec } from '../drill/drillState';
import type { CountryRacePart } from './countryRace';
import type { Run, Span } from '../series';
import type { TooltipContent } from '../tooltip';
import { countryDrillSpans, countryDrillTagAt, countryIdentityKey, historyAt } from '../../dashboard/countryIdentity.ts';
import type { CountryDrillHistory } from '../../dashboard/countryIdentity';
import { shownRuns } from './lines.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { pinPartsRace } from './partsRaceDrill.ts';
import { runAt } from '../series.ts';
import { usWhole } from '../numberFormat.ts';
import { signed32, unsigned32 } from '../../dashboard/validation.ts';

export interface CountryPartsAt { total: number | null; values: readonly (number | null)[]; note?: string }
export interface CountryPartsData {
  histories: readonly CountryDrillHistory[];
  warnings: readonly string[];
  parts: (hour: number) => readonly CountryRacePart[];
  at: (tag: string, hour: number) => CountryPartsAt | null;
  bands: (tag: string, range: Span) => Run[][];
  segmentAt: (tag: string, hour: number) => number | null;
  readHour: (tag: string, hour: number) => number | null;
  keyHours: readonly number[];
  tooltip: (tag: string, hour: number, name: string, colours: ReadonlyMap<string, string>) => Iterable<TooltipContent> | null;
}

export interface CountryPartsSelection { tag: string; hour: number; key: string }

export function countryPartsSelection(histories: readonly CountryDrillHistory[], tag: string, hour: number): CountryPartsSelection | null {
  if (!signed32(hour)) return null;
  const held = historyAt(histories, tag, hour)?.tag;
  if (held === undefined) return null;
  const identity = runAt(histories.find(history => history.tag === held)?.identities ?? [], hour);
  const spans = countryDrillSpans(histories, held, hour);
  return identity && countryDrillTagAt(spans, hour) === held ? { tag: held, hour, key: countryIdentityKey(identity) } : null;
}

export function savedCountryPartsSelection(widget: WidgetSpec, track: BoundTrack | undefined): CountryPartsSelection | null | undefined {
  if (!Object.hasOwn(widget.options ?? {}, 'holder')) return undefined;
  const tag = widget.countries.kind === 'tags' && widget.countries.tags.length === 1 ? widget.countries.tags[0] : null;
  const hour = widget.options?.drillOriginHour;
  if (!track || !tag || !signed32(hour) || hour < track.branch.firstHour || hour > track.branch.lastHour
    || typeof widget.options?.holder !== 'string') return null;
  try {
    const value: unknown = JSON.parse(widget.options.holder);
    if (!Array.isArray(value) || value.length !== 5 || value[0] !== track.source.metadata.campaignKey
      || value[1] !== track.branch.id || value[2] !== tag || !unsigned32(value[3]) || !unsigned32(value[4])) return null;
    return { tag, hour, key: countryIdentityKey({ slot: value[3], countryId: value[4] }) };
  } catch { return null; }
}

export function pinNativeCountryParts(source: WidgetSpec, track: BoundTrack, held: HeldCountry, time: GatedTime, label: string): PinnedSpec {
  const pinned = pinCountryParts(source, track, held, time, label);
  return { ...pinned, options: { ...pinned.options,
    holder: JSON.stringify([track.source.metadata.campaignKey, track.branch.id, held.tag, held.slot, held.countryId]) } };
}

export function bindCountryParts(model: CountryPartsData, tag: string, originHour: number, holder?: string | null) {
  const selected = countryPartsSelection(model.histories, tag, originHour);
  const spans = holder !== undefined && (holder === null || selected?.key !== holder)
    ? new Map<string, readonly Span[]>() : countryDrillSpans(model.histories, tag, originHour);
  const tagAt = (hour: number) => countryDrillTagAt(spans, hour);
  const data: CountryPartsData = {
    ...model,
    at: (_, hour) => { const alias = tagAt(hour); return alias ? model.at(alias, hour) : null; },
    readHour: (_, hour) => { const alias = tagAt(hour); return alias ? model.readHour(alias, hour) : null; },
    segmentAt: (_, hour) => { const alias = tagAt(hour); return alias ? model.segmentAt(alias, hour) : null; },
    tooltip: (_, hour, name, colours) => { const alias = tagAt(hour); return alias ? model.tooltip(alias, hour, name, colours) : null; },
    bands: (_, range) => {
      const bands = Array.from({ length: model.parts(originHour).length + 1 }, () => [] as Run[]);
      for (const [alias, identitySpans] of spans) {
        if (!identitySpans.length) continue;
        model.bands(alias, range).forEach((runs, index) => {
          for (const run of shownRuns(runs, identitySpans, range)) bands[index].push(run);
        });
      }
      const events = new Map<number, number>();
      for (const identitySpans of spans.values()) for (const span of identitySpans) {
        events.set(span.firstHour, (events.get(span.firstHour) ?? 0) + 1);
        events.set(span.lastHour + 1, (events.get(span.lastHour + 1) ?? 0) - 1);
      }
      const unique: Span[] = [];
      let count = 0, start = 0;
      for (const [hour, delta] of [...events].sort((a, b) => a[0] - b[0])) {
        if (count === 1 && start < hour) unique.push({ firstHour: start, lastHour: hour - 1 });
        count += delta; start = hour;
      }
      return bands.map(runs => shownRuns(runs.sort((a, b) => a.firstHour - b.firstHour), unique, range));
    },
  };
  return { data, tagAt };
}

export function exactPartShare(value: bigint | null, total: number | null): string {
  if (value == null || total == null || total === 0) return '—';
  const denominator = BigInt(total);
  const numerator = value * 1000n;
  const negative = (numerator < 0n) !== (denominator < 0n);
  const raw = numerator < 0n ? -numerator : numerator;
  const divisor = denominator < 0n ? -denominator : denominator;
  const tenths = (raw + (negative ? (divisor - 1n) / 2n : divisor / 2n)) / divisor;
  return `${negative && tenths !== 0n ? '-' : ''}${tenths / 10n}${tenths % 10n ? `.${tenths % 10n}` : ''}%`;
}

export function pinCountryParts(source: WidgetSpec, track: Pick<BoundTrack, 'id' | 'branch'>, held: HeldCountry, time: GatedTime, label: string): PinnedSpec {
  const pinned = pinPartsRace(source, track, held.tag, time, label);
  return { ...pinned, options: { ...pinned.options, drillOriginHour: held.origin } };
}

export function countryPartsRemainder(at: CountryPartsAt | null): bigint | null {
  return at?.total == null ? null : BigInt(at.total) - at.values.reduce<bigint>((sum, value) => sum + BigInt(value ?? 0), 0n);
}
export const exactPartAmount = (value: number | bigint) => usWhole.format(value);
