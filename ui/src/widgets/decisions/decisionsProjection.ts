import { formatDateTime } from '../../dashboard/calendar.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import { formatPowerChange, formatSpent } from '../../dashboard/powerDecimals.ts';
import type { StatusMarkKind } from '../../draw/kit.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { newIdentityBoundary } from '../../families/table/identityCell.ts';
import {
  LOG_TABLE_BASE,
  logCountryCell,
  logDateCell,
  logStatusCell,
  pagedTailEntry,
  projectPagedSlots,
  urlArtHandle,
} from '../../families/table/logHelpers.ts';
import type { TableCell, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { DEFAULT_TAIL_PAGE_SIZE, type DatedLogRow } from '../../families/table/rowSource.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { EventFeedSpanReply, MembershipRun, ScriptCurrency, ScriptOccurrence, ScriptPageRow } from '../../generated/dashboard';
import { kindsRequest, validateSpan } from '../event-feed/eventFeedData.ts';
import { plainScripted, reasonsOf } from '../event-feed/scriptedGaps.ts';
import { cardFacets, recordingBreaks, validateOccurrence, type RecordingBreak } from '../event-feed/scriptOccurrences.ts';
import {
  CONTINUING,
  daysLeft,
  decisionAt,
  decisionCard,
  decisionName,
  lifecycleLine,
  OPENERS,
  STATUS_CHOICES,
  statusLabel,
  type DecisionAt,
} from './decisionsData.ts';
import { DECISIONS_COLUMNS } from './definition.ts';

export interface DecisionsProjectionOptions {
  readonly statuses: readonly string[];
  readonly artwork: boolean;
  readonly breaks: readonly RecordingBreak[];
  readonly filter: {
    readonly passing: Readonly<Record<string, readonly (readonly [number, number])[]>> | null;
  };
}

const HEAD_LIMIT = 256;

const DECISION_STATUS_MARKS: Readonly<Record<DecisionAt['status'], { readonly mark: StatusMarkKind; readonly label?: string; readonly muted?: boolean }>> = {
  active: { mark: 'running', label: 'Running' },
  taken: { mark: 'finished', label: 'Taken' },
  completed: { mark: 'finished', label: 'Completed' },
  failed: { mark: 'ended', label: 'Failed', muted: true },
  cancelled: { mark: 'ended', label: 'Cancelled', muted: true },
  timedOut: { mark: 'ended', label: 'Timed out', muted: true },
  removed: { mark: 'ended', label: 'Removed', muted: true },
  unsupported: { mark: 'ended', muted: true },
  unknown: { mark: 'ended', label: 'Not recorded', muted: true },
};

function decisionCostCell(at: DecisionAt): TableCell {
  const proven = at.edges.filter(edge => edge.costs !== null);
  if (!proven.length) return { value: notRecorded('Decision cost was not recorded') };
  const totals = new Map<ScriptCurrency, bigint>();
  for (const edge of proven) {
    for (const cost of edge.costs!) {
      totals.set(cost.currency, (totals.get(cost.currency) ?? 0n) + BigInt(cost.amount));
    }
  }
  if (!totals.size) return { value: notApplicable('No cost') };
  const pp = totals.get('politicalPower');
  const cp = totals.get('commandPower');
  if (pp !== undefined && cp === undefined) {
    const text = pp < 0n ? formatSpent(-pp, 0) : formatPowerChange(pp, 0);
    return { value: knownValue(text), unitSuffix: 'PP' };
  }
  if (cp !== undefined && pp === undefined) {
    const text = cp < 0n ? formatSpent(-cp, 0) : formatPowerChange(cp, 0);
    return { value: knownValue(text), unitSuffix: 'CP' };
  }
  const parts: string[] = [];
  if (pp !== undefined) parts.push(`${pp < 0n ? formatSpent(-pp, 0) : formatPowerChange(pp, 0)} PP`);
  if (cp !== undefined) parts.push(`${cp < 0n ? formatSpent(-cp, 0) : formatPowerChange(cp, 0)} CP`);
  return { value: knownValue(parts.join(', ')) };
}

function decisionTimerCell(at: DecisionAt): TableCell {
  if (at.status !== 'active' || !at.timer) return { value: notApplicable('No active timer'), blank: true };
  const left = daysLeft(at);
  const totalDays = Math.max(1, at.timer.days);
  const elapsedHours = Math.max(0, at.hour - at.timer.from);
  const fraction = Math.max(0, Math.min(1, 1 - elapsedHours / (totalDays * 24)));
  if (left === null) {
    return {
      value: knownValue('Ending'),
      bar: { fraction: knownValue(fraction) },
    };
  }
  return {
    value: knownValue(formatScalar(left, 'integer')),
    unitSuffix: left === 1 ? 'day left' : 'days left',
    bar: { fraction: knownValue(fraction) },
  };
}

function buildDecisionRow(
  readView: BoundedReadView,
  at: DecisionAt,
  artwork: boolean,
  boundary: ReturnType<typeof newIdentityBoundary>,
): TableRow {
  const { card } = at;
  const firstRow = card.occurrence.edges[0]!.row;
  const nameHour = firstRow.nameHour;
  const isMission = at.start?.row.kind === 'decisionActivated' || at.edges.some(edge => edge.row.kind === 'decisionActivated');
  const markInfo = DECISION_STATUS_MARKS[at.status];
  const statusMark: StatusMarkKind = at.status === 'completed' && isMission ? 'succeeded' : markInfo.mark;
  const labelText = markInfo.label ?? statusLabel(at);
  const muted = markInfo.muted === true;

  const artHandle = artwork ? urlArtHandle(card.item?.icon ?? null) : null;
  const artResult = artHandle ? readView.art(artHandle) : null;
  const icon = artResult?.status === 'ready' ? artResult.url : undefined;
  const iconPending = artResult?.status === 'pending' ? true : undefined;

  const rawName = card.item?.name ?? null;
  const reasons = reasonsOf(card.item);
  const participants = firstRow.participants ?? [card.country, card.target];
  const countryEntries: { readonly tag: string | null; readonly missingLabel?: string }[] = [
    { tag: card.country, missingLabel: 'Country not identified' },
  ];
  if (card.target) {
    countryEntries.push({ tag: card.target });
  } else if (participants[1] === null && card.occurrence.edges.some(e => e.row.cause?.notes.some(n => n.includes('target')))) {
    countryEntries.push({ tag: null, missingLabel: 'Target country not recorded' });
  }

  const stateId = at.edges.find(e => e.row.cause?.state != null)?.row.cause?.state ?? null;
  let placeCell: TableCell = { value: notApplicable('No target state'), blank: true };
  if (stateId !== null) {
    const stateRes = readView.who('state', String(stateId), firstRow.hour);
    placeCell = stateRes.name.status === 'ready'
      ? { value: knownValue(stateRes.name.value) }
      : stateRes.name.status === 'pending'
        ? { value: knownValue(`State ${stateId}`) }
        : { value: notRecorded('State name not recorded') };
  }

  const tooltipExtra = (): TooltipExtraRow[] => {
    const out: TooltipExtraRow[] = [
      { label: 'Status', value: knownValue(labelText) },
      { label: 'Lifecycle', value: knownValue(lifecycleLine(at)) },
    ];
    if (at.failedAt !== null) out.push({ label: 'Failure observed', value: knownValue(formatDateTime(at.failedAt)) });
    if (at.failureMissing) out.push({ label: 'Mission failure state', value: notRecorded(at.failureMissing) });
    if (card.item?.token) out.push({ label: 'Content identity', value: knownValue(card.item.token) });
    if (reasons.nameMissing) out.push({ label: 'Name not recorded', value: notRecorded(reasons.nameMissing) });
    if (reasons.gaps) out.push({ label: 'Text not recorded', value: notRecorded(reasons.gaps) });
    if (card.item?.description) out.push({ label: 'Description', value: knownValue(plainScripted(card.item.description)), divider: true });
    return out;
  };

  return {
    key: card.occurrence.id,
    name: plainScripted(decisionName(card)),
    rowHour: firstRow.hour,
    live: at.status === 'active',
    muted,
    cells: {
      date: logDateCell(firstRow.hour),
      kind: artwork
        ? {
            value: rawName !== null ? knownValue(plainScripted(rawName)) : notRecorded(reasons.nameMissing ?? 'Decision icon not recorded'),
            icon,
            iconPending,
            iconSlot: true,
            artShape: 'square',
            artName: plainScripted(decisionName(card)),
          }
        : { value: notApplicable('Game artwork hidden'), blank: true },
      country: logCountryCell(readView, countryEntries, nameHour, boundary),
      item: rawName !== null
        ? { value: knownValue(rawName), ...(isMission ? { inlineSubtext: 'Mission' } : {}) }
        : { value: notRecorded(reasons.nameMissing ?? 'Decision name not recorded') },
      place: placeCell,
      timer: decisionTimerCell(at),
      cost: decisionCostCell(at),
      status: logStatusCell(labelText, statusMark, muted ? 'muted' : undefined),
    },
    tooltipTitle: plainScripted(decisionName(card)),
    tooltipExtra,
  };
}

export const decisionsProjection: TableProjection<DecisionsProjectionOptions> = registerTableProjection({
  kind: 'decisions',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused !== undefined) {
      return { columns: DECISIONS_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, refused };
    }
    const source = readView.rows<ScriptPageRow & DatedLogRow>('main');
    if (!source) {
      return { columns: DECISIONS_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true };
    }
    if (source.refused !== null) {
      return { columns: DECISIONS_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, refused: source.refused };
    }
    const statuses = new Set(options.statuses);
    const countryFn = options.filter.passing
      ? (tag: string, atHour: number) => !!options.filter.passing![tag]?.some(([first, last]) => first <= atHour && atHour <= last)
      : null;
    const boundary = newIdentityBoundary();
    const { slots, rows, windowOffset, totalRows } = projectPagedSlots(
      source,
      hour,
      readView.rowWindow,
      rawRow => {
        if (!rawRow.occurrence) return null;
        let occ: ScriptOccurrence;
        try {
          occ = validateOccurrence(rawRow.occurrence);
        } catch {
          return null;
        }
        const card = decisionCard(occ);
        const first = card?.occurrence.edges[0]?.row;
        const resolved = decisionAt(
          { key: rawRow.key, id: occ.id, hour: first?.hour ?? rawRow.hour, revealedHour: first?.readHour ?? rawRow.hour, by: false, item: card },
          hour,
          options.breaks,
          statuses,
          countryFn,
        );
        if (!resolved) return null;
        return buildDecisionRow(readView, resolved.item, options.artwork, boundary);
      },
    );

    const liveRows: TableRow[] = [];
    const restSlots: typeof slots = [];
    for (const slot of slots) {
      if (slot.kind === 'data' && slot.row.live) liveRows.push(slot.row);
      else restSlots.push(slot);
    }
    const orderedSlots = [...liveRows.map(row => ({ kind: 'data' as const, row })), ...restSlots];
    const orderedRows = [...liveRows, ...rows.filter(r => !r.live)];
    const allLoaded = windowOffset === 0 && source.total <= HEAD_LIMIT;
    const runningCount = allLoaded && statuses.has('active') ? liveRows.length : null;
    const titleSummary = runningCount !== null && runningCount > 0 ? `${formatScalar(runningCount, 'integer')} running` : null;
    const rawHead = source.rawHead as { readonly warnings?: readonly string[]; readonly unavailable?: readonly { readonly label: string; readonly reason: string }[] } | undefined;
    const warnings = [
      ...(rawHead?.warnings ?? []),
      ...(rawHead?.unavailable ?? []).map(u => `${u.label}: ${u.reason}`),
    ];

    return {
      columns: DECISIONS_COLUMNS,
      rows: orderedRows,
      slots: orderedSlots,
      hour,
      hideHeader: true,
      unsorted: true,
      dateGrouped: true,
      windowOffset,
      totalRows,
      nextChangeAt: boundary.next,
      rowTimeBoundaries: false,
      titleSummary,
      ...(source.missing.length > 0 && titleSummary ? { titleSummaryPartial: 'Count from recorded hours' } : {}),
      ...(warnings.length ? { warnings } : {}),
      ...(orderedSlots.length === 0 ? { empty: 'No recorded decisions match these settings.' } : {}),
    };
  },
});

