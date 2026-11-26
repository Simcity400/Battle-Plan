import type { ChartBucket } from '../charts/chartSummary';
import type { Run } from '../charts/series';
import { countryKey } from '../dashboard/countryKey.ts';
import { rowKey, rowName, rowReason, type FollowedAnswer, type FollowedRows } from '../dashboard/follow/message.ts';
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
  readonly partial?: readonly MissingSpan[];
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
    ...(line.partial?.length ? { partial: line.partial } : {}),
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
    ...(raw.partial?.length ? { partial: [...raw.partial] } : {}),
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
interface CountryRank {
  readonly rows: ReadonlyMap<string, number>;
  readonly tags: ReadonlyMap<string, string>;
  readonly unnamed: ReadonlyMap<string, string>;
}

const followedRanks = new WeakMap<FollowedRows, CountryRank>();

export function followedCountryRank(answer: FollowedRows): CountryRank {
  const held = followedRanks.get(answer);
  if (held) return held;
  const rows = new Map<string, number>();
  const tags = new Map<string, string>();
  const unnamed = new Map<string, string>();
  for (let row = 0; row < answer.rows; row++) {
    const key = countryKey(Number(rowKey(answer, row)));
    rows.set(key, row);
    const reason = rowReason(answer, row);
    const tag = rowName(answer, row);
    if (reason) unnamed.set(key, reason);
    else if (tag !== null) tags.set(key, tag);
  }
  const rank = { rows, tags, unnamed };
  followedRanks.set(answer, rank);
  return rank;
}

export function answerRowOf(answer: FollowedAnswer | undefined, tag: string): number {
  if (!answer || answer.kind === 'notRecorded') return -1;
  let rows = answerRows.get(answer);
  if (!rows) {
    const named = new Map<string, number>();
    for (let row = 0; row < answer.rows; row++) {
      const key = rowName(answer, row);
      if (key && !named.has(key)) named.set(key, row);
    }
    answerRows.set(answer, rows = named);
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
  if (head.rankedSlots && answer.kind !== 'notRecorded') return followedCountryRank(answer).rows.get(tag) ?? -1;
  return head.tags.length ? rowIndexOf(head.tags).get(tag) ?? -1 : answerRowOf(answer, tag);
}

interface CursorSlot {
  readonly cursor: BoundedSeriesCursor;
  gen: number;
}

export class SceneTiles {
  private readonly cursors = new Map<string, Map<string, CursorSlot>>();
  private gen = 0;
  private lastBySource: Map<string, ReadonlyMap<string, BoundedSeries>> | null = null;

  series(scene: Scene, sources: SceneSources, binding: TiledSources, hour: number): ReadonlyMap<string, ReadonlyMap<string, BoundedSeries>> | undefined {
    const tiledBySource = binding.tiledBySource;
    if (!tiledBySource) {
      this.cursors.clear();
      this.lastBySource = null;
      return undefined;
    }
    const gen = ++this.gen;
    let answers: BoundedReadView | null = null;
    let allSourcesSame = this.lastBySource !== null;
    let sourceCount = 0;
    const nextBySource = new Map<string, ReadonlyMap<string, BoundedSeries>>();
    for (const source in tiledBySource) {
      if (!Object.hasOwn(tiledBySource, source)) continue;
      sourceCount++;
      const lines = tiledBySource[source]!;
      let sourceSlots = this.cursors.get(source);
      if (!sourceSlots) this.cursors.set(source, sourceSlots = new Map());
      const head = binding.headSourceBySeries?.[source];
      const answer = head ? (answers ??= sceneReadView(scene, sources, hour)).answer(head.answerName) : undefined;
      const prevByTag = this.lastBySource?.get(source);
      let allTagsSame = prevByTag !== undefined;
      let tagCount = 0;
      let byTag: Map<string, BoundedSeries> | null = null;
      const floorHour = Math.floor(hour);
      for (const tag in lines) {
        if (!Object.hasOwn(lines, tag)) continue;
        tagCount++;
        const raw = lines[tag]!;
        const line = hydrateTiledLine(raw);
        let slot = sourceSlots.get(tag);
        if (!slot) sourceSlots.set(tag, slot = { cursor: new BoundedSeriesCursor(), gen });
        else slot.gen = gen;
        let headPoint: WitnessedPoint | null = null;
        let headValue: Value | undefined;
        if (answer && head) {
          const row = headRow(answer, head, tag);
          const value = row >= 0 ? rowAmount(answer, row, head.decimals ?? line.decimals) : null;
          if (value !== undefined && value !== null && Number.isFinite(value)) {
            headPoint = { hour: floorHour, value };
            headValue = knownValue(value);
          } else {
            headValue = valueFromWireCell(answer, row, 0);
          }
        }
        const bounded = slot.cursor.advance(line, hour, { headPoint, headValue });
        if (allTagsSame && prevByTag!.get(tag) === bounded) continue;
        if (allTagsSame) {
          allTagsSame = false;
          byTag = new Map();
          for (const prevTag in lines) {
            if (prevTag === tag) break;
            if (!Object.hasOwn(lines, prevTag)) continue;
            byTag.set(prevTag, prevByTag!.get(prevTag)!);
          }
        }
        (byTag ??= new Map()).set(tag, bounded);
      }
      if (allTagsSame && prevByTag!.size !== tagCount) {
        allTagsSame = false;
        byTag = new Map();
        for (const tag in lines) {
          if (!Object.hasOwn(lines, tag)) continue;
          byTag.set(tag, prevByTag!.get(tag)!);
        }
      }
      const resolvedByTag: ReadonlyMap<string, BoundedSeries> = allTagsSame ? prevByTag! : (byTag ?? new Map());
      if (!allTagsSame) allSourcesSame = false;
      nextBySource.set(source, resolvedByTag);
      for (const [tag, slot] of sourceSlots) {
        if (slot.gen !== gen) sourceSlots.delete(tag);
      }
      if (sourceSlots.size === 0) this.cursors.delete(source);
    }
    for (const [source, sourceSlots] of this.cursors) {
      if (!Object.hasOwn(tiledBySource, source)) {
        this.cursors.delete(source);
      } else if (sourceSlots.size === 0) {
        this.cursors.delete(source);
      }
    }
    if (allSourcesSame && this.lastBySource!.size === sourceCount) return this.lastBySource!;
    this.lastBySource = nextBySource;
    return nextBySource;
  }
}
