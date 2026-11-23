import { rowName, textAt, type FollowedAnswer } from '../../dashboard/follow/message.ts';
import { TRAILING_HOURS } from '../../dashboard/model.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import { overlapsHourRanges } from '../../families/holders.ts';
import { nextWindowRangesChange, type DeclaredTableContext, type HourRanges, type TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { lookAt, type CountryLooksMap } from '../../families/looks.ts';
import {
  declareTableProjection,
  type DeclarativeColumnSpec,
  type TableColumnSpec,
  type TableProjection,
} from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { OFFERED } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, partialValue, wireFloat, wireWhole, type NullableMeaning, type Value } from '../../families/value.ts';
import type { ApmReply, ApmSeries, ApmValue } from '../../generated/dashboard';
import { GAPS_NOTE, playerNames, validateApm, valueNote } from './apmData.ts';
import { apmHeadQuery } from './apmHead.ts';
import { DEFAULT_SORT, TABLE_APM_OPTIONS } from './definition.ts';

export interface ApmTableProjectionOptions {
  readonly players: boolean;
  readonly showPaused: boolean;
  readonly cellBars: boolean;
  readonly rankMarkers: boolean;
  readonly labels: CountryLooksMap;
  readonly unobserved?: HourRanges;
  readonly branchFirst?: number;
  readonly windowHours?: number;
  readonly staticSpan?: readonly [number, number] | null;
}

type DisplayApmRow = ApmSeries & { readonly displayName: string | null };

interface ParsedApm {
  readonly rows: readonly DisplayApmRow[];
  readonly refusal?: string;
  readonly warnings: readonly string[];
}

const APM = { apm: 0, confirmedApm: 1, orders: 2, pausedOrders: 3, minutes: 4, complete: 5, gaps: 6, tag: 7, hour: 8, name: 9, nameMissing: 10 } as const;

function withDisplayNames(rows: readonly ApmSeries[]): DisplayApmRow[] {
  const names = playerNames(rows);
  return rows.map(row => ({ ...row, displayName: names.get(row.id) ?? null }));
}

function apmReason(summary: ApmValue): NullableMeaning {
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

const observationHour = (row: ApmSeries, fallback: number): number =>
  (Number.isFinite(row.hour) ? row.hour : fallback);

function parseAnswer(answer: FollowedAnswer): ParsedApm {
  if (answer.kind === 'notRecorded') return { rows: [], refusal: answer.reason, warnings: [] };
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
    const readText = (col: number): string | null => {
      if (col >= answer.columns) return null;
      const idx = readBig(col);
      return idx === null ? null : textAt(answer, Number(idx));
    };
    const summary: ApmValue = {
      apm: readFloat(APM.apm),
      confirmedApm: readFloat(APM.confirmedApm),
      orders: readBig(APM.orders)?.toString() ?? null,
      pausedOrders: readBig(APM.pausedOrders)?.toString() ?? null,
      unpausedMinutes: readFloat(APM.minutes),
      complete: readBig(APM.complete) === 1n,
      gaps: readBig(APM.gaps) === 1n,
    };
    rows.push({
      id,
      name: readText(APM.name),
      nameMissing: readText(APM.nameMissing),
      tag: readText(APM.tag),
      hour: Number(readBig(APM.hour) ?? 0n),
      summary,
      points: [],
    });
  }
  return { rows: withDisplayNames(rows), warnings: [] };
}

function parseReply(raw: unknown): ParsedApm {
  const reply: ApmReply = validateApm(raw);
  return reply.available
    ? { rows: withDisplayNames(reply.series), warnings: reply.warnings }
    : { rows: [], refusal: 'Player orders were not recorded in this campaign.', warnings: reply.warnings };
}

