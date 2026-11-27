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
  occurrenceSelection,
  pagedTailEntry,
  projectPagedSlots,
  urlArtHandle,
} from '../../families/table/logHelpers.ts';
import type { TableCell, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { DEFAULT_TAIL_PAGE_SIZE, type DatedLogRow } from '../../families/table/rowSource.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, partialValue } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { ScriptCurrency, ScriptOccurrence, ScriptPageRow, ScriptStatus } from '../../generated/dashboard';
import { descriptionRows, gapRows, hasTooltipGaps, hoverTextOf, type TooltipTexts } from '../event-feed/hoverText.ts';
import { gapsForField, plainScripted, reasonsOf, scriptedValue } from '../event-feed/scriptedGaps.ts';
import { recordingBreaks, validateOccurrence, type RecordingBreak } from '../event-feed/scriptOccurrences.ts';
import type { DecisionDetailSubject } from './DecisionDetailWindow.tsx';
import {
  daysLeft,
  decisionAt,
  decisionCard,
  decisionName,
  lifecycleLine,
  STATUS_CHOICES,
  statusLabel,
  type DecisionAt,
  type StatusChoice,
} from './decisionsData.ts';
import { DECISIONS_COLUMNS } from './definition.ts';

export interface DecisionsProjectionOptions {
  readonly artwork: boolean;
  readonly breaks: readonly RecordingBreak[];
  readonly hover?: TooltipTexts;
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
  const proven = at.edges.filter(edge => edge.costs != null);
  if (!chargeable.length && !proven.length) {
    return { cell: { value: notApplicable('No selection cost'), blank: true }, costText: null, costMissing: null };
  }
  const wholly = chargeable.some(edge => edge.costs == null);
  const unread = new Set(chargeable.flatMap(edge => edge.costsUnread ?? []));
  const reasons = [
    ...(wholly ? ['Part of the decision cost was not recorded'] : []),
    ...(unread.size ? [`${[...unread].sort().map(currency => (currency === 'politicalPower' ? 'Political power' : 'Command power')).join(' and ')} cost was not recorded`] : []),
  ];
  const lacking = reasons.length ? reasons.join('; ') : null;
  const shown = (text: string) => (lacking ? partialValue(text, lacking) : knownValue(text));
  const totals = new Map<ScriptCurrency, bigint>();
  for (const edge of proven) {
    for (const cost of edge.costs!) {
      totals.set(cost.currency, (totals.get(cost.currency) ?? 0n) + BigInt(cost.amount));
    }
  }
  if (!totals.size) {
    const missing = wholly && !unread.size ? 'Decision cost was not recorded' : lacking;
    return missing
      ? { cell: { value: notRecorded(missing) }, costText: null, costMissing: missing }
      : { cell: { value: notApplicable('No cost'), blank: true }, costText: null, costMissing: null };
  }
  const pp = totals.get('politicalPower');
  const cp = totals.get('commandPower');
  const formatCost = (raw: bigint) => (raw < 0n ? formatPower(-raw, 0) : formatPowerChange(raw, 0));
  if (pp !== undefined && cp === undefined) {
    const text = formatCost(pp);
    const sprite = readView.art({ kind: 'sprite', key: 'GFX_pol_power', frame: 0 });
    return {
      cell: {
        value: shown(text),
        unitSuffix: 'PP',
        unitArt: sprite,
      },
      costText: `${text} PP`,
      costMissing: lacking,
    };
  }
  if (cp !== undefined && pp === undefined) {
    const text = formatCost(cp);
    const sprite = readView.art({ kind: 'sprite', key: 'GFX_command_power', frame: 0 });
    return {
      cell: {
        value: shown(text),
        unitSuffix: 'CP',
        unitArt: sprite,
      },
      costText: `${text} CP`,
      costMissing: lacking,
    };
  }
  const parts: string[] = [];
  if (pp !== undefined) parts.push(`${formatCost(pp)} PP`);
  if (cp !== undefined) parts.push(`${formatCost(cp)} CP`);
  const joined = parts.join(', ');
  return { cell: { value: shown(joined) }, costText: joined, costMissing: lacking };
}

function decisionTimerCell(at: DecisionAt): { readonly cell: TableCell; readonly nextChangeAt: number } {
  if (at.status !== 'active') {
    return { cell: { value: notApplicable('No active timer'), blank: true }, nextChangeAt: Infinity };
  }
  if (!at.timer) {
    return { cell: { value: notRecorded('Decision duration was not recorded'), missingFigure: 'Duration' }, nextChangeAt: Infinity };
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
  live: boolean,
  artwork: boolean,
  boundary: ReturnType<typeof newIdentityBoundary>,
  hover: TooltipTexts,
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
  const reasons = reasonsOf(card.item);
  const titleGaps = gapsForField(card.item?.gaps, 'title');
  const safeName = plainScripted(rawName ?? decisionName(card), titleGaps) || 'Decision name not recorded';
  const participants = firstRow.participants ?? [card.country, card.target];
  const countryEntries: { readonly tag: string | null; readonly missingLabel?: string }[] = [
    { tag: card.country, missingLabel: 'Country not identified' },
  ];
  if (card.target) {
    countryEntries.push({ tag: card.target });
  } else if (participants[1] === null && card.occurrence.edges.some(e => e.row.cause?.targetRecorded === true)) {
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
    out.push(...gapRows(hover, card.occurrence.id, card.item?.gaps));
    out.push(...descriptionRows(hover, card.occurrence.id, card.item?.description, card.item?.gaps));
    return out;
  };

  return {
    nextChangeAt: timer.nextChangeAt,
    row: {
      key: card.occurrence.id,
      name: safeName,
      rowHour: firstRow.hour,
      live,
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
          ? { value: scriptedValue(rawName, titleGaps), ...(isMission ? { inlineSubtext: 'Mission' } : {}) }
          : { value: notRecorded(reasons.nameMissing ?? 'Decision name not recorded') },
        place: placeCell,
        timer: timer.cell,
        cost: cost.cell,
        status: logStatusCell(labelText, statusMark, muted ? 'muted' : undefined),
      },
      tooltipTitle: safeName,
      tooltipExtra,
      ...(hasTooltipGaps(hover, card.occurrence.id, card.item?.gaps) ? { tooltipScrollId: card.occurrence.id } : {}),
      tooltipAction: 'Click to view decision',
      meta: detailSubject,
    },
  };
}

const RECORDED_STATUS: Readonly<Record<ScriptStatus, DecisionAt['status'] | null>> = {
  active: 'active', taken: 'taken', completed: 'completed', failed: 'failed', cancelled: 'cancelled', timedOut: 'timedOut', removed: 'removed',
  unsupported: 'unsupported', notRecorded: 'unknown', awaiting: null, chosen: null, available: null,
};

const ASKED_STATUSES: Readonly<Record<StatusChoice, readonly ScriptStatus[]>> = {
  active: ['active', 'notRecorded'],
  completed: ['taken', 'completed'],
  failed: ['failed', 'unsupported'],
  cancelled: ['cancelled'],
  timedOut: ['timedOut'],
  removed: ['removed'],
};

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
    const boundary = newIdentityBoundary();
    const asOf = source.at ?? hour;
    let nextTimerChange = Infinity;
    const { slots, rows, windowOffset, totalRows, gapBefore } = projectPagedSlots(
      source,
      asOf,
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
        const resolved = decisionAt(
          card,
          asOf,
          options.breaks,
        );
        if (!resolved) return null;
        const recorded = rawRow.status ? RECORDED_STATUS[rawRow.status] : null;
        const effective = recorded ? { ...resolved, status: recorded } : resolved;
        const live = typeof rawRow.live === 'boolean' ? rawRow.live : effective.status === 'active';
        const built = buildDecisionRow(readView, effective, live, options.artwork, boundary, options.hover ?? []);
        if (asOf === hour && built.nextChangeAt > hour && built.nextChangeAt < nextTimerChange) nextTimerChange = built.nextChangeAt;
        return built.row;
      },
    );

    const rawHead = source.rawHead as {
      readonly warnings?: readonly string[];
      readonly unavailable?: readonly { readonly label: string; readonly reason: string }[];
    } | undefined;
    const runningCount = source.facetTotals?.get('running') ?? null;
    const titleSummary = runningCount !== null && runningCount > 0 ? `${formatScalar(runningCount, 'integer')} running` : null;
    const warnings = [
      ...(rawHead?.warnings ?? []),
      ...(rawHead?.unavailable ?? []).map(u => `${u.label}: ${u.reason}`),
    ];

    return {
      columns: DECISIONS_COLUMNS,
      rows,
      slots,
      hour,
      hideHeader: true,
      unsorted: true,
      dateGrouped: true,
      windowOffset,
      totalRows,
      ...(gapBefore?.length ? { gapBefore } : {}),
      nextChangeAt: Math.min(boundary.next, nextTimerChange),
      rowTimeBoundaries: false,
      titleSummary,
      ...(source.missing.length > 0 && titleSummary ? { titleSummaryPartial: 'Count from recorded hours' } : {}),
      ...(warnings.length ? { warnings } : {}),
      ...(slots.length === 0 ? { empty: 'No recorded decisions match these settings.' } : {}),
    };
  },
});

