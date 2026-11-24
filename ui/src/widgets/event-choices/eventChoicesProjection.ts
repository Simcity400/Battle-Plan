import { formatDateTime } from '../../dashboard/calendar.ts';
import { resolveWindow } from '../../dashboard/model.ts';
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
import { registerTableProjection } from '../../families/table/registry.ts';
import { DEFAULT_TAIL_PAGE_SIZE, type DatedLogRow } from '../../families/table/rowSource.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import type { EventFeedKind, EventFeedSpanReply, MembershipRun, ScriptOccurrence, ScriptPageRow } from '../../generated/dashboard';
import { kindsRequest, validateSpan } from '../event-feed/eventFeedData.ts';
import { reasonsOf } from '../event-feed/scriptedGaps.ts';
import { cardFacets, recordingBreaks, validateOccurrence, type RecordingBreak } from '../event-feed/scriptOccurrences.ts';
import {
  awaitingText,
  choiceAt,
  choiceCard,
  choiceDates,
  choiceText,
  isOpen,
  noChoiceText,
  optionLines,
  type ChoiceAt,
} from './eventChoicesData.ts';
import { AWAITING, EVENT_CHOICES_COLUMNS, EVENT_KINDS } from './definition.ts';
import type { EventDetailSubject } from './EventDetailWindow.tsx';

export interface EventChoicesProjectionOptions {
  readonly openers: readonly EventFeedKind[];
  readonly awaiting: boolean;
  readonly artwork: boolean;
  readonly breaks: readonly RecordingBreak[];
  readonly filter: {
    readonly passing: Readonly<Record<string, readonly (readonly [number, number])[]>> | null;
  };
}

const HEAD_LIMIT = 256;
const CONTINUING: readonly EventFeedKind[] = ['eventOptionSelected'];

function buildChoiceRow(
  readView: BoundedReadView,
  at: ChoiceAt,
  artwork: boolean,
  boundary: ReturnType<typeof newIdentityBoundary>,
): TableRow {
  const { card } = at;
  const firstRow = card.first.row;
  const nameHour = firstRow.nameHour;
  const open = isOpen(at);
  const reasons = reasonsOf(card.item);
  const lines = optionLines(at);
  const chosenLine = lines.find(line => line.chosen) ?? null;

  const artHandle = artwork ? urlArtHandle(card.item?.icon ?? null) : null;
  const artResult = artHandle ? readView.art(artHandle) : null;
  const icon = artResult?.status === 'ready' ? artResult.url : undefined;
  const iconPending = artResult?.status === 'pending' ? true : undefined;

  const countryEntries: { readonly tag: string | null; readonly missingLabel?: string }[] = [
    { tag: card.country, missingLabel: 'Country not identified' },
  ];
  if (card.sender) countryEntries.push({ tag: card.sender });

  let choiceCell: TableCell;
  if (open) {
    choiceCell = logStatusCell('Awaiting choice', 'waiting');
  } else if (at.choice) {
    if (chosenLine && !chosenLine.missing) {
      choiceCell = {
        value: knownValue(chosenLine.text),
        ...(chosenLine.letter ? { inlinePrefix: chosenLine.letter } : {}),
      };
    } else if (chosenLine?.letter) {
      choiceCell = {
        value: notRecorded('Event presentation was not recorded'),
        inlinePrefix: chosenLine.letter,
      };
    } else {
      choiceCell = { value: notRecorded('Choice not recorded') };
    }
  } else {
    choiceCell = { value: notRecorded(at.pause ? 'Choice not recorded (recording paused)' : 'Choice not recorded') };
  }

  const rawTitle = card.item?.name ?? null;
  const countryFlag = card.country ? identityFlags(readView, 'country', [card.country], { atHour: nameHour, boundary })[0] : undefined;
  const countryName = card.country ? identityName(readView, 'country', card.country, nameHour) : null;
  const detailSubject: EventDetailSubject = {
    key: card.occurrence.id,
    hour: firstRow.hour,
    title: rawTitle,
    countryTag: card.country,
    countryName,
    flagUrl: countryFlag?.icon,
    flagPending: countryFlag?.pending,
    item: card.item,
    chosenOrdinal: at.choice?.option ?? null,
    awaiting: open,
    chosenHour: at.choice?.row.hour ?? null,
    noChoiceReason: at.choice ? null : noChoiceText(at),
  };

  const tooltipExtra = (): TooltipExtraRow[] => {
    const out: TooltipExtraRow[] = [
      { label: 'Timing', value: knownValue(choiceDates(at)) },
    ];
    if (open) out.push({ label: 'Status', value: knownValue(awaitingText(at)) });
    if (at.choice) out.push({ label: 'Chosen', value: knownValue(formatDateTime(at.choice.row.hour)) });
    if (card.item?.token) out.push({ label: 'Content identity', value: knownValue(card.item.token) });
    if (reasons.nameMissing) out.push({ label: 'Title not recorded', value: notRecorded(reasons.nameMissing) });
    if (reasons.gaps) out.push({ label: 'Text not recorded', value: notRecorded(reasons.gaps) });
    return out;
  };

  return {
    key: card.occurrence.id,
    name: choiceText(card),
    rowHour: firstRow.hour,
    live: open,
    cells: {
      date: logDateCell(firstRow.hour),
      kind: artwork
        ? {
            value: rawTitle !== null ? knownValue(choiceText(card)) : notRecorded(reasons.nameMissing ?? 'Event picture not recorded'),
            icon,
            iconPending,
            iconSlot: true,
            artShape: 'wide',
            artName: choiceText(card),
          }
        : { value: notApplicable('Game artwork hidden'), blank: true },
      country: logCountryCell(readView, countryEntries, nameHour, boundary),
      item: rawTitle !== null
        ? { value: knownValue(rawTitle) }
        : { value: notRecorded(reasons.nameMissing ?? 'Event presentation was not recorded') },
      choice: choiceCell,
      action: { value: knownValue('View'), actionLabel: 'View' },
    },
    tooltipTitle: choiceText(card),
    tooltipExtra,
    tooltipAction: 'Click to view event',
    meta: detailSubject,
  };
}