export function apmTableColumns(players: boolean, showPaused: boolean, cellBars: boolean): TableColumnSpec[] {
  return [
    { key: 'name', label: players ? 'Player' : 'Country', as: players ? 'plain' : 'flagName', offered: players ? OFFERED.text : OFFERED.country, priority: 0, text: true, hideable: false, widest: ['United Kingdom'] },
    ...(players ? [{ key: 'country', label: 'Country', as: 'flagName' as const, offered: OFFERED.country, priority: 2, text: true, widest: ['United Kingdom'] }] : []),
    { key: 'apm', label: 'eAPM', as: cellBars ? 'faint' : 'plain', tone: 'blue', offered: OFFERED.number, priority: 1, numeric: true, format: 'decimal1', widest: ['999.9'], hint: 'Orders per unpaused real minute' },
    { key: 'orders', label: 'Orders', as: 'plain', offered: OFFERED.number, priority: 1, numeric: true, format: 'integer', widest: ['999,999'], hint: 'Player-issued gameplay orders during unpaused play' },
    ...(showPaused ? [{ key: 'paused', label: 'Paused orders', as: 'plain' as const, offered: OFFERED.number, priority: 2, numeric: true, format: 'integer' as const, widest: ['99,999'] }] : []),
    { key: 'minutes', label: 'Unpaused minutes', as: 'plain', offered: OFFERED.number, priority: 3, numeric: true, format: 'decimal1', widest: ['9,999.9'] },
  ];
}

export const apmTableProjection: TableProjection<ApmTableProjectionOptions> = registerTableProjection(
  declareTableProjection<ParsedApm, DisplayApmRow, ApmTableProjectionOptions>({
    kind: 'apm-table',
    validate: parseReply,
    fromAnswer: parseAnswer,
    nextChange: (_sources, hour, opt) => (
      opt.staticSpan ? Infinity : nextWindowRangesChange(opt.unobserved, opt.windowHours ?? TRAILING_HOURS, hour)
    ),
    rank: true,
    rankMarkers: opt => opt.rankMarkers,
    columns: (_parsed, opt): readonly DeclarativeColumnSpec<DisplayApmRow, ApmTableProjectionOptions>[] => {
      const specs = apmTableColumns(opt.players, opt.showPaused, opt.cellBars);
      const gapsPartial = (row: DisplayApmRow) => (row.summary.gaps ? GAPS_NOTE : null);
      return specs.map((spec): DeclarativeColumnSpec<DisplayApmRow, ApmTableProjectionOptions> => {
        if (spec.key === 'name') {
          return opt.players
            ? {
                ...spec,
                value: row => (row.displayName !== null
                  ? knownValue(row.displayName)
                  : notRecorded(row.nameMissing ?? 'Player name was not recorded.')),
              }
            : { ...spec, countryField: 'tag', lookHour: (row, h) => observationHour(row, h) };
        }
        if (spec.key === 'country') return { ...spec, countryField: 'tag', lookHour: (row, h) => observationHour(row, h) };
        if (spec.key === 'apm') return { ...spec, value: row => rateValue(row.summary) };
        if (spec.key === 'orders') {
          return {
            ...spec,
            path: 'summary.orders',
            rawBigInt: true,
            whenNull: { kind: 'notRecorded', reason: 'Player orders were not recorded for this period.' },
            partial: gapsPartial,
          };
        }
        if (spec.key === 'paused') {
          return {
            ...spec,
            path: 'summary.pausedOrders',
            rawBigInt: true,
            whenNull: { kind: 'notRecorded', reason: 'Paused player orders were not recorded for this period.' },
            partial: gapsPartial,
          };
        }
        return {
          ...spec,
          path: 'summary.unpausedMinutes',
          whenNull: { kind: 'notRecorded', reason: 'Unpaused play time was not recorded for this period.' },
          partial: gapsPartial,
        };
      });
    },
    rows: (parsed, _live, hour, opt) => {
      if (parsed.refusal) return { rows: [], empty: parsed.refusal, emptyKind: 'unavailable', warnings: parsed.warnings };
      if (opt.unobserved?.length) {
        const win = opt.windowHours ?? TRAILING_HOURS;
        const [winFirst, winLast] = opt.staticSpan ?? [Math.max(opt.branchFirst ?? 0, hour - (win - 1)), hour];
        if (overlapsHourRanges(opt.unobserved, winFirst, winLast)) {
          return { rows: [], empty: 'Country group membership was not recorded throughout this period.', emptyKind: 'unavailable', warnings: parsed.warnings };
        }
      }
      if (!parsed.rows.length) return { rows: [], empty: 'No eAPM is available for these countries, players and dates.', emptyKind: 'empty', warnings: parsed.warnings };
      return { rows: parsed.rows, warnings: parsed.warnings };
    },
    rowKey: row => row.id,
    rowName: (row, hour, opt) => opt.players
      ? (row.displayName ?? 'Name not recorded')
      : (lookAt(row.tag ? opt.labels[row.tag] : undefined, observationHour(row, hour))?.name ?? 'Name not recorded'),
    tooltipTitle: (row, hour, opt) => opt.players
      ? (row.displayName ?? 'Name not recorded')
      : (lookAt(row.tag ? opt.labels[row.tag] : undefined, observationHour(row, hour))?.name ?? 'Name not recorded'),
    tooltipNote: row => valueNote(row.summary),
  }),
);