export const decisionsDeclaration: TableWidgetDeclaration<DecisionsProjectionOptions> = {
  family: 'table',
  kind: 'decisions',
  label: 'Decisions',
  subject: 'decisions',
  emptyCampaign: 'Load a campaign to see decisions.',
  icon: 'ballot',
  countries: { mode: 'filter' },
  columns: DECISIONS_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx): SourcePlan {
    const win = !ctx.following ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const rawFrom = ctx.widget.time.mode === 'range' && win ? Math.max(Math.floor(win.firstHour), ctx.branch.firstHour) : null;
    const boundFrom = rawFrom !== null ? ctx.gateSpan(rawFrom, ctx.branch.lastHour)?.firstHour ?? null : null;
    const chosen = STATUS_CHOICES.filter(status => ctx.options[status.value] !== false).map(status => status.value);
    const statuses = chosen.length === STATUS_CHOICES.length ? null : chosen.flatMap(choice => ASKED_STATUSES[choice]);
    const selection = occurrenceSelection(ctx);
    return {
      owner: `${ctx.track.source.id}:${ctx.branch.id}:decisions:${statuses ? statuses.join(',') : 'every'}:${selection ? JSON.stringify(selection) : 'all'}`,
      entries: [
        pagedTailEntry(
          'main',
          ctx.track.source,
          ctx.branch.id,
          ctx.track.axisOffset,
          {
            product: 'decisions-tail',
            branchId: ctx.branch.id,
            from: boundFrom,
            limit: HEAD_LIMIT,
            ask: { ...(statuses ? { statuses } : {}), ...selection },
          },
          ctx.following,
          DEFAULT_TAIL_PAGE_SIZE,
        ),
      ],
    };
  },
  projection: decisionsProjection,
  projectionOptions(ctx): DecisionsProjectionOptions {
    return { artwork: ctx.options.artwork !== false, breaks: recordingBreaks(ctx.branch), hover: hoverTextOf(ctx.options) };
  },
};
