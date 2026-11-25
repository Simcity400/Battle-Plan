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
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { DivisionEvents, DivisionLogRow, DivisionLogTailReply } from '../../generated/dashboard';
import { DIVISION_LOG_COLUMNS } from './definition.ts';

interface Options {
  readonly ranged: boolean;
  readonly events: DivisionEvents;
  readonly prefix: string;
  readonly selected: readonly string[];
}

const base = { columns: DIVISION_LOG_COLUMNS, unsorted: true, dateGrouped: true, dayDividersOnly: true, hideHeader: true, rowTimeBoundaries: false } as const;
const blank: TableCell = { value: notApplicable('No detail for this row'), blank: true };
const statuses: Readonly<Record<string, string>> = {
  first_observed: 'First seen', division_destroyed: 'Destroyed', division_overrun: 'Overrun', division_deleted: 'Removed',
  division_disbanded: 'Disbanded', division_capitulation: 'Removed at capitulation', division_scripted: 'Removed by script',
  division_template_removed: 'Template removed', returned: 'Returned', merged: 'Merged', hq_transferred: 'HQ transferred',
  volunteer_transferred: 'Volunteer transferred', exile_transferred: 'Exile transferred',
};

function divisionRow(view: BoundedReadView, row: DivisionLogRow, boundary: ReturnType<typeof newIdentityBoundary>): TableRow {
  const date = logDateCell(row.hour, row.event?.kind === 1 && !row.event.exact ? 'by' : undefined);
  const country = row.tag === null
    ? { value: notRecorded('Division owner not recorded'), missingFigure: 'Owner' }
    : logCountryCell(view, [{ tag: row.tag }], row.observed, boundary);
  const e = row.event;
  if (!e) {
    const name = `${formatScalar(row.members)} divisions`;
    return {
      key: row.key, name, rowHour: row.hour, pageGroup: { members: row.members, open: row.open },
      cells: { date, kind: { value: knownValue('Division group'), glyph: 'helmet', artTile: 'flat' }, item: { ...country, inlinePrefix: name, tooltipLabel: 'Country' },
        country: blank, place: blank, outcome: blank, action: blank },
    };
  }
  const child = row.parent !== null;
  const place: TableCell = e.province !== null && e.placeHour !== null
    ? { value: resolveName(view, 'province', String(e.province), { atHour: e.placeHour, boundary }).value, glyph: 'place', missingFigure: 'Place',
      ...(e.kind === 0 || e.exact ? {} : { tooltipLabel: 'Last seen at' }) }
    : { value: notRecorded('Location not recorded'), missingFigure: 'Place', ...(e.kind === 0 || e.exact ? {} : { tooltipLabel: 'Last seen at' }) };
  const name = e.name ?? 'Division name not recorded';
  const art = drawableIcon(e.icon);
  const kind = artKindCell(view, urlArtHandle(art), 'drawn', e.templateName, e.iconMissing ?? 'Division template artwork not recorded');
  const status = statuses[e.status] ?? 'Removal cause not recorded';
  const outcome: TableCell = e.kind === 0 ? blank
    : e.kind === 2 ? { value: knownValue('Transferred'), glyph: 'transfer', tone: 'muted' }
    : e.killerTag !== null || e.matchedDeaths !== null
      ? { ...logCountryCell(view, [{ tag: e.killerTag, missingLabel: 'Killer not attributed' }], row.observed, boundary), glyph: 'loss' }
      : { value: statuses[e.status] ? knownValue(status) : notRecorded(status), glyph: 'loss', missingFigure: 'Cause' };
  const tooltipExtra = (): TooltipExtraRow[] => [
    { label: 'Event', value: statuses[e.status] ? knownValue(status) : notRecorded(status) },
    ...(child ? [{ label: 'Country', value: country.value }] : []),
    { label: 'Template', value: e.templateName ? knownValue(e.templateName) : notRecorded(e.templateMissing ?? 'Template not recorded') },
    { label: 'Observed', value: knownValue(formatDateTime(row.observed)) },
    ...(e.afterHour === null ? [] : [{ label: 'First seen after', value: knownValue(formatDateTime(e.afterHour)) }]),
    ...(e.matchedDeaths === null ? [] : [{ label: 'Killer', value: e.killerTag === null
      ? notRecorded(e.matchedDeaths > 1 ? 'Multiple deaths matched this observation' : 'Killer not recorded') : outcome.value }]),
    ...(e.killerDivision === null ? [] : [{ label: 'Killer division', value: knownValue(e.killerDivision) }]),
    ...(!art && e.iconMissing ? [{ label: 'Artwork', value: notRecorded(e.iconMissing) }] : []),
  ];
  return {
    key: row.key, name, rowHour: row.hour, meta: row.observed, ...(row.parent ? { pageParent: row.parent } : {}),
    cells: {
      date: child ? blank : date, kind: child ? blank : { ...kind, artTile: 'raised' },
      item: { value: e.name ? knownValue(e.name) : notRecorded(e.nameMissing ?? name), missingFigure: 'Division',
        ...(!child && e.templateName ? { inlineSubtext: e.templateName } : {}) },
      country: child ? blank : country, place, outcome, action: blank,
    },
    tooltipExtra,
  };
}

