import { dayLabel } from '../../charts/draw.ts';
import { liveKey } from '../../charts/live.ts';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import { lookAt, type CountryLook } from '../../draw/tables/looks.ts';
import { OFFERED } from '../../draw/tables/settings.ts';
import type { GameScoreRow } from '../../generated/dashboard';
import {
  declareTableProjection,
  type TableColumnSpec,
  type TableGroupSpec,
  type TableProjection,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { knownValue, notRecorded, valueFromLive, valueNumber, type Value } from '../../families/value.ts';
import type { HourRanges } from '../occupied-states/occupiedProjection.ts';
import { SCORE_LABELS, SCORE_PARTS, scoreHint, scorePart, validateGameScore, type GameScoreReply, type ScorePart } from './gameScoreData.ts';

export interface FactionRun {
  readonly id: string;
  readonly tag: string;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly names: readonly { readonly firstHour: number; readonly lastHour: number; readonly name: string | null }[];
  readonly fallbackName: string | null;
}

export interface GameScoreProjectionOptions {
  readonly all: boolean;
  readonly chosenTags: readonly string[];
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly top: number | null;
  readonly by: ScorePart;
  readonly factions: boolean;
  readonly factionRuns: readonly FactionRun[] | null;
  readonly rankMarkers: boolean;
  readonly cellBars: boolean;
  readonly labels: Readonly<Record<string, readonly CountryLook[]>>;
}

const validated = new WeakMap<object, GameScoreReply>();

function checkedScore(raw: unknown): GameScoreReply {
  if (!raw || typeof raw !== 'object') return validateGameScore(raw);
  let held = validated.get(raw);
  if (!held) {
    held = validateGameScore(raw);
    validated.set(raw, held);
  }
  return held;
}

const inRanges = (ranges: HourRanges | undefined, hour: number) => !!ranges?.some(([first, last]) => first <= hour && hour <= last);

function factionAt(runs: readonly FactionRun[] | null, tag: string, hour: number): TableGroupSpec {
  if (!runs) return { key: 'faction-not-recorded', label: 'Faction not known yet', order: 2 };
  for (const run of runs) {
    if (run.tag === tag && run.firstHour <= hour && hour <= run.lastHour) {
      const name = run.names.find(n => n.firstHour <= hour && hour <= n.lastHour)?.name ?? run.fallbackName;
      return { key: `faction:${run.id}`, label: name ?? 'Name not recorded', order: name === null ? 1 : 0, orderKey: BigInt(run.id) };
    }
  }
  return { key: 'no-faction', label: 'No faction', order: 3 };
}

export function gameScoreColumns(hints?: GameScoreReply['hints'], cellBars = false): TableColumnSpec[] {
  const numDisplay = cellBars ? 'faint' : 'plain';
  return [
    {
      key: 'country',
      label: SCORE_LABELS.country,
      as: 'flagName',
      offered: OFFERED.country,
      priority: 0,
      text: true,
      hideable: false,
      widest: ['United Kingdom'],
      hint: scoreHint(hints ? { hints } : undefined, 'country'),
    },
    ...SCORE_PARTS.map((part, index): TableColumnSpec => ({
      key: part,
      label: SCORE_LABELS[part],
      as: numDisplay,
      tone: 'blue',
      offered: OFFERED.number,
      priority: index + 1,
      numeric: true,
      format: { decimals: 0 },
      widest: ['99,999'],
      hint: scoreHint(hints ? { hints } : undefined, part),
    })),
  ];
}

export const gameScoreProjection: TableProjection<GameScoreProjectionOptions> = registerTableProjection(
  declareTableProjection<unknown, GameScoreRow, GameScoreProjectionOptions>({
    kind: 'game-score',
    rank: true,
    rankMarkers: options => options.rankMarkers,
    columns: (raw, options) => {
      const reply = checkedScore(raw);
      const specs = gameScoreColumns(reply.hints, options.cellBars);
      return [
        {
          ...specs[0]!,
          iconSlot: true,
          country: row => row.tag,
          looks: (row, opt) => opt.labels[row.tag],
          value: (row, _live, hour, opt) => {
            const look = lookAt(opt.labels[row.tag], hour);
            return look && look.name ? knownValue(look.name) : notRecorded(look?.status ?? 'Country name unavailable');
          },
        },
        ...SCORE_PARTS.map((part, idx) => ({
          ...specs[idx + 1]!,
          country: (row: GameScoreRow) => row.tag,
          value: (row: GameScoreRow, live: Parameters<typeof valueFromLive>[0]): Value => {
            if (row[part].missing !== null) return notRecorded(row[part].missing);
            return valueFromLive(live, liveKey(row.tag, part));
          },
        })),
      ];
    },
    rows: (raw, live, hour, options) => {
      const reply = checkedScore(raw);
      if (reply.missing !== null) {
        return { rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
      }
      let filtered: GameScoreRow[];
      if (options.top != null) {
        const scored: { row: GameScoreRow; val: number }[] = [];
        for (const row of reply.rows) {
          if (row[options.by].missing !== null) continue;
          const v = valueNumber(valueFromLive(live, liveKey(row.tag, options.by)));
          if (v !== undefined) scored.push({ row, val: v });
        }
        scored.sort((a, b) => b.val - a.val || (a.row.tag < b.row.tag ? -1 : a.row.tag > b.row.tag ? 1 : 0));
        filtered = scored.slice(0, options.top).map(item => item.row);
      } else if (options.all) {
        filtered = reply.rows;
      } else if (options.membership) {
        filtered = reply.rows.filter(row => inRanges(options.membership![row.tag], hour));
      } else {
        const chosen = new Set(options.chosenTags);
        filtered = reply.rows.filter(row => chosen.has(row.tag));
      }
      if (!filtered.length) {
        return { rows: [], empty: `No country’s scores recorded on ${dayLabel(hour)}.`, emptyKind: 'unavailable', warnings: reply.warnings };
      }
      return { rows: filtered, warnings: reply.warnings };
    },
    rowKey: row => row.tag,
    rowName: (row, hour, options) => lookAt(options.labels[row.tag], hour)?.name ?? row.tag,
    group: (row, hour, options) => (options.factions ? factionAt(options.factionRuns, row.tag, hour) : undefined),
    tooltipExtra: (row, raw, _live, hour, options): TooltipExtraRow[] => {
      const reply = checkedScore(raw);
      const faction = factionAt(options.factionRuns, row.tag, hour).label;
      return [
        { label: 'Faction', value: knownValue(faction), divider: true },
        ...reply.multipliers.map((multiplier, index): TooltipExtraRow => ({
          label: `${SCORE_LABELS[scorePart(multiplier.part)]} multiplier`,
          value: multiplier.raw === null ? notRecorded(multiplier.missing ?? 'Not recorded') : knownValue(formatRaw(multiplier.raw, 5, 5)),
          ...(index === 0 ? { divider: true } : {}),
        })),
      ];
    },
  }),
);