function apmHeadParams(ctx: DeclaredTableContext<unknown>) {
  const membership = ctx.countries.membershipRuns
    ? ctx.countries.membershipRuns.flatMap(run => {
      const held = ctx.gateSpan(run.firstHour, run.lastHour);
      return held ? [{ tag: run.tag, ...held }] : [];
    })
    : null;
  return {
    branchId: ctx.branch.id,
    group: ctx.options.group === 'player' ? 'player' as const : 'country' as const,
    tags: ctx.countries.all ? null : [...ctx.countries.tags],
    membership,
    players: contentSet(ctx.options.players).map(contentKey),
  };
}

export const apmTableDeclaration: TableWidgetDeclaration<ApmTableProjectionOptions> = {
  family: 'table',
  kind: 'apm-table',
  label: 'eAPM over the selected period',
  subject: 'eAPM',
  emptyCampaign: 'Load a campaign to see eAPM.',
  icon: 'chart-bars',
  optionFields: TABLE_APM_OPTIONS,
  countries: {
    mode: 'multi',
    labels: true,
    allowSingleSlot: true,
    allTagsForLabels: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group',
  },
  source: {
    kind: 'head',
    product: 'player-apm',
    timeShape: 'trailing-window',
    windowHours: TRAILING_HOURS,
    followQuery: ctx => apmHeadQuery(apmHeadParams(ctx), ctx.branch, TRAILING_HOURS),
    productParams: (ctx, span) => ({ ...apmHeadParams(ctx), registry: false, ...span, buckets: 1 }),
    dynamicKeys: ['firstHour', 'lastHour', 'buckets'],
    validate: validateApm,
  },
  projection: apmTableProjection,
  columns: ctx => apmTableColumns(ctx.options.group === 'player', ctx.options.showPausedActions === true, ctx.options.cellBars === true),
  settings: { kind: 'options', defaultSort: DEFAULT_SORT },
  projectionOptions: ctx => ({
    players: ctx.options.group === 'player',
    showPaused: ctx.options.showPausedActions === true,
    cellBars: ctx.options.cellBars === true,
    rankMarkers: ctx.options.rankMarkers === true,
    labels: ctx.countries.labels,
    unobserved: ctx.countries.unobserved,
    branchFirst: ctx.branch.firstHour,
    windowHours: TRAILING_HOURS,
    staticSpan: ctx.staticSpan,
  }),
  unavailable: ctx => (!ctx.following && ctx.staticSpan && ctx.countries.unobservedIn(ctx.staticSpan[0], ctx.staticSpan[1]))
    ? 'Country group membership was not recorded throughout this period.'
    : null,
  keyByBranch: true,
};
