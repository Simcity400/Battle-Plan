import type { ChartBucket } from '../charts/chartSummary';
import type { Run } from '../charts/series';
import { countryKey } from '../dashboard/countryKey.ts';
import { rowKey, rowName, type FollowedAnswer } from '../dashboard/follow/message.ts';
import { rowAmount } from '../dashboard/follow/painters.ts';
import { followedRow, keyId } from '../dashboard/keyedSeriesData.ts';
import type { TiledLine } from '../dashboard/tiles/view';
import type { Scene } from '../draw/lane.ts';
import type { MissingSpan, TilePart } from '../generated/dashboard';
import { BoundedSeriesCursor, type WitnessedPoint } from './boundedView.ts';
import type { BoundedReadView, BoundedSeries } from './contract.ts';
import { sceneReadView, type SceneSources } from './scene.ts';
import { knownValue, valueFromWireCell, type Value } from './value.ts';

export interface SerializedTiledLine {
  readonly source: string;
  readonly tag: string;
  readonly head: string;
  readonly decimals: number;
  readonly binHours: number;
  readonly from: number;
  readonly to: number;
  readonly width: number;
  readonly buckets: readonly (ChartBucket | null)[];
  readonly runs?: readonly Run[];
  readonly missing: readonly MissingSpan[];
  readonly parts: readonly TilePart[];
}

export interface SeriesHeadSource {
  readonly answerName: string;
  readonly tags: readonly string[];
  readonly rankedSlots?: boolean;
  readonly keys?: readonly { readonly tag: string; readonly part: string }[];
  readonly partBits?: number;
  readonly decimals?: number;
}

export interface TiledSources {
  readonly tiledBySource?: Readonly<Record<string, Readonly<Record<string, SerializedTiledLine>>>>;
  readonly headSourceBySeries?: Readonly<Record<string, SeriesHeadSource>>;
}

const serializedLines = new WeakMap<TiledLine, SerializedTiledLine>();

function serializeTiledLine(source: string, line: TiledLine): SerializedTiledLine {
  const cached = serializedLines.get(line);
  if (cached && cached.source === source) return cached;
  const serialized: SerializedTiledLine = {
    source,
    tag: line.tag,
    head: line.head,
    decimals: line.decimals,
    binHours: line.summary.tiles?.binHours ?? 1,
    from: line.summary.from,
    to: line.summary.to,
    width: line.summary.width,
    buckets: line.summary.buckets,
    ...(line.summary.runs ? { runs: line.summary.runs } : {}),
    missing: line.missing,
    parts: line.parts,
  };
  serializedLines.set(line, serialized);
  return serialized;
}

export function serializeTiledBySource(
  tiledSeries: Readonly<Record<string, ReadonlyMap<string, TiledLine>>>,
): Record<string, Record<string, SerializedTiledLine>> {
  const tiledBySource: Record<string, Record<string, SerializedTiledLine>> = {};
  for (const [sourceName, lines] of Object.entries(tiledSeries)) {
    const byTag: Record<string, SerializedTiledLine> = {};
    for (const [tag, line] of lines) byTag[tag] = serializeTiledLine(sourceName, line);
    tiledBySource[sourceName] = byTag;
  }
  return tiledBySource;
}

const hydrated = new WeakMap<SerializedTiledLine, TiledLine>();

function hydrateTiledLine(raw: SerializedTiledLine): TiledLine {
  let line = hydrated.get(raw);
  if (!line) hydrated.set(raw, line = {
    tag: raw.tag,
    head: raw.head,
    decimals: raw.decimals,
    summary: {
      from: raw.from,
      to: raw.to,
      width: raw.width,
      buckets: [...raw.buckets],
      ...(raw.runs ? { runs: raw.runs } : {}),
      tiles: { binHours: raw.binHours, valueAt: () => null },
    },
    missing: [...raw.missing],
    parts: [...raw.parts],
    marks: null,
  });
  return line;
}

