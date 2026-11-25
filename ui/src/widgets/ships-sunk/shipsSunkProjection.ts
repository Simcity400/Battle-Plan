import { drawableIcon } from '../../dashboard/artwork.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import { contentSet, setToken } from '../../dashboard/options.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { artKindCell, newIdentityBoundary, resolveName } from '../../families/table/identityCell.ts';
import { LOG_TABLE_BASE, logCountryCell, logDateCell, occurrenceSelection, pagedTailEntry, projectPagedSlots, urlArtHandle } from '../../families/table/logHelpers.ts';
import type { TableCell, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notRecorded, type Value } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { ShipClassInfo, ShipSinkingAttribution, SinkingRow, ShipsSunkTailReply } from '../../generated/dashboard';
import { SHIPS_SUNK_COLUMNS } from './definition.ts';

interface Options {
  readonly prefix: string;
  readonly selected: readonly string[];
}

const base = { columns: SHIPS_SUNK_COLUMNS, unsorted: true, dateGrouped: true, dayDividersOnly: true, hideHeader: true, rowTimeBoundaries: false } as const;

function killerBeside(cause: ShipSinkingAttribution): string | undefined {
  if (cause.kind === 'ship') return `by ${cause.className ?? cause.classKey ?? cause.name}`;
  if (cause.kind === 'airWing') return 'by aircraft';
  return cause.kind === 'cause' ? cause.text : undefined;
}

function sunkBy(cause: ShipSinkingAttribution): Value {
  if (cause.kind === 'ship') {
    const shipClass = cause.className ?? cause.classKey;
    return knownValue(shipClass ? `${cause.name} (${shipClass})` : cause.name);
  }
  if (cause.kind === 'airWing') {
    const carrierClass = cause.carrierClassName ?? cause.carrierClassKey;
    if (!cause.carrierName) return knownValue('Aircraft');
    return knownValue(carrierClass ? `Aircraft from ${cause.carrierName} (${carrierClass})` : `Aircraft from ${cause.carrierName}`);
  }
  return cause.kind === 'cause' ? knownValue(cause.text) : notRecorded('Killer not recorded');
}

function shipRow(view: BoundedReadView, row: SinkingRow, shipClass: ShipClassInfo | undefined, boundary: ReturnType<typeof newIdentityBoundary>): TableRow {
  const name = row.name ?? 'Ship name not recorded';
  const className = shipClass?.name ?? row.recordedClass;
  const victim = { tag: row.victim, dimmed: true, missingLabel: 'Victim not recorded' };
  const attacker = { tag: row.attacker, missingLabel: 'Killer not recorded' };
  const cause = row.sinkingCause;
  const country = logCountryCell(view, row.attacker === null && cause.kind === 'cause' ? [victim] : [attacker, victim], row.hour, boundary);
  const place = row.seaZone ? knownValue(row.seaZone) : notRecorded('Sea zone not recorded');
  let loss: TableCell;
  let explained: readonly TooltipExtraRow[];
  if (row.battle.kind === 'linked') {
    const where = row.battleProvince === null ? null : resolveName(view, 'province', String(row.battleProvince), { atHour: row.hour, boundary }).value;
    const killer = killerBeside(cause);
    loss = where?.kind === 'known' ? { value: knownValue(`Battle of ${where.value}`), strong: true, narrowText: 'Battle', ...(killer ? { inlineSubtext: killer } : {}) }
      : { value: notRecorded('Naval battle recorded, battle name not recorded'), missingFigure: 'Battle name' };
    explained = [{ label: 'Battle', value: loss.value }, { label: 'Sunk by', value: sunkBy(cause) }];
  } else {
    loss = cause.kind === 'notRecorded' ? { value: notRecorded(cause.reason), missingFigure: 'Cause' }
      : { value: knownValue(cause.kind === 'cause' ? cause.text : cause.kind === 'airWing' ? 'Air attack' : `Sunk by ${cause.name}`) };
    explained = [{ label: 'Cause', value: loss.value }];
  }
  return {
    key: row.key, name, rowHour: row.hour, meta: row.hour,
    cells: {
      date: logDateCell(row.hour),
      kind: artKindCell(view, urlArtHandle(drawableIcon(shipClass?.icon)), 'equipment', className, 'Ship class artwork not recorded'),
      country,
      item: { value: row.name ? knownValue(row.name) : notRecorded(name), missingFigure: 'Ship name', inlineSubtext: className ?? 'Class not recorded' },
      place: { value: place, missingFigure: 'Sea zone' }, cause: loss,
    },
    tooltipExtra: () => [
      { label: 'Class', value: className ? knownValue(className) : notRecorded('Ship class not recorded') },
      { label: 'Killer', value: row.attacker ? logCountryCell(view, [attacker], row.hour).value : notRecorded('Killer not recorded') },
      { label: 'Victim', value: row.victim ? logCountryCell(view, [victim], row.hour).value : notRecorded('Victim not recorded') },
      { label: 'Sea zone', value: place }, ...explained,
      { label: 'First recorded', value: knownValue(formatDateTime(row.observed)) },
    ],
  };
}