export const eventChoicesProjection: TableProjection<EventChoicesProjectionOptions> = registerTableProjection({
  kind: 'event-choices',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    if (!options.openers.length) {
      return {
        columns: EVENT_CHOICES_COLUMNS,
        rows: [],
        hour,
        hideHeader: true,
        unsorted: true,
        dateGrouped: true,
        rowHeightMode: 'media',
        empty: 'No recorded events match these settings.',
      };
    }
    const refused = readView.refused('main');
    if (refused !== undefined) {
      return { columns: EVENT_CHOICES_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, rowHeightMode: 'media', refused };
    }
    const source = readView.rows<ScriptPageRow & DatedLogRow>('main');
    if (!source) {
      return { columns: EVENT_CHOICES_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, rowHeightMode: 'media' };
    }
    if (source.refused !== null) {
      return { columns: EVENT_CHOICES_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, rowHeightMode: 'media', refused: source.refused };
    }
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
        const card = choiceCard(occ);
        const resolved = choiceAt(
          { key: rawRow.key, id: occ.id, hour: card?.first.row.hour ?? rawRow.hour, revealedHour: card?.first.row.readHour ?? rawRow.hour, by: false, item: card },
          hour,
          options.breaks,
          options.awaiting,
          countryFn,
        );
        if (!resolved) return null;
        return buildChoiceRow(readView, resolved.item, options.artwork, boundary);
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
    const rawHead = source.rawHead as { readonly warnings?: readonly string[]; readonly unavailable?: readonly { readonly label: string; readonly reason: string }[] } | undefined;
    const warnings = [
      ...(rawHead?.warnings ?? []),
      ...(rawHead?.unavailable ?? []).map(u => `${u.label}: ${u.reason}`),
    ];

    return {
      columns: EVENT_CHOICES_COLUMNS,
      rows: orderedRows,
      slots: orderedSlots,
      hour,
      hideHeader: true,
      unsorted: true,
      dateGrouped: true,
      rowHeightMode: 'media',
      windowOffset,
      totalRows,
      nextChangeAt: boundary.next,
      rowTimeBoundaries: false,
      ...(warnings.length ? { warnings } : {}),
      ...(orderedSlots.length === 0 ? { empty: 'No recorded events match these settings.' } : {}),
    };
  },
});

export const eventChoicesDeclaration: TableWidgetDeclaration<EventChoicesProjectionOptions, EventFeedSpanReply> = {
  family: 'table',
  kind: 'event-choices',
  label: 'Event choices',
  subject: 'event choices',
  emptyCampaign: 'Load a campaign to see event choices.',
  icon: 'ballot',
  countries: { mode: 'filter', labels: true },
  aux: {
    product: 'event-feed-span',
    params: track => kindsRequest(track).params as Record<string, unknown>,
    validate: validateSpan,
  },
  columns: EVENT_CHOICES_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx): SourcePlan {
    const openers = EVENT_KINDS.filter(kind => ctx.options[kind.value] !== false).map(kind => kind.kind);
    if (!openers.length) {
      return { owner: `${ctx.track.source.id}:${ctx.branch.id}:event-choices:empty`, entries: [], allowEmptyFollow: true };
    }
    const kinds = ctx.aux?.kinds;
    if (!kinds) return { owner: `${ctx.track.source.id}:${ctx.branch.id}:event-choices:loading`, entries: [] };
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
    const facets = cardFacets(kinds, openers, CONTINUING);
    return {
      owner: `${ctx.track.source.id}:${ctx.branch.id}:event-choices:${openers.join(',')}:${scopeRuns ? scopeRuns.length : 'all'}`,
      entries: [
        pagedTailEntry(
          'main',
          ctx.track.source,
          ctx.branch.id,
          ctx.track.axisOffset,
          {
            product: 'event-choices-tail',
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
  projection: eventChoicesProjection,
  projectionOptions(ctx): EventChoicesProjectionOptions {
    const openers = EVENT_KINDS.filter(kind => ctx.options[kind.value] !== false).map(kind => kind.kind);
    return {
      openers,
      awaiting: ctx.options[AWAITING.value] === true,
      artwork: ctx.options.artwork !== false,
      breaks: recordingBreaks(ctx.branch),
      filter: { passing: ctx.countries.filter.passing },
    };
  },
};
