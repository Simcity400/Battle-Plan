import { unavailableArt } from '../../families/art/types.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import { formatPower, formatPowerChange } from '../../dashboard/powerDecimals.ts';
import type { StatusMarkKind } from '../../draw/kit.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { identityFlags, identityName, newIdentityBoundary } from '../../families/table/identityCell.ts';
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
import { registerFamilyProjection } from '../../families/registry.ts';
import { DEFAULT_TAIL_PAGE_SIZE, type DatedLogRow } from '../../families/table/rowSource.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { EventFeedSpanReply, MembershipRun, ScriptCurrency, ScriptOccurrence, ScriptPageRow } from '../../generated/dashboard';
import { kindsRequest, validateSpan } from '../event-feed/eventFeedData.ts';
import { plainScripted, reasonsOf } from '../event-feed/scriptedGaps.ts';
import { cardFacets, recordingBreaks, validateOccurrence, type RecordingBreak } from '../event-feed/scriptOccurrences.ts';
import type { DecisionDetailSubject } from './DecisionDetailWindow.tsx';
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

const CHARGEABLE_KINDS = new Set(['decisionSelected', 'decisionTimeoutEffect']);

function decisionCostInfo(
  readView: BoundedReadView,
  at: DecisionAt,
): { readonly cell: TableCell; readonly costText: string | null; readonly costMissing: string | null } {
  const chargeable = at.edges.filter(edge => CHARGEABLE_KINDS.has(edge.row.kind));
  const proven = at.edges.filter(edge => edge.costs !== null);
  if (!chargeable.length && !proven.length) {
    return { cell: { value: notApplicable('No selection cost'), blank: true }, costText: null, costMissing: null };
  }
  if (chargeable.some(edge => edge.costs === null)) {
    return {
      cell: { value: notRecorded('Decision cost was not recorded') },
      costText: null,
      costMissing: 'Decision cost was not recorded',
    };
  }
  const totals = new Map<ScriptCurrency, bigint>();
  for (const edge of proven) {
    for (const cost of edge.costs!) {
      totals.set(cost.currency, (totals.get(cost.currency) ?? 0n) + BigInt(cost.amount));
    }
  }
  if (!totals.size) {
    return { cell: { value: notApplicable('No cost'), blank: true }, costText: null, costMissing: null };
  }
  const pp = totals.get('politicalPower');
  const cp = totals.get('commandPower');
  const formatCost = (raw: bigint) => (raw < 0n ? formatPower(-raw, 0) : formatPowerChange(raw, 0));
  if (pp !== undefined && cp === undefined) {
    const text = formatCost(pp);
    const sprite = readView.art({ kind: 'sprite', key: 'GFX_pol_power', frame: 0 });
    return {
      cell: {
        value: knownValue(text),
        unitSuffix: 'PP',
        unitArt: sprite,
      },
      costText: `${text} PP`,
      costMissing: null,
    };
  }
  if (cp !== undefined && pp === undefined) {
    const text = formatCost(cp);
    const sprite = readView.art({ kind: 'sprite', key: 'GFX_command_power', frame: 0 });
    return {
      cell: {
        value: knownValue(text),
        unitSuffix: 'CP',
        unitArt: sprite,
      },
      costText: `${text} CP`,
      costMissing: null,
    };
  }
  const parts: string[] = [];
  if (pp !== undefined) parts.push(`${formatCost(pp)} PP`);
  if (cp !== undefined) parts.push(`${formatCost(cp)} CP`);
  const joined = parts.join(', ');
  return { cell: { value: knownValue(joined) }, costText: joined, costMissing: null };
}

function decisionTimerCell(at: DecisionAt): { readonly cell: TableCell; readonly nextChangeAt: number } {
  if (at.status !== 'active' || !at.timer) {
    return { cell: { value: notApplicable('No active timer'), blank: true }, nextChangeAt: Infinity };
  }
  const left = daysLeft(at);
  const totalDays = Math.max(1, at.timer.days);
  const fraction = left !== null ? Math.max(0, Math.min(1, left / totalDays)) : 0;
  const nextChangeAt = left !== null ? at.timer.from + (at.timer.days - left + 1) * 24 : Infinity;
  if (left === null) {
    return {
      cell: {
        value: knownValue('Ending'),
        bar: { fraction: knownValue(fraction) },
      },
      nextChangeAt: Infinity,
    };
  }
  return {
    cell: {
      value: knownValue(formatScalar(left, 'integer')),
      unitSuffix: left === 1 ? 'day left' : 'days left',
      bar: { fraction: knownValue(fraction) },
    },
    nextChangeAt,
  };
}

