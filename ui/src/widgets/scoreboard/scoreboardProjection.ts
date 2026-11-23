import { countryKey } from '../../dashboard/countryKey.ts';
import type { CountryLook } from '../../draw/tables/looks.ts';
import { lookAt } from '../../draw/tables/looks.ts';
import { OFFERED } from '../../draw/tables/settings.ts';
import type { ScoreboardColumn, ScoreboardReply } from '../../generated/dashboard';
import type { FamilySources } from '../../families/contract.ts';
import {
  knownValue,
  notRecorded,
  partialValue,
  valueFromNamedAnswer,
  valueFromRawBigInt,
  type NumericScalar,
  type Value,
} from '../../families/value.ts';
import type { HourRanges } from '../occupied-states/occupiedProjection.ts';
import { defaultWidestSamples, type ValueFormat } from '../../families/valueFormat.ts';
import type { TableCell, TableColumnSpec, TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { partialSource, rankedTags, scoreboardColumnNote, validateScoreboardReply } from './scoreboardData.ts';

export const RANK_SOURCE = 'top:rank';

export interface ScoreboardProjectionOptions {
  readonly shown: readonly ScoreboardColumn[];
  readonly candidates: readonly string[];
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly top: number | null;
  readonly rankMarkers: boolean;
  readonly looks: Readonly<Record<string, readonly CountryLook[]>>;
}

function columnFormat(column: ScoreboardColumn): ValueFormat {
  return { decimals: column.decimals, percent: column.unit === 'percent' };
}

export function scoreboardColumnsSpec(shown: readonly ScoreboardColumn[], top: number | null): TableColumnSpec[] {
  return [
    {
      key: 'country',
      label: 'Country',
      as: 'flagName',
      offered: OFFERED.country,
      priority: 0,
      text: true,
      hideable: false,
      widest: ['United Kingdom'],
      hint: top != null ? 'Top countries, sorted by name.' : undefined,
    },
    ...shown.map((column, index): TableColumnSpec => {
      const format = columnFormat(column);
      return {
        key: column.key,
        label: column.label,
        as: 'faint',
        tone: 'blue',
        offered: OFFERED.number,
        priority: index + 1,
        numeric: true,
        format,
        widest: defaultWidestSamples(format),
        hideable: shown.length > 1,
        hint: scoreboardColumnNote(column),
      };
    }),
  ];
}

function countryNameValue(tag: string, looks: Readonly<Record<string, readonly CountryLook[]>>, hour: number): { value: Value<string>; name: string; runs?: readonly CountryLook[] } {
  const runs = looks[tag];
  const look = lookAt(runs, hour);
  if (look && look.name) return { value: knownValue(look.name), name: look.name, runs };
  return { value: notRecorded(look?.status ?? 'Country name unavailable'), name: tag, runs };
}

export const scoreboardProjection: TableProjection<ScoreboardProjectionOptions> = registerTableProjection({
  kind: 'scoreboard',
  family: 'table',
  nextChange(_sources, hour, options) {
    if (!options.membership) return Infinity;
    let soonest = Infinity;
    for (const ranges of Object.values(options.membership)) {
      for (const [first, last] of ranges) {
        if (first > hour) soonest = Math.min(soonest, first);
        if (last + 1 > hour) soonest = Math.min(soonest, last + 1);
      }
    }
    return soonest;
  },
  project(sources: FamilySources, hour: number, options: ScoreboardProjectionOptions): TableModel {
    const columns = scoreboardColumnsSpec(options.shown, options.top);
    const panel = sources.panels.get('main');

    if (panel !== undefined) {
      const reply: ScoreboardReply = validateScoreboardReply(panel, options.shown.length);
      if (reply.missing !== null) {
        return { columns, rows: [], hour: reply.hour, empty: reply.missing, emptyKind: 'unavailable', rank: true, rankMarkers: options.rankMarkers, warnings: reply.warnings };
      }
      const rows: TableRow[] = reply.rows.map(row => {
        const key = countryKey(row.country);
        const identity = countryNameValue(row.tag, options.looks, reply.hour);
        const cells: Record<string, TableCell> = {
          country: { value: identity.value, iconSlot: true, country: row.tag, ...(identity.runs ? { looks: identity.runs } : {}) },
        };
        let hasPartial = false;
        options.shown.forEach((column, at) => {
          const rawCell = row.cells[at]!;
          const format = columnFormat(column);
          const value: Value<bigint> = !column.available
            ? notRecorded(column.reason ?? 'Unavailable')
            : valueFromRawBigInt(
              rawCell.valueRaw,
              { kind: 'notRecorded', reason: rawCell.missing ?? column.reason ?? 'Not available at this time' },
              { partial: rawCell.partial ? 'Some inputs were not recorded at this hour' : null },
            );
          if (value.kind === 'partial') hasPartial = true;
          cells[column.key] = { value, format, country: row.tag };
        });
        return {
          key,
          name: identity.name,
          cells,
          ...(hasPartial ? { tooltipNote: 'Partial values omit unrecorded inputs at this hour.' } : {}),
        };
      });
      return {
        columns,
        rows,
        hour: reply.hour,
        rank: true,
        rankMarkers: options.rankMarkers,
        warnings: reply.warnings,
        ...(!rows.length ? { empty: 'No countries available for this selection.', emptyKind: 'unavailable' } : {}),
      };
    }

    const rankAnswer = options.top != null ? sources.answers.get(RANK_SOURCE) : undefined;
    if (options.top != null) {
      if (!rankAnswer) {
        return { columns, rows: [], hour, rank: true, rankMarkers: options.rankMarkers };
      }
      if (rankAnswer.kind === 'notRecorded') {
        return { columns, rows: [], hour, empty: rankAnswer.reason, emptyKind: 'unavailable', rank: true, rankMarkers: options.rankMarkers };
      }
    }

    const memberTags = options.top != null && rankAnswer && rankAnswer.kind !== 'notRecorded'
      ? rankedTags(rankAnswer)
      : options.membership
        ? options.candidates.filter(tag => options.membership![tag]?.some(([first, last]) => first <= hour && hour <= last))
        : options.candidates;

    const rows: TableRow[] = memberTags.map(tag => {
      const identity = countryNameValue(tag, options.looks, hour);
      const cells: Record<string, TableCell> = {
        country: { value: identity.value, iconSlot: true, country: tag, ...(identity.runs ? { looks: identity.runs } : {}) },
      };
      let hasPartial = false;
      for (const column of options.shown) {
        const format = columnFormat(column);
        if (!column.available || !column.family) {
          cells[column.key] = { value: notRecorded(column.reason ?? 'Unavailable'), format, country: tag };
          continue;
        }
        const answer = sources.answers.get(column.key);
        const partialAns = column.partial ? sources.answers.get(partialSource(column.key)) : undefined;
        let isPartialCell = false;
        if (partialAns) {
          const mark = valueFromNamedAnswer(partialAns, tag);
          if (mark.kind === 'known' && mark.value !== 0 && mark.value !== 0n) isPartialCell = true;
        }
        const base = valueFromNamedAnswer(answer, tag);
        let value: Value<NumericScalar> = base;
        if (isPartialCell && (base.kind === 'known' || base.kind === 'zero')) {
          value = partialValue(base.value, 'Some inputs were not recorded at this hour');
          hasPartial = true;
        }
        cells[column.key] = { value, format, country: tag };
      }
      return {
        key: tag,
        name: identity.name,
        cells,
        ...(hasPartial ? { tooltipNote: 'Partial values omit unrecorded inputs at this hour.' } : {}),
      };
    });

    return {
      columns,
      rows,
      hour,
      rank: true,
      rankMarkers: options.rankMarkers,
      ...(!rows.length ? { empty: 'No countries available for this selection.', emptyKind: 'unavailable' } : {}),
    };
  },
});
