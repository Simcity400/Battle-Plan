import type { TiledLine, TileView } from '../../dashboard/tiles/view.ts';
import { boundedUnrecorded, type UnrecordedLine } from '../boundedView.ts';

export type TiledSeriesMap = Readonly<Record<string, ReadonlyMap<string, TiledLine>>>;

export const NO_TILED_SERIES: TiledSeriesMap = Object.freeze({});

export function reuseTiledSeries(
  previous: TiledSeriesMap,
  views: ReadonlyMap<string, Pick<TileView, 'current'>>,
): TiledSeriesMap {
  let count = 0;
  let same = true;
  for (const [name, tv] of views) {
    if (!tv.current) continue;
    count++;
    if (previous[name] !== tv.current) same = false;
  }
  if (same && count === Object.keys(previous).length) return previous;
  if (count === 0) return Object.keys(previous).length === 0 ? previous : NO_TILED_SERIES;
  const next: Record<string, ReadonlyMap<string, TiledLine>> = {};
  for (const [name, tv] of views) if (tv.current) next[name] = tv.current;
  return next;
}

export interface HeldTileUnrecorded {
  readonly series: TiledSeriesMap;
  readonly result: ReturnType<typeof boundedUnrecorded> | null;
}

export function reuseTileUnrecorded(
  previous: HeldTileUnrecorded | null,
  series: TiledSeriesMap | undefined,
  hour: number,
): HeldTileUnrecorded | null {
  if (!series) return null;
  if (
    previous
    && previous.series === series
    && (previous.result === null || (hour >= previous.result.validInterval[0] && hour <= previous.result.validInterval[1]))
  ) {
    return previous;
  }
  const lines: UnrecordedLine[] = [];
  for (const entry of Object.values(series)) for (const line of entry.values()) lines.push(line);
  return { series, result: lines.length ? boundedUnrecorded(lines, hour) : null };
}

export function topTileProof(
  tops: readonly Pick<TileView, 'top'>[],
  at: number,
  trackOffset: number,
  gateRecorded: (hour: number) => number | null,
  unrecordedSpan: (hour: number) => readonly [number, number],
): readonly [number, number] {
  if (gateRecorded(at) === null) {
    const [unrecordedLo, unrecordedHi] = unrecordedSpan(at);
    return [unrecordedLo - trackOffset, unrecordedHi + 1 - trackOffset];
  }
  let lo = -Infinity;
  let hi = Infinity;
  for (const tv of tops) {
    const held = tv.top;
    lo = Math.max(lo, held ? held.from : at);
    hi = Math.min(hi, held ? held.until : at);
  }
  return [lo - trackOffset, hi + 1 - trackOffset];
}