export const shipsSunkProjection: TableProjection<Options> = registerFamilyProjection({
  kind: 'ships-sunk', family: 'table',
  project(view, options): TableModel {
    const hour = view.hour;
    const source = view.rows<SinkingRow>('main');
    const refused = view.refused('main') ?? source?.refused;
    if (!source || refused) return { ...base, hour, rows: [], ...(refused ? { refused } : {}) };
    const head = source.rawHead as ShipsSunkTailReply | undefined;
    const classes = (head?.classes ?? []).filter(chip => chip.firstHour <= Math.min(hour, source.at ?? hour));
    const byToken = new Map(classes.map(chip => [chip.class.token, chip.class]));
    const boundary = newIdentityBoundary();
    const page = projectPagedSlots(source, hour, view.rowWindow, row => shipRow(view, row, byToken.get(row.classToken), boundary));
    const selected = new Set(options.selected);
    const items = classes.map(({ class: type }) => {
      const value = options.prefix + type.token;
      return { value, label: type.name ?? type.key ?? `Class name not recorded (${type.token})`, icon: drawableIcon(type.icon) ?? undefined,
        checked: selected.size === 0 || selected.has(value) };
    });
    return {
      ...base, ...page, hour, nextChangeAt: boundary.next,
      kindFilter: { key: 'classes', noun: 'classes', allLabel: 'All classes', items, selected: options.selected },
      titleSummary: source.total === 0 && source.missing.length ? 'No recorded sinkings' : `${formatScalar(source.total)} sunk`,
      ...(source.missing.length ? { titleSummaryPartial: 'Count from recorded hours' } : {}),
      warnings: head?.warnings,
      ...(page.slots.length ? {} : { empty: 'No recorded sinkings match these settings.' }),
    };
  },
});

const prefixOf = (campaign: string, branch: string) => setToken(`${campaign}:${branch}:`);

export const shipsSunkDeclaration: TableWidgetDeclaration<Options> = {
  family: 'table', kind: 'ships-sunk', label: 'Ships sunk', subject: 'ships', icon: 'anchor',
  emptyCampaign: 'Load a campaign to see its ships sunk log.', countries: { mode: 'filter' }, columns: SHIPS_SUNK_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx) {
    const win = ctx.widget.time.mode === 'range' ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const from = win ? ctx.gateSpan(Math.max(Math.floor(win.firstHour), ctx.branch.firstHour), ctx.branch.lastHour)?.firstHour ?? null : null;
    const selected = contentSet(ctx.options.classes);
    const prefix = prefixOf(ctx.track.source.metadata.campaignKey, ctx.branch.id);
    const classes = selected.filter(key => key.startsWith(prefix)).map(key => Number(key.slice(prefix.length))).filter(Number.isSafeInteger);
    const ask = { ...occurrenceSelection(ctx), ...(selected.length ? { classes } : {}) };
    return {
      owner: JSON.stringify([ctx.track.source.id, ctx.branch.id, 'ships-sunk', from, ask]),
      entries: [pagedTailEntry('main', ctx.track.source, ctx.branch.id, ctx.track.axisOffset,
        { product: 'ships-sunk-tail', branchId: ctx.branch.id, from, limit: 64, ask }, ctx.following)],
    };
  },
  projection: shipsSunkProjection,
  projectionOptions: ctx => ({ prefix: prefixOf(ctx.track.source.metadata.campaignKey, ctx.branch.id), selected: contentSet(ctx.options.classes) }),
};
