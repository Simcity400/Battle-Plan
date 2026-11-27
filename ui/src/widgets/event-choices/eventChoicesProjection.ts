import { unavailableArt } from '../../families/art/types.ts';
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
  occurrenceSelection,
  pagedTailEntry,
  projectPagedSlots,
  urlArtHandle,
} from '../../families/table/logHelpers.ts';
import type { TableCell, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { DEFAULT_TAIL_PAGE_SIZE, validateRowTailReply, type DatedLogRow, type TailReply } from '../../families/table/rowSource.ts';
import { TABLE_SETTINGS_KEY, tableSettingsOf } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { ScriptEventKind, ScriptPageRow } from '../../generated/dashboard';
import { gapRows, hasTooltipGaps, hoverTextOf, unreadRows, type TooltipTexts } from '../event-feed/hoverText.ts';
import { gapsForField, reasonsOf, scriptedValue } from '../event-feed/scriptedGaps.ts';
import { recordingBreaks, validateOccurrence, type RecordingBreak } from '../event-feed/scriptOccurrences.ts';
  import {
  awaitingText,
  choiceAt,
  choiceCard,
  choiceDates,
  choiceNotRecordedText,
  choiceText,
  FIRING_NOT_RECORDED,
  isOpen,
  noChoiceText,
  optionLetter,
  optionLines,
  type ChoiceAt,
} from './eventChoicesData.ts';
import { AWAITING, EVENT_CHOICES_COLUMNS, EVENT_KINDS } from './definition.ts';
import type { EventDetailSubject } from './EventDetailWindow.tsx';

export interface EventChoicesProjectionOptions {
  readonly byDate?: boolean;
  readonly search?: string;
  readonly events: readonly ScriptEventKind[];
  readonly awaiting: boolean;
  readonly artwork: boolean;
  readonly breaks: readonly RecordingBreak[];
  readonly hover?: TooltipTexts;
}

const HEAD_LIMIT = 256;
const nonBlank = (value: string | null | undefined): string | null => (value && value.trim().length > 0 ? value : null);

interface ChoicePageInfo {
  readonly open: boolean;
  readonly live: boolean;
  readonly notRecordedReason?: string;
  readonly notRecordedHour?: number;
}

function buildChoiceRow(
  readView: BoundedReadView,
  at: ChoiceAt,
  page: ChoicePageInfo,
  artwork: boolean,
  boundary: ReturnType<typeof newIdentityBoundary>,
  hover: TooltipTexts,
): TableRow {
  const { card } = at;
  const firstRow = card.first.row;
  const nameHour = firstRow.nameHour;
  const open = page.open;
  const reasons = reasonsOf(card.item);
  const lines = optionLines(at);
  const chosenLine = lines.find(line => line.chosen) ?? null;

  const artHandle = artwork ? urlArtHandle(card.item?.icon ?? null) : null;
  const art = artHandle ? readView.art(artHandle) : unavailableArt('The product drew no icon for this item');

  const countryEntries: { readonly tag: string | null; readonly missingLabel?: string }[] = [
    { tag: card.country, missingLabel: 'Country not identified' },
  ];
  if (card.sender) countryEntries.push({ tag: card.sender });

  const notRecordedText = page.notRecordedReason !== undefined
    ? choiceNotRecordedText(page.notRecordedReason)
    : at.pause
      ? noChoiceText(at)
      : 'Choice not recorded';

  let choiceCell: TableCell;
  if (open) {
    choiceCell = logStatusCell('Awaiting choice', 'waiting');
  } else if (at.choice) {
    const chosenText = chosenLine && !chosenLine.missing ? nonBlank(chosenLine.text) : null;
    if (chosenText !== null) {
      const optionGaps = chosenLine?.ordinal != null ? gapsForField(card.item?.gaps, 'option', chosenLine.ordinal) : [];
      choiceCell = {
        value: scriptedValue(chosenText, optionGaps),
        ...(chosenLine?.letter ? { inlinePrefix: chosenLine.letter } : {}),
      };
    } else if (chosenLine?.letter) {
      choiceCell = {
        value: notRecorded('Event presentation was not recorded'),
        inlinePrefix: chosenLine.letter,
      };
    } else {
      choiceCell = { value: notRecorded('Choice not recorded'), missingFigure: 'Choice' };
    }
  } else {
    choiceCell = {
      value: notRecorded(notRecordedText),
      missingFigure: 'Choice',
    };
  }

  const rawTitle = nonBlank(card.item?.name);
  const titleGaps = gapsForField(card.item?.gaps, 'title');
  const safeChoiceText = nonBlank(choiceText(card)) ?? 'Title not recorded';
  const countryFlag = card.country ? identityFlags(readView, 'country', [card.country], { atHour: nameHour, boundary })[0] : undefined;
  const countryName = card.country ? identityName(readView, 'country', card.country, nameHour) : null;
  const detailSubject: EventDetailSubject = {
    key: card.occurrence.id,
    hour: firstRow.hour,
    firedHour: card.fired?.row.hour ?? null,
    asOfHour: at.hour,
    title: rawTitle,
    countryTag: card.country,
    countryName,
    flagUrl: countryFlag?.art.status === 'ready' ? countryFlag.art.url : undefined,
    flagPending: countryFlag?.art.status === 'pending',
    item: card.item,
    chosenOrdinal: at.choice?.option ?? null,
    awaiting: open,
    chosenHour: at.choice ? (card.fired ? at.choice.row.hour : at.choice.row.readHour) : null,
    noChoiceReason: at.choice ? (at.choice.option == null ? 'Chosen option not recorded' : null) : (open ? 'No option chosen yet' : notRecordedText),
  };

  const timingBreakHour = page.notRecordedHour !== undefined
    ? page.notRecordedHour
    : page.notRecordedReason !== undefined
      ? null
      : undefined;

  const tooltipExtra = (): TooltipExtraRow[] => {
    const out: TooltipExtraRow[] = [
      { label: 'Timing', value: knownValue(choiceDates(at, timingBreakHour)) },
    ];
    if (open) out.push({ label: 'Status', value: knownValue(awaitingText(at)) });
    if (detailSubject.chosenHour !== null) out.push({ label: card.fired ? 'Chosen' : 'Chosen by', value: knownValue(formatDateTime(detailSubject.chosenHour)) });
    if (card.item?.token) out.push({ label: 'Content identity', value: knownValue(card.item.token) });
    if (reasons.nameMissing) out.push({ label: 'Title not recorded', value: notRecorded(reasons.nameMissing) });
    out.push(...gapRows(hover, card.occurrence.id, card.item?.gaps, optionLetter), ...unreadRows(hover, card.occurrence.id));
    return out;
  };

  return {
    key: card.occurrence.id,
    name: safeChoiceText,
    rowHour: card.fired ? firstRow.hour : firstRow.readHour,
    live: page.live,
    cells: {
      date: card.fired ? logDateCell(card.fired.row.hour) : {
        value: notRecorded(`${FIRING_NOT_RECORDED}. Chosen by ${formatDateTime(firstRow.readHour)}`),
      },
      kind: artwork
        ? {
            value: rawTitle !== null ? knownValue(safeChoiceText) : notRecorded(reasons.nameMissing ?? 'Event picture not recorded'),
            art,
            iconSlot: true,
            artShape: 'wide',
            artName: safeChoiceText,
          }
        : { value: notApplicable('Game artwork hidden'), blank: true },
      country: logCountryCell(readView, countryEntries, nameHour, boundary),
      item: rawTitle !== null
        ? { value: scriptedValue(rawTitle, titleGaps) }
        : { value: notRecorded(reasons.nameMissing ?? 'Event presentation was not recorded') },
      choice: choiceCell,
      action: { value: knownValue('View'), actionLabel: 'View' },
    },
    tooltipTitle: safeChoiceText,
    tooltipExtra,
    ...(hasTooltipGaps(hover, card.occurrence.id, card.item?.gaps) ? { tooltipScrollId: card.occurrence.id } : {}),
    tooltipAction: 'Click to view event',
    meta: detailSubject,
  };
}

function validateEventChoicesTail(raw: unknown, readAt?: number): TailReply<ScriptPageRow & DatedLogRow> {
  return validateRowTailReply(raw, (row, _columns, at): ScriptPageRow & DatedLogRow => {
    if (row.occurrence === null) throw new Error('Event choice was not recorded.');
    if (row.notRecordedReason !== undefined && typeof row.notRecordedReason !== 'string') {
      throw new Error('The occurrence card is malformed.');
    }
    if (row.notRecordedHour !== undefined) {
      if (typeof row.notRecordedHour !== 'number' || !Number.isInteger(row.notRecordedHour)) {
        throw new Error('The occurrence card is malformed.');
      }
      if (at !== undefined && row.notRecordedHour > at) {
        throw new Error(`Event choice break at hour ${row.notRecordedHour} is after displayed hour ${at}.`);
      }
    }
    const occurrence = validateOccurrence(row.occurrence, at);
    if (!choiceCard(occurrence)) throw new Error('The occurrence card is malformed.');
    return { ...(row as unknown as ScriptPageRow & DatedLogRow), occurrence };
  }, readAt);
}

export const eventChoicesProjection: TableProjection<EventChoicesProjectionOptions> = registerFamilyProjection({
  kind: 'event-choices',
  family: 'table',
  validateTail: validateEventChoicesTail,
  project(readView, options): TableModel {
    const hour = readView.hour;
    if (!options.events.length) {
      return {
        columns: EVENT_CHOICES_COLUMNS,
        rows: [],
        hour,
        searchable: true, search: options.search ?? '',
        dateGrouped: options.byDate !== false,
        empty: 'No recorded events match these settings.',
      };
    }
    const refused = readView.refused('main');
    if (refused !== undefined) {
      return { columns: EVENT_CHOICES_COLUMNS, rows: [], hour, searchable: true, search: options.search ?? '', dateGrouped: options.byDate !== false, refused };
    }
    const source = readView.rows<ScriptPageRow & DatedLogRow>('main');
    if (!source) {
      return { columns: EVENT_CHOICES_COLUMNS, rows: [], hour, searchable: true, search: options.search ?? '', dateGrouped: options.byDate !== false };
    }
    if (source.refused !== null) {
      return { columns: EVENT_CHOICES_COLUMNS, rows: [], hour, searchable: true, search: options.search ?? '', dateGrouped: options.byDate !== false, refused: source.refused };
    }
    const boundary = newIdentityBoundary();
    let nextAwaitingChange = Infinity;
    const readAt = source.at ?? hour;
    const { slots, rows, windowOffset, totalRows, gapBefore } = projectPagedSlots(
      source,
      readAt,
      readView.rowWindow,
      rawRow => {
        const resolved = choiceAt(choiceCard(rawRow.occurrence), readAt, options.breaks);
        if (!resolved) throw new Error('Event choice row was not validated by the tail decoder.');
        const open = rawRow.status != null ? rawRow.status === 'awaiting' : isOpen(resolved);
        const live = typeof rawRow.live === 'boolean' ? rawRow.live : open;
        const page: ChoicePageInfo = {
          open,
          live,
          ...(rawRow.notRecordedReason !== undefined ? { notRecordedReason: rawRow.notRecordedReason } : {}),
          ...(rawRow.notRecordedHour !== undefined ? { notRecordedHour: rawRow.notRecordedHour } : {}),
        };
        if (open && !source.frozen) {
          const nextDay = (Math.floor(readAt / 24) + 1) * 24;
          if (nextDay < nextAwaitingChange) nextAwaitingChange = nextDay;
        }
        return buildChoiceRow(readView, resolved, page, options.artwork, boundary, options.hover ?? []);
      },
    );

    const awaitingCount = source.facetTotals?.get('awaiting') ?? source.total;
    const titleSummary = options.awaiting
      ? (awaitingCount > 0 ? `${formatScalar(awaitingCount, 'integer')} awaiting` : null)
      : (source.total > 0 ? `${formatScalar(source.total, 'integer')} ${source.total === 1 ? 'event' : 'events'}` : null);
    const rawHead = source.rawHead as { readonly warnings?: readonly string[]; readonly unavailable?: readonly { readonly label: string; readonly reason: string }[] } | undefined;
    const warnings = [
      ...(rawHead?.warnings ?? []),
      ...(rawHead?.unavailable ?? []).map(u => `${u.label}: ${u.reason}`),
    ];

    const hasMissing = source.missing.some(span => span.firstHour <= readAt);

    return {
      columns: EVENT_CHOICES_COLUMNS,
      rows,
      slots,
      hour,
      searchable: true, search: options.search ?? '',
      dateGrouped: options.byDate !== false,
      windowOffset,
      totalRows,
      ...(gapBefore?.length ? { gapBefore } : {}),
      nextChangeAt: Math.min(boundary.next, nextAwaitingChange),
      rowTimeBoundaries: false,
      titleSummary,
      ...(hasMissing && titleSummary ? { titleSummaryPartial: 'Count from recorded hours' } : {}),
      ...(warnings.length ? { warnings } : {}),
      ...(slots.length === 0 ? { empty: 'No recorded events match these settings.' } : {}),
    };
  },
});

export const eventChoicesDeclaration: TableWidgetDeclaration<EventChoicesProjectionOptions> = {
  family: 'table',
  kind: 'event-choices',
  label: 'Event choices',
  subject: 'event choices',
  emptyCampaign: 'Load a campaign to see event choices.',
  icon: 'ballot',
  countries: { mode: 'filter', content: 'history' },
  columns: EVENT_CHOICES_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx): SourcePlan {
    const { sort } = tableSettingsOf(ctx.options[TABLE_SETTINGS_KEY], EVENT_CHOICES_COLUMNS, LOG_TABLE_BASE);
    const search = typeof ctx.options.search === 'string' ? ctx.options.search : '';
    const events = EVENT_KINDS.filter(kind => ctx.options[kind.value] !== false).map(kind => kind.event);
    if (!events.length) {
      return { owner: `${ctx.track.source.id}:${ctx.branch.id}:event-choices:empty`, entries: [], allowEmptyFollow: true };
    }
    const win = !ctx.following ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const rawFrom = ctx.widget.time.mode === 'range' && win ? Math.max(Math.floor(win.firstHour), ctx.branch.firstHour) : null;
    const boundFrom = rawFrom !== null ? ctx.gateSpan(rawFrom, ctx.branch.lastHour)?.firstHour ?? null : null;
    const awaiting = ctx.options[AWAITING.value] === true;
    const selection = occurrenceSelection(ctx);
    return {
      owner: `${ctx.track.source.id}:${ctx.branch.id}:event-choices:${events.join(',')}:${awaiting}:${selection ? JSON.stringify(selection) : 'all'}:${JSON.stringify([search, sort])}`,
      loadingLabel: search.trim() || sort.column === 'item' || sort.column === 'choice' ? 'Preparing titles…' : undefined,
      entries: [
        pagedTailEntry(
          'main',
          ctx.track.source,
          ctx.branch.id,
          ctx.track.axisOffset,
          {
            product: 'event-choices-tail',
            branchId: ctx.branch.id,
            from: boundFrom,
            limit: HEAD_LIMIT,
            ask: { search, sort,
              ...(events.length === EVENT_KINDS.length ? {} : { events }),
              ...(awaiting ? { statuses: ['awaiting'] } : {}),
              ...selection,
            },
          },
          ctx.following,
          DEFAULT_TAIL_PAGE_SIZE,
          validateEventChoicesTail,
        ),
      ],
    };
  },
  projection: eventChoicesProjection,
  projectionOptions(ctx): EventChoicesProjectionOptions {
    const events = EVENT_KINDS.filter(kind => ctx.options[kind.value] !== false).map(kind => kind.event);
    return { byDate: ctx.settings.sort.column === 'date', search: typeof ctx.options.search === 'string' ? ctx.options.search : '', events, awaiting: ctx.options[AWAITING.value] === true, artwork: ctx.options.artwork !== false, breaks: recordingBreaks(ctx.branch), hover: hoverTextOf(ctx.options) };
  },
};
