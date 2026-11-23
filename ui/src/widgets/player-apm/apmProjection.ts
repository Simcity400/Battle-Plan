import { rowName, type FollowedAnswer } from '../../dashboard/follow/message.ts';
import { lookAt, type CountryLook } from '../../draw/tables/looks.ts';
import { OFFERED } from '../../draw/tables/settings.ts';
import type { ApmReply, ApmSeries, ApmValue } from '../../generated/dashboard';
import type { FamilySources } from '../../families/contract.ts';
import { knownValue, notApplicable, notRecorded, partialValue, valueFromNullable, valueFromRawBigInt, wireFloat, wireWhole, type Value } from '../../families/value.ts';
import type { TableCell, TableColumnSpec, TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { GAPS_NOTE, validateApm, valueNote } from './apmData.ts';

export interface ApmTableProjectionOptions {
  readonly players: boolean;
  readonly showPaused: boolean;
  readonly cellBars: boolean;
  readonly rankMarkers: boolean;
  readonly playerNames: Readonly<Record<string, string>>;
  readonly labels: Readonly<Record<string, readonly CountryLook[]>>;
}

const APM = { apm: 0, confirmedApm: 1, orders: 2, pausedOrders: 3, minutes: 4, complete: 5, gaps: 6, tag: 7, hour: 8 } as const;

function unpackTag(raw: bigint | null): string | null {
  if (raw === null) return null;
  let out = '';
  for (let v = BigInt.asUintN(64, raw); v > 0n; v >>= 8n) out += String.fromCharCode(Number(v & 0xffn));
  return out || null;
}

function apmReason(summary: ApmValue): { readonly kind: 'notRecorded' | 'notApplicable'; readonly reason: string } {
  if (!summary.complete) return { kind: 'notRecorded', reason: 'Player orders or unpaused time were not fully recorded for this period; the rate is unknown.' };
  if (summary.unpausedMinutes === 0) return { kind: 'notApplicable', reason: 'No unpaused time was observed in this period.' };
  return { kind: 'notRecorded', reason: 'It is unclear whether some orders count toward eAPM; the rate is unknown.' };
}

function rateValue(summary: ApmValue): Value<number> {
  if (summary.apm === null) {
    const why = apmReason(summary);
    return why.kind === 'notApplicable' ? notApplicable(why.reason) : notRecorded(why.reason);
  }
  return summary.gaps ? partialValue(summary.apm, GAPS_NOTE) : knownValue(summary.apm);
}

function seriesFromAnswer(answer: FollowedAnswer): { readonly rows: ApmSeries[]; readonly refusal?: string } {
  if (answer.kind === 'notRecorded') return { rows: [], refusal: answer.reason };
  const rows: ApmSeries[] = [];
  for (let row = 0; row < answer.rows; row++) {
    const at = row * answer.columns;
    const id = rowName(answer, row) ?? String(row);
    const readFloat = (col: number): number | null => {
      const v = wireFloat(answer, at + col);
      return v.kind === 'known' || v.kind === 'zero' ? Number(v.value) : null;
    };
    const readBig = (col: number): bigint | null => {
      const v = wireWhole(answer, at + col);
      return v.kind === 'known' || v.kind === 'zero' ? BigInt(v.value) : null;
    };
    const tag = unpackTag(readBig(APM.tag));
    const rowHour = Number(readBig(APM.hour) ?? 0n);
    const summary: ApmValue = {
      apm: readFloat(APM.apm),
      confirmedApm: readFloat(APM.confirmedApm),
      orders: readBig(APM.orders)?.toString() ?? null,
      pausedOrders: readBig(APM.pausedOrders)?.toString() ?? null,
      unpausedMinutes: readFloat(APM.minutes),
      complete: readBig(APM.complete) === 1n,
      gaps: readBig(APM.gaps) === 1n,
    };
    rows.push({ id, name: id, tag, hour: rowHour, summary, points: [] });
  }
  return { rows };
}

export function apmTableColumns(players: boolean, showPaused: boolean, cellBars: boolean): TableColumnSpec[] {
  return [
    {
      key: 'name',
      label: players ? 'Player' : 'Country',
      as: players ? 'plain' : 'flagName',
      offered: players ? OFFERED.text : OFFERED.country,
      priority: 0,
      text: true,
      hideable: false,
      widest: ['United Kingdom'],
    },
    ...(players ? [{
      key: 'country',
      label: 'Country',
      as: 'flagName' as const,
      offered: OFFERED.country,
      priority: 2,
      text: true,
      widest: ['United Kingdom'],
    }] : []),
    {
      key: 'apm',
      label: 'eAPM',
      as: cellBars ? 'faint' : 'plain',
      tone: 'blue',
      offered: OFFERED.number,
      priority: 1,
      numeric: true,
      format: 'decimal1',
      widest: ['999.9'],
      hint: 'Orders per unpaused real minute',
    },
    {
      key: 'orders',
      label: 'Orders',
      as: 'plain',
      offered: OFFERED.number,
      priority: 1,
      numeric: true,
      format: 'integer',
      widest: ['999,999'],
      hint: 'Player-issued gameplay orders during unpaused play',
    },
    ...(showPaused ? [{
      key: 'paused',
      label: 'Paused orders',
      as: 'plain' as const,
      offered: OFFERED.number,
      priority: 2,
      numeric: true,
      format: 'integer' as const,
      widest: ['99,999'],
    }] : []),
    {
      key: 'minutes',
      label: 'Unpaused minutes',
      as: 'plain',
      offered: OFFERED.number,
      priority: 3,
      numeric: true,
      format: 'decimal1',
      widest: ['9,999.9'],
    },
  ];
}

export const apmTableProjection: TableProjection<ApmTableProjectionOptions> = registerTableProjection({
  kind: 'apm-table',
  family: 'table',
  project(sources: FamilySources, hour: number, options: ApmTableProjectionOptions): TableModel {
    const columns = apmTableColumns(options.players, options.showPaused, options.cellBars);
    const answer = sources.answers.get('main');
    const panel = sources.panels.get('main');

    let series: readonly ApmSeries[] | null = null;
    let refusal: string | null = null;
    let warnings: readonly string[] = [];

    if (answer !== undefined) {
      const parsed = seriesFromAnswer(answer);
      if (parsed.refusal) refusal = parsed.refusal;
      else series = parsed.rows;
    } else if (panel !== undefined) {
      const reply: ApmReply = validateApm(panel);
      warnings = reply.warnings;
      if (!reply.available) refusal = 'Player orders were not recorded in this campaign.';
      else series = reply.series;
    }

    if (refusal !== null) {
      return { columns, rows: [], hour, empty: refusal, emptyKind: 'unavailable', rank: true, rankMarkers: options.rankMarkers, warnings };
    }
    if (series === null) {
      return { columns, rows: [], hour, rank: true, rankMarkers: options.rankMarkers };
    }
    if (!series.length) {
      return { columns, rows: [], hour, empty: 'No eAPM is available for these countries, players and dates.', emptyKind: 'empty', rank: true, rankMarkers: options.rankMarkers, warnings };
    }

    const rows: TableRow[] = series.map(entry => {
      const tag = entry.tag;
      const runs = tag ? options.labels[tag] : undefined;
      const look = lookAt(runs, entry.hour || hour);
      const countryVal = tag === null
        ? notRecorded('Unknown country')
        : (look && look.name ? knownValue(look.name) : notRecorded(look?.status ?? 'Country name unavailable'));
      const playerName = options.playerNames[entry.id] ?? entry.name;
      const nameStr = options.players ? playerName : (look?.name ?? tag ?? entry.id);
      const why = apmReason(entry.summary);

      const cells: Record<string, TableCell> = {
        name: options.players
          ? { value: knownValue(playerName) }
          : { value: countryVal, iconSlot: true, country: tag ?? undefined, ...(runs ? { looks: runs } : {}) },
        apm: { value: rateValue(entry.summary), format: 'decimal1' },
        orders: { value: valueFromRawBigInt(entry.summary.orders, why), format: 'integer' },
        minutes: { value: valueFromNullable(entry.summary.unpausedMinutes, why), format: 'decimal1' },
      };
      if (options.players) {
        cells.country = { value: countryVal, iconSlot: true, country: tag ?? undefined, ...(runs ? { looks: runs } : {}) };
      }
      if (options.showPaused) {
        cells.paused = { value: valueFromRawBigInt(entry.summary.pausedOrders, why), format: 'integer' };
      }
      return {
        key: entry.id,
        name: nameStr,
        cells,
        tooltipTitle: nameStr,
        tooltipNote: valueNote(entry.summary),
      };
    });

    return {
      columns,
      rows,
      hour,
      rank: true,
      rankMarkers: options.rankMarkers,
      warnings,
    };
  },
});