export const decisionsDeclaration: TableWidgetDeclaration<DecisionsProjectionOptions, EventFeedSpanReply> = {
  family: 'table',
  kind: 'decisions',
  label: 'Decisions',
  subject: 'decisions',
  emptyCampaign: 'Load a campaign to see decisions.',
  icon: 'ballot',
  countries: { mode: 'filter', labels: true },
  aux: {
    product: 'event-feed-span',
    params: track => kindsRequest(track).params as Record<string, unknown>,
    validate: validateSpan,
  },
  columns: DECISIONS_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx): SourcePlan {
    const kinds = ctx.aux?.kinds;
    if (!kinds) return { owner: `${ctx.track.source.id}:${ctx.branch.id}:decisions:loading`, entries: [] };
    const win = !ctx.following ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const rawFrom = ctx.widget.time.mode === 'range' && win ? Math.max(Math.floor(win.firstHour), ctx.branch.firstHour) : null;
    const boundFrom = rawFrom !== null ? ctx.gateSpan(rawFrom, ctx.branch.lastHour)?.firstHour ?? null : null;
    const scopeRuns: MembershipRun[] | undefined = ctx.countries.filter.passing
      ? Object.entries(ctx.countries.filter.passing).flatMap(([tag, ranges]) =>
          ranges.flatMap(([first, last]) => {
            const bound = ctx.gateSpan(first, last);
            return bound ? [{ tag, firstHour: bound.firstHour, lastHour: bound.lastHour }] : [];
          }),
        )
      : undefined;
    const facets = cardFacets(kinds, OPENERS, CONTINUING);
    return {
      owner: `${ctx.track.source.id}:${ctx.branch.id}:decisions:${scopeRuns ? scopeRuns.length : 'all'}`,
      entries: [
        pagedTailEntry(
          'main',
          ctx.track.source,
          ctx.branch.id,
          ctx.track.axisOffset,
          {
            product: 'decisions-tail',
            branchId: ctx.branch.id,
            facets,
            from: boundFrom,
            limit: HEAD_LIMIT,
            ...(scopeRuns ? { scope: scopeRuns } : {}),
          },
          ctx.following,
          DEFAULT_TAIL_PAGE_SIZE,
        ),
      ],
    };
  },
  projection: decisionsProjection,
  projectionOptions(ctx): DecisionsProjectionOptions {
    const statuses = STATUS_CHOICES.filter(status => ctx.options[status.value] !== false).map(status => status.value);
    return {
      statuses,
      artwork: ctx.options.artwork !== false,
      breaks: recordingBreaks(ctx.branch),
      filter: { passing: ctx.countries.filter.passing },
    };
  },
};