function buildDecisionRow(
  readView: BoundedReadView,
  at: DecisionAt,
  artwork: boolean,
  boundary: ReturnType<typeof newIdentityBoundary>,
): { readonly row: TableRow; readonly nextChangeAt: number } {
  const { card } = at;
  const firstRow = card.occurrence.edges[0]!.row;
  const nameHour = firstRow.nameHour;
  const isMission = at.start?.row.kind === 'decisionActivated' || at.edges.some(edge => edge.row.kind === 'decisionActivated');
  const markInfo = DECISION_STATUS_MARKS[at.status];
  const statusMark: StatusMarkKind = at.status === 'completed' && isMission ? 'succeeded' : markInfo.mark;
  const labelText = markInfo.label ?? statusLabel(at);
  const muted = markInfo.muted === true;

  const artHandle = artwork ? urlArtHandle(card.item?.icon ?? null) : null;
  const art = artHandle ? readView.art(artHandle) : unavailableArt('The product drew no icon for this item');

  const rawName = card.item?.name && card.item.name.trim().length > 0 ? card.item.name : null;
  const safeName = plainScripted(rawName ?? decisionName(card)) || 'Decision name not recorded';
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
  let placeLabel: string | null = null;
  if (stateId !== null) {
    const stateRes = readView.who('state', String(stateId), firstRow.hour);
    if (stateRes.name.status === 'ready') {
      placeLabel = stateRes.name.value;
      placeCell = { value: knownValue(placeLabel) };
    } else if (stateRes.name.status === 'pending') {
      placeLabel = `State ${stateId}`;
      placeCell = { value: knownValue(placeLabel) };
    } else {
      placeCell = { value: notRecorded('State name not recorded') };
    }
  }

  const timer = decisionTimerCell(at);
  const cost = decisionCostInfo(readView, at);
  const countryFlag = card.country ? identityFlags(readView, 'country', [card.country], { atHour: nameHour, boundary })[0] : undefined;
  const targetFlag = card.target ? identityFlags(readView, 'country', [card.target], { atHour: nameHour, boundary })[0] : undefined;
  const detailSubject: DecisionDetailSubject = {
    key: card.occurrence.id,
    asOfHour: at.hour,
    startedHour: firstRow.hour,
    name: rawName,
    isMission,
    icon: art.status === 'ready' ? art.url : undefined,
    countryTag: card.country,
    countryName: card.country ? identityName(readView, 'country', card.country, nameHour) : null,
    countryFlagUrl: countryFlag?.art.status === 'ready' ? countryFlag.art.url : undefined,
    countryFlagPending: countryFlag?.art.status === 'pending',
    targetTag: card.target,
    targetName: card.target ? identityName(readView, 'country', card.target, nameHour) : null,
    targetFlagUrl: targetFlag?.art.status === 'ready' ? targetFlag.art.url : undefined,
    targetFlagPending: targetFlag?.art.status === 'pending',
    place: placeLabel,
    statusLabel: labelText,
    lifecycle: lifecycleLine(at),
    daysLeft: daysLeft(at),
    costText: cost.costText,
    costMissing: cost.costMissing,
    token: card.item?.token ?? null,
    description: card.item?.description ?? null,
    failedAt: at.failedAt,
    failureMissing: at.failureMissing,
    nameMissing: reasons.nameMissing ?? null,
    gaps: reasons.gaps ?? null,
  };

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
    nextChangeAt: timer.nextChangeAt,
    row: {
      key: card.occurrence.id,
      name: safeName,
      rowHour: firstRow.hour,
      live: at.status === 'active',
      muted,
      cells: {
        date: logDateCell(firstRow.hour),
        kind: artwork
          ? {
              value: rawName !== null ? knownValue(safeName) : notRecorded(reasons.nameMissing ?? 'Decision icon not recorded'),
              art,
              iconSlot: true,
              artShape: 'square',
              artName: safeName,
            }
          : { value: notApplicable('Game artwork hidden'), blank: true },
        country: logCountryCell(readView, countryEntries, nameHour, boundary),
        item: rawName !== null
          ? { value: knownValue(rawName), ...(isMission ? { inlineSubtext: 'Mission' } : {}) }
          : { value: notRecorded(reasons.nameMissing ?? 'Decision name not recorded') },
        place: placeCell,
        timer: timer.cell,
        cost: cost.cell,
        status: logStatusCell(labelText, statusMark, muted ? 'muted' : undefined),
      },
      tooltipTitle: safeName,
      tooltipExtra,
      tooltipAction: 'Click to view decision',
      meta: detailSubject,
    },
  };
}

export const decisionsProjection: TableProjection<DecisionsProjectionOptions> = registerFamilyProjection({
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
    let nextTimerChange = Infinity;
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
        const built = buildDecisionRow(readView, resolved.item, options.artwork, boundary);
        if (built.nextChangeAt > hour && built.nextChangeAt < nextTimerChange) nextTimerChange = built.nextChangeAt;
        return built.row;
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
    const rawHead = source.rawHead as {
      readonly activeCount?: string | number | null;
      readonly warnings?: readonly string[];
      readonly unavailable?: readonly { readonly label: string; readonly reason: string }[];
    } | undefined;
    const productActive = rawHead?.activeCount != null && Number.isFinite(Number(rawHead.activeCount))
      ? Number(rawHead.activeCount)
      : null;
    const wholePopulationVisible = windowOffset === 0
      && source.total <= HEAD_LIMIT
      && (!readView.rowWindow || readView.rowWindow.count >= source.total);
    const runningCount = statuses.has('active')
      ? (productActive ?? (wholePopulationVisible ? liveRows.length : null))
      : null;
    const titleSummary = runningCount !== null && runningCount > 0 ? `${formatScalar(runningCount, 'integer')} running` : null;
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
      nextChangeAt: Math.min(boundary.next, nextTimerChange),
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
