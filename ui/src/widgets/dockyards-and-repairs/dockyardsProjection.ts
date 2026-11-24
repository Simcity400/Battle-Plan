import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableProjection, type TooltipExtraRow } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, textValue, notApplicable, valueFromNullable, type Value } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { NavalRepairsReply, RepairPort, YardShip } from '../../generated/dashboard';
import { fraction, shipLabel, shipPicture, shipPictureCell, shipStrength, shipTooltip, validateRepairs } from '../refits/yardsData.ts';
import type { ArtState } from '../../families/table/identityCell.ts';
import { COLUMNS, TABLE_BASE } from './definition.ts';

const NO_OPTIONS: Record<string, never> = {};
const ON_PORT = 'Shown on the port';
const ON_SHIP = 'Shown on each ship';
const PERCENT = { decimals: 0, maxDecimals: 1, percent: true } as const;

type DockRow =
  | { readonly kind: 'port'; readonly key: string; readonly port: RepairPort; readonly children: readonly DockRow[] }
  | { readonly kind: 'ship'; readonly key: string; readonly ship: YardShip; readonly picture: ArtState };

const portName = (port: RepairPort): Value<string> => textValue(port.name, 'Port name unavailable');

const [NAME, STRENGTH, PRIORITY, DOCKYARDS] = COLUMNS;

const DOCK_COLUMNS: readonly DeclarativeColumnSpec<DockRow, Record<string, never>>[] = [
  { ...NAME!, priority: 0, text: true, cellKind: 'item', hideable: false, widest: ['Wilhelmshaven harbour'],
    value: row => row.kind === 'port' ? portName(row.port) : knownValue(shipLabel(row.ship).name),
    extra: row => (row.kind === 'ship' ? shipPictureCell(row.ship, row.picture) : {}) },
  { ...STRENGTH!, priority: 1, numeric: true, format: PERCENT, widest: ['100.0%'],
    value: (row, live) => row.kind === 'ship' ? shipStrength(row.ship, live) : notApplicable(ON_SHIP),
    extra: row => (row.kind === 'port' ? { blank: true } : {}),
    bar: (row, live) => row.kind === 'ship' ? { fraction: fraction(shipStrength(row.ship, live)) } : null },
  { ...PRIORITY!, priority: 2, numeric: true, format: 'integer', widest: ['9'],
    value: row => row.kind === 'port'
      ? valueFromNullable(row.port.priority, { kind: 'notRecorded', reason: 'Repair priority not recorded' })
      : notApplicable(ON_PORT),
    extra: row => (row.kind === 'ship' ? { blank: true } : {}) },
  { ...DOCKYARDS!, priority: 3, numeric: true, format: 'integer', widest: ['99'], unit: 'repairing',
    value: row => row.kind === 'port'
      ? valueFromNullable(row.port.dockyards, { kind: 'notRecorded', reason: 'Dockyards repairing not recorded' })
      : notApplicable(ON_PORT),
    extra: row => (row.kind === 'ship' ? { blank: true } : {}) },
];

function summary(reply: NavalRepairsReply): string | null {
  const parts: string[] = [];
  if (reply.queuedShips !== null) parts.push(`${formatScalar(reply.queuedShips, 'integer')} ${reply.queuedShips === 1 ? 'ship' : 'ships'} waiting`);
  if (reply.usedDockyards !== null && reply.repairLimit !== null) {
    parts.push(`${formatScalar(reply.usedDockyards, 'integer')} / ${formatScalar(reply.repairLimit, 'integer')} repair dockyards in use`);
  }
  return parts.length ? parts.join(' · ') : null;
}

export const dockyardsProjection: TableProjection<Record<string, never>> = registerTableProjection(
  declareTableProjection<NavalRepairsReply, DockRow, Record<string, never>>({
    kind: 'dockyards-and-repairs',
    validate: validateRepairs,
    columns: DOCK_COLUMNS,
    rows: (reply, _live, _hour, _options, readView) => {
      const rows: DockRow[] = reply.ports.map(port => ({
        kind: 'port',
        key: String(port.province),
        port,
        children: port.queue.map((ship, index) => ({ kind: 'ship', key: `${port.province}|${index}|${ship.handle}`, ship, picture: shipPicture(ship, readView) })),
      }));
      return {
        rows,
        ...(!rows.length
          ? reply.censusComplete
            ? { empty: 'No repair ports at this date', emptyKind: 'empty' as const }
            : { empty: 'Repair ports were not fully recorded at this date', emptyKind: 'unavailable' as const }
          : {}),
        titleSummary: summary(reply),
        warnings: reply.warnings,
      };
    },
    rowKey: row => row.key,
    rowName: row => row.kind === 'port' ? (row.port.name ?? 'Port name unavailable') : shipLabel(row.ship).name,
    children: row => row.kind === 'port' && row.children.length ? row.children : undefined,
    initiallyOpen: () => false,
    tooltipExtra: (row): TooltipExtraRow[] => row.kind === 'ship'
      ? shipTooltip(row.ship)
      : [
          { label: 'Repair priority', value: textValue(row.port.policy, 'Repair priority name not localised'), divider: true },
          { label: 'Ships waiting', value: knownValue(formatScalar(row.port.queue.length, 'integer')) },
        ],
  }),
);

export const dockyardsDeclaration: TableWidgetDeclaration<Record<string, never>> = {
  family: 'table',
  kind: 'dockyards-and-repairs',
  label: 'Dockyards and repairs',
  subject: 'repair ports',
  emptyCampaign: 'Load a campaign to see dockyards and repairs.',
  icon: 'cube',
  countries: { mode: 'single', choose: 'Choose a country to see its dockyards.' },
  sources: ctx => {
    const params = ctx.countryTag ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) } : null;
    return {
      owner: `dockyards-and-repairs:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
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
              product: 'naval-repairs',
              followQuery: { panel: 'naval-repairs', ...params },
              params,
              validate: validateRepairs,
            },
          }]
        : [],
    };
  },
  projection: dockyardsProjection,
  columns: DOCK_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: () => NO_OPTIONS,
  jumpToDate: true,
};
