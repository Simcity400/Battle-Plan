import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableProjection, type TooltipExtraRow } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import { knownValue, textValue, notApplicable, notRecorded, valueFromNullable, type Value } from '../../families/value.ts';
import type { NavalRefitsReply, RefitLine, YardShip } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';
import { fixedText, fraction, refitProgress, shipLabel, shipStrength, shipTooltip, validateRefits } from './yardsData.ts';

const NO_OPTIONS: Record<string, never> = {};

type RefitRow =
  | { readonly kind: 'line'; readonly key: string; readonly line: RefitLine; readonly children: readonly RefitRow[] }
  | { readonly kind: 'ship'; readonly key: string; readonly ship: YardShip };

const ON_LINE = 'Shown on the refit line';
const ON_SHIP = 'Shown on each ship';
const PERCENT = { decimals: 0, maxDecimals: 1, percent: true } as const;

const lineName = (line: RefitLine): Value<string> => textValue(line.target, 'Refit design name unavailable');

const icText = (raw: string | null, reason: string): Value<string> => raw === null ? notRecorded(reason) : knownValue(`${fixedText(raw)} IC`);

const [NAME, PROGRESS, STRENGTH, DOCKYARDS] = COLUMNS;

const REFIT_COLUMNS: readonly DeclarativeColumnSpec<RefitRow, Record<string, never>>[] = [
  { ...NAME!, priority: 0, text: true, hideable: false, widest: ['Early Cruiser C Klasse refit line'],
    value: row => row.kind === 'line' ? lineName(row.line) : knownValue(shipLabel(row.ship).name) },
  { ...PROGRESS!, priority: 1, numeric: true, format: PERCENT, widest: ['100.0%'], value: () => notApplicable(ON_LINE) },
  { ...STRENGTH!, priority: 2, numeric: true, format: PERCENT, widest: ['100.0%'], value: () => notApplicable(ON_SHIP) },
  { ...DOCKYARDS!, priority: 3, numeric: true, format: 'integer', widest: ['99'],
    value: row => row.kind === 'line'
      ? valueFromNullable(row.line.dockyards, { kind: 'notRecorded', reason: 'Refit dockyards not recorded' })
      : notApplicable(ON_LINE),
    extra: row => (row.kind === 'ship' ? { blank: true } : {}) },
];

export const refitsProjection: TableProjection<Record<string, never>> = registerTableProjection(
  declareTableProjection<NavalRefitsReply, RefitRow, Record<string, never>>({
    kind: 'refits',
    validate: validateRefits,
    columns: (reply, _options) => [
      REFIT_COLUMNS[0]!,
      { ...REFIT_COLUMNS[1]!, value: (row, live) => row.kind === 'line' ? refitProgress(reply, row.line, live) : notApplicable(ON_LINE),
        extra: row => (row.kind === 'ship' ? { blank: true } : {}),
        bar: (row, live) => row.kind === 'line' ? { fraction: fraction(refitProgress(reply, row.line, live)) } : null },
      { ...REFIT_COLUMNS[2]!, value: (row, live) => row.kind === 'ship' ? shipStrength(row.ship, live) : notApplicable(ON_SHIP),
        extra: row => (row.kind === 'line' ? { blank: true } : {}),
        bar: (row, live) => row.kind === 'ship' ? { fraction: fraction(shipStrength(row.ship, live)) } : null },
      REFIT_COLUMNS[3]!,
    ],
    rows: reply => {
      if (reply.missing !== null) return { rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
      const rows: RefitRow[] = reply.lines.map(line => ({
        kind: 'line',
        key: line.handle,
        line,
        children: line.ships.map(ship => ({ kind: 'ship', key: `${line.handle}|${ship.handle}`, ship })),
      }));
      return {
        rows,
        ...(!rows.length
          ? reply.censusComplete
            ? { empty: 'No refits at this date', emptyKind: 'empty' as const }
            : { empty: 'Refits were not fully recorded at this date', emptyKind: 'unavailable' as const }
          : {}),
        titleSummary: reply.dockyards === null ? null : `${reply.dockyards.toLocaleString('en-US')} ${reply.dockyards === 1 ? 'dockyard' : 'dockyards'}`,
        warnings: reply.warnings,
      };
    },
    rowKey: row => row.key,
    rowName: row => row.kind === 'line' ? (row.line.target ?? 'Refit design name unavailable') : shipLabel(row.ship).name,
    children: row => row.kind === 'line' ? row.children : undefined,
    initiallyOpen: () => false,
    tooltipExtra: (row, reply): TooltipExtraRow[] => row.kind === 'ship'
      ? shipTooltip(row.ship)
      : [
          { label: 'Work done', value: icText(row.line.progress, 'Refit progress not recorded'), divider: true },
          { label: 'Cost', value: icText(row.line.cost, 'Refit cost not recorded') },
          { label: 'Remaining cost', value: icText(row.line.remaining, 'Remaining cost not recorded') },
          { label: 'Ships', value: knownValue(row.line.ships.length.toLocaleString('en-US')) },
          ...(reply.costRead === null ? [] : [{ label: 'Cost read', value: knownValue(formatDateOnly(reply.costRead)) }]),
        ],
  }),
);

export const refitsDeclaration: TableWidgetDeclaration<Record<string, never>> = {
  family: 'table',
  kind: 'refits',
  label: 'Refits',
  subject: 'refits',
  emptyCampaign: 'Load a campaign to see refits.',
  icon: 'anchor',
  countries: { mode: 'single', choose: 'Choose a country to see its refits.' },
  sources: ctx => {
    const params = ctx.countryTag ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) } : null;
    return {
      owner: `refits:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
      entries: params
        ? [{
            name: 'main',
            source: ctx.track.source,
            branch: ctx.branch.id,
            offset: ctx.track.axisOffset,
            read: {
              shape: 'point',
              channel: 'panel',
              coverage: { mode: 'keyed', keys: [ctx.countryTag!] },
              product: 'naval-refits',
              followQuery: { panel: 'naval-refits', ...params },
              params,
              validate: validateRefits,
            },
          }]
        : [],
    };
  },
  projection: refitsProjection,
  columns: REFIT_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: () => NO_OPTIONS,
  jumpToDate: true,
};