export const divisionLogProjection: TableProjection<Options> = registerFamilyProjection({
  kind: 'division-log', family: 'table',
  project(view, options): TableModel {
    const hour = view.hour;
    const source = view.rows<DivisionLogRow>('main');
    const refused = view.refused('main') ?? source?.refused;
    if (!source || refused) return { ...base, hour, rows: [], ...(refused ? { refused } : {}) };
    const boundary = newIdentityBoundary();
    const page = projectPagedSlots(source, hour, view.rowWindow, row => divisionRow(view, row, boundary));
    const head = source.rawHead as DivisionLogTailReply | undefined;
    const at = Math.min(hour, source.at ?? hour);
    const chips = (head?.templates ?? []).filter(chip => chip.firstHour <= at);
    const selected = new Set(options.selected);
    const items = chips.map(chip => {
      const value = options.prefix + chip.id;
      return { value, label: chip.name ?? `Template name not recorded (${chip.id})`, checked: selected.size === 0 || selected.has(value) };
    });
    const totals = [...(source.facetTotals ?? [])].filter(([kind]) =>
      options.events === 'both' || (kind === 'created') === (options.events === 'created'));
    const titleSummary = totals.map(([kind, count]) => {
      const partial = kind === 'created' ? head?.censusIncomplete : source.missing.length > 0;
      return partial && count === 0 ? `No ${kind} divisions recorded` : `${formatScalar(count)} ${kind}`;
    }).join(' · ');
    return {
      ...base, ...page, hour, nextChangeAt: boundary.next, ...(options.ranged || !page.slots.length ? {} : { endNote: 'Start of the recording' }),
      kindFilter: { key: 'templates', noun: 'templates', allLabel: 'All templates', items, selected: options.selected },
      warnings: head?.warnings, titleSummary,
      ...(titleSummary && source.missing.length ? { titleSummaryPartial: 'Count from recorded hours' } : {}),
      ...(page.slots.length ? {} : head?.unavailable
        ? { empty: 'Division history was not recorded.', emptyDetail: head.unavailable, emptyKind: 'unavailable' as const }
        : { empty: 'No recorded division events match these settings.' }),
    };
  },
});

const prefixOf = (campaign: string, branch: string) => setToken(`${campaign}:${branch}:`);
const eventsOf = (value: unknown): DivisionEvents => value === 'created' || value === 'lost' ? value : 'both';

export const divisionLogDeclaration: TableWidgetDeclaration<Options> = {
  family: 'table', kind: 'division-log', label: 'Division log', subject: 'divisions', icon: 'shield',
  emptyCampaign: 'Load a campaign to see its divisions.', countries: { mode: 'filter' }, columns: DIVISION_LOG_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx) {
    const win = ctx.widget.time.mode === 'range' ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const from = win ? ctx.gateSpan(Math.max(Math.floor(win.firstHour), ctx.branch.firstHour), ctx.branch.lastHour)?.firstHour ?? null : null;
    const selected = contentSet(ctx.options.templates);
    const prefix = prefixOf(ctx.track.source.metadata.campaignKey, ctx.branch.id);
    const templates = selected.filter(key => key.startsWith(prefix)).map(key => Number(key.slice(prefix.length))).filter(Number.isSafeInteger);
    const expanded = contentSet(ctx.options.expanded);
    const ask = {
      ...occurrenceSelection(ctx), events: eventsOf(ctx.options.events),
      ...(selected.length ? { templates } : {}), ...(expanded.length ? { expanded } : {}), ...(ctx.options.totals === true ? { totals: true } : {}),
    };
    return {
      owner: JSON.stringify([ctx.track.source.id, ctx.branch.id, 'division-log', from, ask]),
      entries: [pagedTailEntry('main', ctx.track.source, ctx.branch.id, ctx.track.axisOffset,
        { product: 'division-log-tail', branchId: ctx.branch.id, from, limit: 64, ask }, ctx.following)],
    };
  },
  projection: divisionLogProjection,
  projectionOptions: ctx => ({ ranged: ctx.widget.time.mode === 'range', events: eventsOf(ctx.options.events), prefix: prefixOf(ctx.track.source.metadata.campaignKey, ctx.branch.id), selected: contentSet(ctx.options.templates) }),
};
