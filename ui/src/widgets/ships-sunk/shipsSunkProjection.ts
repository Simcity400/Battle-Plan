import { drawableIcon } from '../../dashboard/artwork.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import { contentSet, setToken } from '../../dashboard/options.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { artKindCell, newIdentityBoundary, resolveName } from '../../families/table/identityCell.ts';
import { LOG_TABLE_BASE, logCountryCell, logDateCell, logWatchCell, occurrenceSelection, pagedTailEntry, projectPagedSlots, urlArtHandle } from '../../families/table/logHelpers.ts';
import type { TableCell, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, type Value } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { ShipClassInfo, ShipSinkingAttribution, SinkingRow, ShipsSunkTailReply } from '../../generated/dashboard';
import { SHIPS_SUNK_COLUMNS } from './definition.ts';

interface Options {
  readonly prefix: string;
  readonly selected: readonly string[];
}

const base = { columns: SHIPS_SUNK_COLUMNS, unsorted: true, dateGrouped: true, hideHeader: true, rowTimeBoundaries: false, rhythm: 'log' } as const;

function killerBeside(cause: ShipSinkingAttribution): string | undefined {
  if (cause.kind === 'ship') return `by ${cause.className ?? cause.classKey ?? cause.name}`;
  if (cause.kind === 'airWing') return 'by aircraft';
  return cause.kind === 'cause' ? cause.text : undefined;
}

function sunkBy(cause: ShipSinkingAttribution): readonly TooltipExtraRow[] {
  const classOf = (name: string | null, key: string | null): Value => {
    const shown = name ?? key;
    return shown ? knownValue(shown) : notRecorded('Killer class not recorded');
  };
  if (cause.kind === 'ship') return [{ label: 'Sunk by', value: knownValue(cause.name) }, { label: 'Killer class', value: classOf(cause.className, cause.classKey) }];
  if (cause.kind === 'airWing') {
    return cause.carrierName
      ? [{ label: 'Sunk by', value: knownValue(`Aircraft from ${cause.carrierName}`) }, { label: 'Carrier class', value: classOf(cause.carrierClassName, cause.carrierClassKey) }]
      : [{ label: 'Sunk by', value: knownValue('Aircraft') }];
  }
  return [{ label: 'Sunk by', value: cause.kind === 'cause' ? knownValue(cause.text) : notRecorded('Killer not recorded') }];
}

export interface SinkingSubject {
  readonly hour: number;
  readonly battle: { readonly key: string } | null;
}

function shipRow(view: BoundedReadView, row: SinkingRow, shipClass: ShipClassInfo | undefined, boundary: ReturnType<typeof newIdentityBoundary>): TableRow {
  const name = row.name ?? 'Ship name not recorded';
  const className = shipClass?.name ?? row.recordedClass;
  const victim = { tag: row.victim, dimmed: true, missingLabel: 'Victim not recorded', role: 'Victim' };
  const attacker = { tag: row.attacker, missingLabel: 'Killer not recorded', role: 'Killer' };
  const cause = row.sinkingCause;
  const namedCause = row.attacker === null && cause.kind === 'cause';
  const flags = logCountryCell(view, namedCause ? [victim] : [attacker, victim], row.hour, boundary);
  const country = namedCause ? { ...flags, tooltipRows: [{ label: 'Killer', value: notRecorded('Killer not recorded') }, ...flags.tooltipRows ?? []] } : flags;
  const place = row.seaZone ? knownValue(row.seaZone) : notRecorded('Sea zone not recorded');
  let loss: TableCell;
  let battle: SinkingSubject['battle'] = null;
  if (row.battle.kind === 'linked') {
    const battleValue = resolveName(view, 'battle', row.battle.id, { boundary }).value;
    const killer = killerBeside(cause);
    const tooltipRows = [{ label: 'Battle', value: battleValue }, ...sunkBy(cause)];
    loss = { value: battleValue, strong: true, tooltipRows, ...(killer ? { inlineSubtext: killer } : {}) };
    battle = { key: row.battle.id };
  } else {
    loss = cause.kind === 'notRecorded' ? { value: notRecorded(cause.reason), missingFigure: 'Cause' }
      : { value: knownValue(cause.kind === 'cause' ? cause.text : cause.kind === 'airWing' ? 'Air attack' : `Sunk by ${cause.name}`) };
  }
  return {
    key: row.key, name, rowHour: row.hour, meta: { hour: row.hour, battle } satisfies SinkingSubject,
    cells: {
      date: logDateCell(row.hour),
      kind: artKindCell(view, urlArtHandle(drawableIcon(shipClass?.icon)), 'equipment', className, 'Ship class artwork not recorded'),
      country,
      item: { value: row.name ? knownValue(row.name) : notRecorded(name), missingFigure: 'Ship name', inlineSubtext: className ?? 'Class not recorded',
        tooltipRows: [{ label: 'Class', value: className ? knownValue(className) : notRecorded('Ship class not recorded') }] },
      place: { value: place, missingFigure: 'Sea zone' }, cause: loss,
      action: battle ? logWatchCell() : { value: notApplicable('No recorded battle to watch'), blank: true },
    },
    tooltipExtra: () => [{ label: 'First recorded', value: knownValue(formatDateTime(row.observed)) }],
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
  family: 'table', kind: 'ships-sunk', label: 'Ships sunk', subject: 'ships', icon: 'ship',
  emptyCampaign: 'Load a campaign to see its ships sunk log.', countries: { mode: 'filter', content: 'history' }, columns: SHIPS_SUNK_COLUMNS,
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