const rowIndexes = new WeakMap<readonly string[], ReadonlyMap<string, number>>();

function rowIndexOf(tags: readonly string[]): ReadonlyMap<string, number> {
  let index = rowIndexes.get(tags);
  if (!index) rowIndexes.set(tags, index = new Map(tags.map((tag, row) => [tag, row])));
  return index;
}

const answerRows = new WeakMap<FollowedAnswer, ReadonlyMap<string, number>>();
const slotRows = new WeakMap<FollowedAnswer, ReadonlyMap<string, number>>();

export function answerRowOf(answer: FollowedAnswer | undefined, tag: string, rankedSlots = false): number {
  if (!answer || answer.kind === 'notRecorded') return -1;
  const cache = rankedSlots ? slotRows : answerRows;
  let rows = cache.get(answer);
  if (!rows) {
    const named = new Map<string, number>();
    for (let row = 0; row < answer.rows; row++) {
      const key = rankedSlots ? countryKey(Number(rowKey(answer, row))) : rowName(answer, row);
      if (key && !named.has(key)) named.set(key, row);
    }
    cache.set(answer, rows = named);
  }
  return rows.get(tag) ?? -1;
}

const keyIndexes = new WeakMap<readonly { readonly tag: string; readonly part: string }[], ReadonlyMap<string, { readonly tag: string; readonly part: string }>>();

function keyOf(keys: readonly { readonly tag: string; readonly part: string }[], tag: string): { readonly tag: string; readonly part: string } | undefined {
  let index = keyIndexes.get(keys);
  if (!index) keyIndexes.set(keys, index = new Map(keys.map(key => [keyId(key), key])));
  return index.get(tag);
}

function headRow(answer: FollowedAnswer, head: SeriesHeadSource, tag: string): number {
  if (head.keys && head.partBits !== undefined) {
    const matched = keyOf(head.keys, tag);
    return matched ? followedRow(answer, matched, head.partBits) : -1;
  }
  return head.tags.length ? rowIndexOf(head.tags).get(tag) ?? -1 : answerRowOf(answer, tag, head.rankedSlots);
}

export class SceneTiles {
  private readonly cursors = new Map<string, BoundedSeriesCursor>();

  series(scene: Scene, sources: SceneSources, binding: TiledSources, hour: number): ReadonlyMap<string, ReadonlyMap<string, BoundedSeries>> | undefined {
    if (!binding.tiledBySource) {
      this.cursors.clear();
      return undefined;
    }
    const active = new Set<string>();
    let answers: BoundedReadView | null = null;
    const bySource = new Map<string, ReadonlyMap<string, BoundedSeries>>();
    for (const [source, lines] of Object.entries(binding.tiledBySource)) {
      const head = binding.headSourceBySeries?.[source];
      const answer = head ? (answers ??= sceneReadView(scene, sources, hour)).answer(head.answerName) : undefined;
      const byTag = new Map<string, BoundedSeries>();
      for (const [tag, raw] of Object.entries(lines)) {
        const line = hydrateTiledLine(raw);
        const key = `${source}\u0000${tag}`;
        active.add(key);
        let cursor = this.cursors.get(key);
        if (!cursor) this.cursors.set(key, cursor = new BoundedSeriesCursor());
        let headPoint: WitnessedPoint | null = null;
        let headValue: Value | undefined;
        if (answer && head) {
          const row = headRow(answer, head, tag);
          const value = row >= 0 ? rowAmount(answer, row, head.decimals ?? line.decimals) : null;
          if (value !== undefined && value !== null && Number.isFinite(value)) {
            headPoint = { hour: Math.floor(hour), value };
            headValue = knownValue(value);
          } else {
            headValue = valueFromWireCell(answer, row, 0);
          }
        }
        byTag.set(tag, cursor.advance(line, hour, { headPoint, headValue }));
      }
      bySource.set(source, byTag);
    }
    for (const key of this.cursors.keys()) if (!active.has(key)) this.cursors.delete(key);
    return bySource;
  }
}
