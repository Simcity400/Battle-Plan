import { resolveWindow } from '../../dashboard/model.ts';
import { contentSet } from '../../dashboard/options.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { artKindCell, newIdentityBoundary, resolveName, type IdentityBoundary } from '../../families/table/identityCell.ts';
import { LOG_TABLE_BASE, logCountryCell, logDateCell, occurrenceSelection, pagedTailEntry, projectPagedSlots } from '../../families/table/logHelpers.ts';
import type { TableCell, TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { PeaceDealsRow, PeaceDealsTailReply, PeaceTermCount, PeaceTermKind } from '../../generated/dashboard';
import { PEACE_DEALS_COLUMNS } from './definition.ts';

const base = { columns: PEACE_DEALS_COLUMNS, unsorted: true, dateGrouped: true, hideHeader: true, rowTimeBoundaries: false, rhythm: 'log' } as const;
const blank: TableCell = { value: notApplicable('No detail for this row'), blank: true };
const TERMS_INSET = 104;
const TERM_WIDTH = 80;
const labels: Record<PeaceTermKind, readonly [string, string]> = {
  state: ['state', 'states'], puppet: ['puppet', 'puppets'], government: ['new government', 'new governments'],
  release: ['released', 'released'], ship: ['ship', 'ships'], other: ['other action', 'other actions'],
};

function quantity({ kind, count }: PeaceTermCount) {
  return { figure: formatScalar(count), unit: labels[kind][count === 1 ? 0 : 1] };
}

function summary(terms: ReturnType<typeof quantity>[]) {
  return terms.map(({ figure, unit }) => `${figure} ${unit}`).join(' · ');
}

type Options = { expanded: readonly string[] };

function peaceRow(view: BoundedReadView, row: PeaceDealsRow, boundary: IdentityBoundary, options: Options): TableRow {
  const d = row.detail;
  const common = { key: row.key, rowHour: row.hour, meta: row.hour };
  if (d.kind === 'treaty') {
    const country = logCountryCell(view, [
      { tag: d.winner, missingLabel: 'Winner not recorded' }, { tag: d.loser, dimmed: true, missingLabel: 'Loser not recorded' },
    ], row.hour, boundary);
    const counted = d.terms.map(quantity);
    const value = counted.length ? knownValue(summary(counted)) : d.whitePeace ? knownValue('White peace') : notRecorded('Peace terms not recorded');
    const quantities = counted.length ? counted : d.whitePeace ? [{ unit: 'White peace' }] : [];
    return {
      ...common, name: 'Peace deal',
      ...(d.members ? { pageGroup: { members: d.members, open: options.expanded.includes(row.key) } } : {}),
      cells: {
        date: logDateCell(row.hour),
        kind: artKindCell(view, { kind: 'sprite', key: 'GFX_global_diplorequest_icons', frame: 20 }, 'drawn', 'Peace deal', 'Peace deal artwork unavailable'),
        country,
        terms: { value, missingFigure: 'Terms', quantities },
        action: blank,
      },
      tooltipExtra: () => [{ label: d.derived ? 'Inferred sides' : 'Recorded sides', value: country.value },
        { label: d.derived ? 'Observed net outcomes' : 'Settlement terms', value }],
      tooltipNote: d.limitations.join(' '),
    };
  }
  if (d.kind === 'more') {
    const name = d.hidden ? `Show all ${formatScalar(d.terms)} terms · ${formatScalar(d.hidden)} more` : 'Show fewer terms';
    const cell: TableCell = { value: knownValue(name) };
    return { ...common, key: `all:${row.parent}`, name, muted: true, pageParent: row.parent ?? undefined, detailColumn: 'terms',
      cells: { date: blank, kind: blank, country: blank, terms: cell, action: blank } };
  }
  const flags = d.items.map(term => {
    const flag = logCountryCell(view, [{ tag: term.tag, missingLabel: term.kind === 'state' ? 'Former owner not recorded' : 'Country not recorded' }], term.at, boundary).flags![0]!;
    const name = term.kind === 'state'
      ? term.state === null ? null : resolveName(view, 'state', String(term.state), { atHour: row.hour, boundary }).value
      : term.tag === null ? null : resolveName(view, 'country', term.tag, { atHour: term.at, boundary }).value;
    const known = name?.kind === 'known';
    const label = known ? String(name.value) : term.kind === 'state' ? 'State name not recorded' : 'Country name not recorded';
    return { ...flag, label, ...(known ? {} : { muted: true }), ...(term.kind === 'state' ? {} : { note: labels[term.kind][0] }) };
  });
  const list = flags.map(f => f.note ? `${f.label} \u00b7 ${f.note}` : f.label);
  let name: string;
  let cell: TableCell;
  if (d.kind === 'recipient') {
    const recipient = logCountryCell(view, [{ tag: d.tag, missingLabel: 'Recipient not recorded' }], row.hour, boundary);
    const text = d.tag === null || recipient.value.kind !== 'known' ? notRecorded('Recipient not recorded') : knownValue(String(recipient.value.value));
    const note = summary(d.terms.map(quantity));
    name = [text.kind === 'known' ? text.value : 'Recipient not recorded', note].filter(Boolean).join(' \u00b7 ');
    cell = { value: knownValue([name, ...list].join(', ')), heading: { flag: recipient.flags![0]!, text, ...(note ? { note } : {}) }, ...(flags.length ? { flags } : {}) };
  } else {
    name = list.length > 1 ? `${list[0]} \u2013 ${list[list.length - 1]}` : (list[0] ?? 'Terms');
    cell = { value: knownValue(list.join(', ')), flags };
  }
  return {
    ...common, key: `child:${row.key}`, name, pageParent: row.parent ?? undefined, detailColumn: 'terms',
    cells: { date: blank, kind: blank, country: blank, terms: cell, action: blank },
  };
}

export const peaceDealsProjection: TableProjection<Options> = registerFamilyProjection({
  kind: 'peace-deals', family: 'table',
  project(view, options): TableModel {
    const hour = view.hour;
    const source = view.rows<PeaceDealsRow>('main');
    const refused = view.refused('main') ?? source?.refused;
    if (!source || refused) return { ...base, hour, rows: [], ...(refused ? { refused } : {}) };
    const boundary = newIdentityBoundary();
    const page = projectPagedSlots(source, hour, view.rowWindow, row => peaceRow(view, row, boundary, options));
    const total = source.facetTotals?.get('treaties');
    return {
      ...base, ...page, hour, nextChangeAt: boundary.next,
      titleSummary: total === undefined ? undefined : total === 0 && source.missing.length ? 'No recorded peace deals' : `${formatScalar(total)} ${total === 1 ? 'peace deal' : 'peace deals'}`,
      ...(source.missing.length ? { titleSummaryPartial: 'Count from recorded hours' } : {}),
      warnings: (source.rawHead as PeaceDealsTailReply | undefined)?.warnings,
      ...(page.slots.length ? {} : { empty: 'No peace deals recorded through this hour.' }),
    };
  },
});

export const peaceDealsDeclaration: TableWidgetDeclaration<Options> = {
  family: 'table', kind: 'peace-deals', label: 'Peace deals', subject: 'peace deals', icon: 'olive-branch',
  emptyCampaign: 'Load a campaign to see its peace deals.', countries: { mode: 'filter' }, columns: PEACE_DEALS_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx, viewport) {
    const win = ctx.widget.time.mode === 'range' ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const from = win ? ctx.gateSpan(Math.max(Math.floor(win.firstHour), ctx.branch.firstHour), ctx.branch.lastHour)?.firstHour ?? null : null;
    const expanded = contentSet(ctx.options.expanded);
    const full = contentSet(ctx.options.full).filter(key => expanded.includes(key));
    const termsPerLine = Math.max(1, Math.min(64, Math.floor(((viewport.width ?? 320) - TERMS_INSET) / TERM_WIDTH)));
    const ask = { ...occurrenceSelection(ctx), ...(expanded.length ? { expanded, ...(full.length ? { full } : {}), termsPerLine } : {}) };
    return {
      owner: JSON.stringify([ctx.track.source.id, ctx.branch.id, 'peace-deals', from, ask]),
      entries: [pagedTailEntry('main', ctx.track.source, ctx.branch.id, ctx.track.axisOffset,
        { product: 'peace-deals-tail', branchId: ctx.branch.id, from, limit: 64, ask }, ctx.following)],
    };
  },
  projection: peaceDealsProjection,
  projectionOptions: ctx => ({ expanded: contentSet(ctx.options.expanded) }),
};
