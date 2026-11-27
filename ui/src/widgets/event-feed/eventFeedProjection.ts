import { unavailableArt } from '../../families/art/types.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import { contentSet } from '../../dashboard/options.ts';
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
import { formatRecordedPercent, formatScalar } from '../../families/valueFormat.ts';
import { EVENT_FEED_ROW_LABELS, type EventFeedFoldRow, type EventFeedItem, type EventFeedLine, type EventFeedMoreRow, type EventFeedPageRow, type EventFeedRow, type ScriptEdge, type ScriptStatus } from '../../generated/dashboard.ts';
import { choiceNotRecordedText, optionLetter } from '../event-choices/eventChoicesData.ts';
import type { EventDetailSubject } from '../event-choices/EventDetailWindow.tsx';
import { CATEGORIES, EVENT_FEED_COLUMNS } from './definition.ts';
import { effectiveEventItem, feedKindLabel, validateFeedRow } from './eventFeedData.ts';
import { descriptionRows, gapRows, hasTooltipGaps, hoverTextOf, optionLines, optionRows, type TooltipTexts } from './hoverText.ts';
import { gapsForField, plainScripted, reasonsOf, scriptedValue } from './scriptedGaps.ts';
import { validateScriptEdge } from './scriptOccurrences.ts';

export interface EventFeedProjectionOptions {
  readonly byDate?: boolean;
  readonly search?: string;
  readonly categories: readonly string[];
  readonly artwork: boolean;
  readonly hover?: TooltipTexts;
  readonly expanded?: readonly string[];
}

const HEAD_LIMIT = 256;
const GROUP = 'group:';

const LOSER_SECOND_KINDS = new Set(['annexed', 'warDeclared', 'warStarted']);
const ONE_COUNTRY_KINDS = new Set(['ceasedToExist', 'projectFinished', 'factionJoined', 'factionLeft']);
const VIEWABLE_KINDS = new Set(['scriptEventFired', 'newsEventFired', 'eventOptionSelected']);

const KIND_SPRITES: Readonly<Record<string, string>> = {
  warDeclared: 'GFX_diplo_action_declare_war_bg',
  warStarted: 'GFX_diplo_action_declare_war_bg',
  warJoined: 'GFX_diplo_action_join_allies_bg',
  civilWarWarTransferred: 'GFX_diplo_action_join_allies_bg',
  warEnded: 'GFX_diplo_action_offer_peace_bg',
  peace: 'GFX_diplo_action_offer_peace_bg',
  peaceConferenceStarted: 'GFX_diplo_action_offer_peace_bg',
  factionJoined: 'GFX_diplo_action_invite_to_faction_bg',
  factionLeft: 'GFX_diplo_action_dissolve_faction_bg',
  guaranteeGranted: 'GFX_diplo_action_guarantee_independence_bg',
  capitulated: 'GFX_surrender_icon',
  ceasedToExist: 'GFX_surrender_icon',
  annexed: 'GFX_diplo_action_justify_wargoals_bg',
  released: 'GFX_diplo_action_justify_wargoals_bg',
  stateTransferred: 'GFX_diplo_action_justify_wargoals_bg',
  becameSubject: 'GFX_relation_puppet_receiver',
  nuclearStrike: 'GFX_topbar_nuke',
  nukesIncreased: 'GFX_topbar_nuke',
};

interface FeedPageInfo {
  readonly choice: ScriptEdge | null;
  readonly status?: ScriptStatus;
  readonly notRecordedReason?: string;
  readonly live: boolean;
}

function parsedOptionOrdinal(event: EventFeedRow, choice: ScriptEdge | null): number | null {
  if (choice && typeof choice.option === 'number' && Number.isInteger(choice.option)) return choice.option;
  if (event.kind === 'eventOptionSelected' && typeof event.cause?.selectedOption === 'number') return event.cause.selectedOption;
  return null;
}

const nonBlank = (value: string | null | undefined): string | null => (value && value.trim().length > 0 ? value : null);

function optionCellOf(item: EventFeedItem | null, ordinal: number): TableCell {
  const options = item?.options ?? [];
  const index = options.findIndex(opt => opt.ordinal === ordinal);
  if (index >= 0) {
    const opt = options[index]!;
    const letter = optionLetter(index);
    const text = nonBlank(opt.name);
    if (text !== null) return { value: scriptedValue(text, gapsForField(item?.gaps, 'option', ordinal)), inlinePrefix: letter };
    return { value: notRecorded('Event presentation was not recorded'), inlinePrefix: letter };
  }
  if (ordinal >= 0) {
    return { value: notRecorded('Event presentation was not recorded'), inlinePrefix: optionLetter(ordinal) };
  }
  return { value: notRecorded('Choice not recorded'), missingFigure: 'Choice' };
}

function scriptEventDetailCell(item: EventFeedItem | null, chosenOrdinal: number | null, page: FeedPageInfo): TableCell {
  if (page.choice) {
    return chosenOrdinal !== null && chosenOrdinal >= 0
      ? optionCellOf(item, chosenOrdinal)
      : { value: notRecorded('Choice not recorded'), missingFigure: 'Choice' };
  }
  if (page.status === 'awaiting' || page.live) {
    return logStatusCell('Awaiting choice', 'waiting');
  }
  if (page.status === 'notRecorded') {
    const reason = page.notRecordedReason !== undefined ? choiceNotRecordedText(page.notRecordedReason) : 'Choice not recorded';
    return { value: notRecorded(reason), missingFigure: 'Choice' };
  }
  return { value: knownValue('Fired') };
}

function eventItemAndSubtext(
  event: EventFeedRow,
  page: FeedPageInfo,
  readView: BoundedReadView,
): { readonly primary: string | null; readonly secondary?: string; readonly detail?: TableCell; readonly missingReason?: string } {
  const item = effectiveEventItem(event, page.choice);
  const reasons = reasonsOf(item);
  const named = nonBlank(item?.name);
  const label = EVENT_FEED_ROW_LABELS[event.kind] ?? null;
  const chosenOrdinal = parsedOptionOrdinal(event, page.choice);
  const stateLabel = (stateId: number | null | undefined): string | undefined => {
    if (stateId == null) return undefined;
    const res = readView.who('state', String(stateId), event.hour);
    return res.name.status === 'ready' ? res.name.value : `State ${stateId}`;
  };

  switch (event.kind) {
    case 'scriptEventFired':
      return {
        primary: named,
        missingReason: reasons.nameMissing ?? 'Event presentation was not recorded',
        detail: scriptEventDetailCell(item, chosenOrdinal, page),
      };
    case 'newsEventFired':
      return {
        primary: named,
        secondary: 'News',
        missingReason: reasons.nameMissing ?? 'Event presentation was not recorded',
        detail: scriptEventDetailCell(item, chosenOrdinal, page),
      };
    case 'eventOptionSelected':
      return {
        primary: named,
        missingReason: reasons.nameMissing ?? 'Event presentation was not recorded',
        detail: chosenOrdinal !== null && chosenOrdinal >= 0
          ? optionCellOf(item, chosenOrdinal)
          : { value: notRecorded('Choice not recorded'), missingFigure: 'Choice' },
      };
    case 'eventWindowPresented':
      return {
        primary: named,
        secondary: event.window?.presented?.shown === false ? 'Window (hidden)' : 'Window',
        missingReason: reasons.nameMissing ?? 'Event presentation was not recorded',
      };
    case 'decisionSelected':
      return { primary: named ?? label, secondary: named ? 'Decision' : undefined, missingReason: reasons.nameMissing };
    case 'decisionActivated':
      return { primary: named ?? label, secondary: named ? 'Mission' : undefined, missingReason: reasons.nameMissing };
    case 'decisionCompleted':
      return {
        primary: named ?? label,
        secondary: event.decisionState?.kind === 'failed' ? 'Failed' : 'Completed',
        missingReason: reasons.nameMissing,
      };
    case 'decisionTimedOut':
      return { primary: named ?? label, secondary: event.decisionState?.kind === 'failed' ? 'Failed' : 'Timed out', missingReason: reasons.nameMissing };
    case 'decisionCancelled':
      return { primary: named ?? label, secondary: event.decisionState?.kind === 'failed' ? 'Failed' : 'Cancelled', missingReason: reasons.nameMissing };
    case 'decisionRemoved':
      return { primary: named ?? label, secondary: event.decisionState?.kind === 'failed' ? 'Failed' : 'Removed', missingReason: reasons.nameMissing };
    case 'decisionReenabled':
      return { primary: named ?? label, secondary: 'Reenabled' };
    case 'decisionDurationChanged':
      return { primary: named ?? label, secondary: label ?? undefined };
    case 'decisionRemovalEffect':
    case 'decisionCancellationEffect':
    case 'decisionCompletionEffect':
    case 'decisionTimeoutEffect':
      return { primary: named ?? label, secondary: 'Effect' };
    case 'warStarted':
    case 'warDeclared':
    case 'warEnded':
    case 'peace':
    case 'annexed':
    case 'released':
      return { primary: label, secondary: event.cause?.how ?? undefined };
    case 'warJoined':
      return { primary: label, secondary: event.cause?.reason ?? event.cause?.how ?? undefined };
    case 'civilWarWarTransferred':
    case 'peaceConferenceStarted':
    case 'capitulated':
    case 'ceasedToExist':
    case 'guaranteeGranted':
      return { primary: label };
    case 'stateTransferred':
      return { primary: label, secondary: stateLabel(event.cause?.state) };
    case 'factionJoined':
    case 'factionLeft':
      return {
        primary: label,
        ...(named ? { secondary: named } : {}),
        ...(named === null ? { detail: { value: notRecorded(reasons.nameMissing ?? 'Faction name not recorded at this hour') } } : {}),
      };
    case 'becameSubject':
    case 'rulingPartyChanged':
    case 'lawChanged':
    case 'advisorHired':
    case 'focusCompleted':
    case 'projectFinished':
      return { primary: label, secondary: named ?? undefined };
    case 'nukesIncreased':
      return { primary: label, secondary: event.amount ? `+${event.amount}` : undefined };
    case 'nuclearStrike': {
      const targetState = stateLabel(event.strike?.state);
      const losses: string[] = [];
      if (event.strike?.manpowerLost != null) losses.push(`${formatScalar(event.strike.manpowerLost, 'integer')} casualties`);
      if (event.strike?.buildingLevels != null) losses.push(`${formatScalar(event.strike.buildingLevels, 'decimal1')} building levels`);
      if (event.strike?.warSupport != null) losses.push(`${formatRecordedPercent(event.strike.warSupport, { signed: true })} war support`);
      if (event.strike?.units != null) losses.push(`${formatScalar(event.strike.units, 'integer')} ${event.strike.units === 1 ? 'unit' : 'units'}`);
      if (event.strike?.equipmentLost != null) losses.push(`${formatScalar(event.strike.equipmentLost, 'integer')} equipment`);
      const detailCell: TableCell | undefined = losses.length
        ? { value: knownValue(losses.join(' · ')) }
        : event.strike?.notRecorded.length
          ? { value: notRecorded(event.strike.notRecorded.join('; ')) }
          : undefined;
      return {
        primary: label,
        secondary: targetState,
        detail: detailCell,
      };
    }
  }
}

function buildFeedTableRow(
  readView: BoundedReadView,
  event: EventFeedRow,
  page: FeedPageInfo,
  key: string,
  artwork: boolean,
  displayedHour: number,
  boundary: ReturnType<typeof newIdentityBoundary>,
  hover: TooltipTexts,
  parent?: string,
): TableRow {
  const item = effectiveEventItem(event, page.choice);
  const { primary, secondary, detail, missingReason } = eventItemAndSubtext(event, page, readView);
  const reasons = reasonsOf(item);
  const spriteKey = !item?.icon ? KIND_SPRITES[event.kind] : undefined;
  const artHandle = artwork
    ? (urlArtHandle(item?.icon ?? null) ?? (spriteKey ? { kind: 'sprite' as const, key: spriteKey, frame: 0 } : null))
    : null;
  const art = artHandle ? readView.art(artHandle) : unavailableArt('The product drew no icon for this item');

  const rawRoles = event.participants ?? event.tags;
  const countryEntries: { readonly tag: string | null; readonly dimmed?: boolean; readonly missingLabel?: string }[] = [];
  for (let idx = 0; idx < rawRoles.length; idx++) {
    const tag = rawRoles[idx] ?? null;
    if (tag === null && event.participants === undefined) continue;
    if (idx > 0 && ONE_COUNTRY_KINDS.has(event.kind)) continue;
    if (idx > 0 && tag !== null && tag === rawRoles[0]) continue;
    if (tag === null && idx === 1) {
      if (event.kind.startsWith('decision')) {
        if (event.cause?.target?.kind !== 'country') continue;
      } else if (rawRoles[0] !== null && !['scriptEventFired', 'newsEventFired', 'warStarted', 'warDeclared', 'warJoined', 'peace', 'annexed', 'released', 'guaranteeGranted'].includes(event.kind)) {
        continue;
      }
    }
    countryEntries.push({
      tag,
      dimmed: idx === 1 && LOSER_SECOND_KINDS.has(event.kind) || (idx === 0 && (event.kind === 'capitulated' || event.kind === 'ceasedToExist')),
      missingLabel: event.participantsMissing?.[idx] ?? (idx === 0 && event.ownerMissing ? event.ownerMissing : 'Country not identified'),
    });
  }
  if (!countryEntries.length && event.tags.length) {
    for (const tag of event.tags) countryEntries.push({ tag });
  }

  const viewable = VIEWABLE_KINDS.has(event.kind);
  const chosenOrdinal = parsedOptionOrdinal(event, page.choice);
  const chosenHour = page.choice?.row.hour ?? (event.kind === 'eventOptionSelected' ? event.readHour : null);
  let detailSubject: EventDetailSubject | undefined;
  if (viewable) {
    const recipient = event.kind === 'eventOptionSelected'
      ? (event.participants ? event.participants[0] : (event.tags[0] ?? null))
      : (event.participants ? event.participants[1] : (event.tags[0] ?? null));
    const flag = recipient ? identityFlags(readView, 'country', [recipient], { atHour: event.nameHour, boundary })[0] : undefined;
    const countryName = recipient ? identityName(readView, 'country', recipient, event.nameHour) : null;
    const awaiting = !page.choice && (page.status === 'awaiting' || page.live);
    const noChoiceReason = chosenOrdinal !== null
      ? null
      : chosenHour !== null
        ? 'Chosen option not recorded'
        : awaiting
          ? 'No option chosen yet'
          : page.status === 'notRecorded'
            ? (page.notRecordedReason !== undefined ? choiceNotRecordedText(page.notRecordedReason) : 'Choice not recorded')
            : 'Event fired';
    detailSubject = {
      key,
      hour: event.hour,
      firedHour: event.kind === 'eventOptionSelected' ? null : event.hour,
      asOfHour: displayedHour,
      title: item?.name ?? null,
      countryTag: recipient,
      countryName,
      flagUrl: flag?.art.status === 'ready' ? flag.art.url : undefined,
      flagPending: flag?.art.status === 'pending',
      item,
      chosenOrdinal,
      awaiting,
      chosenHour,
      noChoiceReason,
    };
  }

  const titleGaps = gapsForField(item?.gaps, 'title');
  const titleLabel = primary !== null ? plainScripted(primary, titleGaps) : (missingReason ?? 'Not recorded');
  const tooltipExtra = (): TooltipExtraRow[] => {
    const out: TooltipExtraRow[] = [
      { label: event.exact ? 'Time' : 'Observed at', value: knownValue(formatDateTime(event.hour)) },
    ];
    if (secondary) out.push({ label: 'Detail', value: knownValue(plainScripted(secondary)) });
    if (event.cause?.how) out.push({ label: 'Cause', value: knownValue(event.cause.how) });
    if (event.strike) {
      if (event.strike.manpowerLost != null) out.push({ label: 'Casualties', value: knownValue(event.strike.manpowerLost), format: 'integer' });
      if (event.strike.buildingLevels != null) out.push({ label: 'Building levels', value: knownValue(event.strike.buildingLevels), format: 'decimal1' });
      if (event.strike.warSupport != null) out.push({ label: 'War support', value: knownValue(event.strike.warSupport), format: { decimals: 0, percent: true, signed: true } });
      if (event.strike.units != null) out.push({ label: 'Units', value: knownValue(event.strike.units), format: 'integer' });
      if (event.strike.equipmentLost != null) out.push({ label: 'Equipment lost', value: knownValue(event.strike.equipmentLost), format: 'integer' });
      for (const reason of event.strike.notRecorded) {
        out.push({ label: 'Effect not recorded', value: notRecorded(reason) });
      }
    }
    if (item?.token) out.push({ label: 'Content identity', value: knownValue(item.token) });
    if (reasons.nameMissing) out.push({ label: 'Name not recorded', value: notRecorded(reasons.nameMissing) });
    out.push(...gapRows(hover, key, item?.gaps, optionLetter));
    out.push(...descriptionRows(hover, key, item?.description, item?.gaps));
    const choiceOptions = !event.item?.options?.length ? page.choice?.row.item?.options : undefined;
    out.push(...(choiceOptions?.length
      ? optionLines(
          choiceOptions.map(option => option.name),
          optionLetter,
          item?.gaps,
          choiceOptions.map(option => option.ordinal),
        )
      : optionRows(hover, key, optionLetter, item?.gaps)));
    return out;
  };

  return {
    key,
    name: titleLabel,
    rowHour: event.hour,
    ...(parent ? { pageParent: `${GROUP}${parent}` } : {}),
    cells: {
      date: logDateCell(event.hour),
      kind: artwork
        ? {
            value: knownValue(titleLabel),
            art,
            iconSlot: true,
            artShape: viewable ? 'wide' : 'square',
            artName: titleLabel,
          }
        : { value: notApplicable('Game artwork hidden'), blank: true },
      country: logCountryCell(readView, countryEntries, event.nameHour, boundary),
      item: primary !== null
        ? { value: scriptedValue(primary, titleGaps), ...(secondary ? { inlineSubtext: secondary } : {}) }
        : { value: notRecorded(missingReason ?? 'Not recorded') },
      detail: detail ?? { value: notApplicable('No extra detail'), blank: true },
      action: viewable ? { value: knownValue('View'), actionLabel: 'View' } : { value: notApplicable('No event window for this entry'), blank: true },
    },
    tooltipTitle: titleLabel,
    tooltipExtra,
    ...(hasTooltipGaps(hover, key, item?.gaps) ? { tooltipScrollId: key } : {}),
    ...(viewable ? { tooltipAction: 'Click to view event', meta: detailSubject } : {}),
  };
}

type ValidatedEventFeedEvent = Omit<EventFeedPageRow, 'row'> & { readonly line: 'event'; readonly row: EventFeedRow } & DatedLogRow;
type ValidatedEventFeedFold = Omit<EventFeedFoldRow, 'firstRow'> & { readonly line: 'fold'; readonly firstRow: EventFeedRow } & DatedLogRow;
type ValidatedEventFeedMore = EventFeedMoreRow & { readonly line: 'more' } & DatedLogRow;
type ValidatedEventFeedLine = ValidatedEventFeedEvent | ValidatedEventFeedFold | ValidatedEventFeedMore;

function buildFoldRow(
  readView: BoundedReadView,
  fold: ValidatedEventFeedFold,
  artwork: boolean,
  boundary: ReturnType<typeof newIdentityBoundary>,
  expanded: readonly string[],
): TableRow {
  const count = Number(fold.count);
  const label = `${formatScalar(count, 'integer')} events`;
  const kindLabel = feedKindLabel(fold.kind);
  const groupKey = `${GROUP}${fold.key}`;
  const open = expanded.includes(groupKey);
  const first = fold.firstRow;
  const spriteKey = !first.item?.icon ? KIND_SPRITES[fold.kind] : undefined;
  const artHandle = artwork
    ? (urlArtHandle(first.item?.icon ?? null) ?? (spriteKey ? { kind: 'sprite' as const, key: spriteKey, frame: 0 } : null))
    : null;
  const art = artHandle ? readView.art(artHandle) : unavailableArt('The product drew no icon for this item');
  return {
    key: groupKey,
    name: label,
    rowHour: fold.hour,
    pageGroup: { members: count, open },
    cells: {
      date: logDateCell(fold.hour),
      kind: artwork
        ? {
            value: knownValue(kindLabel),
            art,
            iconSlot: true,
            artShape: VIEWABLE_KINDS.has(fold.kind) ? 'wide' : 'square',
            artName: kindLabel,
          }
        : { value: knownValue(kindLabel), blank: true },
      country: logCountryCell(readView, [{ tag: fold.country }], first.nameHour, boundary),
      item: { value: knownValue(label) },
      detail: { value: notApplicable('Collapsed group'), blank: true },
      action: { value: notApplicable('Expand group'), blank: true },
    },
    tooltipTitle: label,
    tooltipAction: open ? 'Click to collapse' : 'Click to expand',
  };
}

function buildMoreRow(more: ValidatedEventFeedMore): TableRow {
  const blank: TableCell = { value: notApplicable('Not applicable to this fold line'), blank: true };
  const remaining = Number(more.remaining);
  const name = remaining > 0
    ? `Show all ${formatScalar(Number(more.total), 'integer')} events \u00b7 ${formatScalar(remaining, 'integer')} more`
    : 'Show fewer events';
  return {
    key: `all:${GROUP}${more.parent}`,
    name,
    rowHour: more.hour,
    muted: true,
    pageParent: `${GROUP}${more.parent}`,
    cells: { date: blank, kind: blank, country: blank, item: { value: knownValue(name) }, detail: blank, action: blank },
  };
}

function validateEventFeedTail(raw: unknown, readAt?: number): TailReply<ValidatedEventFeedLine> {
  return validateRowTailReply(raw, (row, _columns, at): ValidatedEventFeedLine => {
    if (row.line === 'event') {
      if (row.row === null) throw new Error('Event was not recorded.');
      if (row.notRecordedReason !== undefined && typeof row.notRecordedReason !== 'string') {
        throw new Error('The event feed row is malformed.');
      }
      if (row.parent !== undefined && typeof row.parent !== 'string') {
        throw new Error('The event feed row is malformed.');
      }
      const valid = validateFeedRow(row.row);
      if (at !== undefined && valid.readHour > at) {
        throw new Error(`Event ${valid.id} was read at hour ${valid.readHour} after displayed hour ${at}.`);
      }
      const choice = row.choice == null ? null : validateScriptEdge(row.choice, at);
      return { ...(row as unknown as Extract<EventFeedLine, { line: 'event' }> & DatedLogRow), line: 'event', row: valid, choice };
    }
    if (row.line === 'fold') {
      if (row.firstRow === null) throw new Error('Event was not recorded.');
      if (
        typeof row.count !== 'string' || !/^\d+$/.test(row.count) || Number(row.count) < 5
        || typeof row.country !== 'string' || !row.country.length || row.country.includes('\0')
        || typeof row.kind !== 'string'
      ) {
        throw new Error('The event feed row is malformed.');
      }
      const firstRow = validateFeedRow(row.firstRow);
      if (row.kind !== firstRow.kind) {
        throw new Error('The event feed row is malformed.');
      }
      if (at !== undefined && firstRow.readHour > at) {
        throw new Error(`Event ${firstRow.id} was read at hour ${firstRow.readHour} after displayed hour ${at}.`);
      }
      return { ...(row as unknown as Extract<EventFeedLine, { line: 'fold' }> & DatedLogRow), line: 'fold', firstRow };
    }
    if (row.line === 'more') {
      if (
        typeof row.parent !== 'string' || !row.parent.length
        || typeof row.total !== 'string' || !/^\d+$/.test(row.total)
        || typeof row.remaining !== 'string' || !/^\d+$/.test(row.remaining)
      ) {
        throw new Error('The event feed row is malformed.');
      }
      return { ...(row as unknown as Extract<EventFeedLine, { line: 'more' }> & DatedLogRow), line: 'more' };
    }
    throw new Error('The event feed row is malformed.');
  }, readAt);
}

export const eventFeedProjection: TableProjection<EventFeedProjectionOptions> = registerFamilyProjection({
  kind: 'event-feed',
  family: 'table',
  validateTail: validateEventFeedTail,
  project(readView, options): TableModel {
    const hour = readView.hour;
    const empty = { columns: EVENT_FEED_COLUMNS, rows: [], hour, searchable: true, search: options.search ?? '', dateGrouped: options.byDate !== false } as const;
    if (!options.categories.length) return { ...empty, empty: 'No recorded events match these settings.' };
    const refused = readView.refused('main');
    if (refused !== undefined) return { ...empty, refused };
    const source = readView.rows<ValidatedEventFeedLine>('main');
    if (!source) return empty;
    if (source.refused) return { ...empty, refused: source.refused };

    const boundary = newIdentityBoundary();
    const expanded = options.expanded ?? [];
    const paged = projectPagedSlots(
      source,
      hour,
      readView.rowWindow,
      rawLine => {
        if (rawLine.line === 'fold') return buildFoldRow(readView, rawLine, options.artwork, boundary, expanded);
        if (rawLine.line === 'more') return buildMoreRow(rawLine);
        const page: FeedPageInfo = {
          choice: rawLine.choice ?? null,
          status: rawLine.status,
          ...(rawLine.notRecordedReason !== undefined ? { notRecordedReason: rawLine.notRecordedReason } : {}),
          live: rawLine.live === true,
        };
        return buildFeedTableRow(readView, rawLine.row, page, rawLine.key, options.artwork, source.at ?? hour, boundary, options.hover ?? [], rawLine.parent);
      },
    );
    const rawHead = source.rawHead as { readonly warnings?: readonly string[]; readonly unavailable?: readonly { readonly label: string; readonly reason: string }[] } | undefined;
    const warnings = [...(rawHead?.warnings ?? []), ...(rawHead?.unavailable ?? []).map(u => `${u.label}: ${u.reason}`)];
    return {
      ...empty,
      rows: paged.rows,
      slots: paged.slots,
      windowOffset: paged.windowOffset,
      totalRows: paged.totalRows,
      ...(paged.gapBefore?.length ? { gapBefore: paged.gapBefore } : {}),
      nextChangeAt: boundary.next,
      rowTimeBoundaries: false,
      ...(warnings.length ? { warnings: [...new Set(warnings)] } : {}),
      ...(paged.slots.length === 0 ? { empty: 'No recorded events match these settings.' } : {}),
    };
  },
});

function excludedKindsOf(raw: unknown): string[] {
  if (typeof raw === 'string' && raw.length > 0) return raw.split(',').filter(Boolean);
  return Array.isArray(raw) ? raw.filter((item): item is string => typeof item === 'string') : [];
}

export const eventFeedDeclaration: TableWidgetDeclaration<EventFeedProjectionOptions> = {
  family: 'table',
  kind: 'event-feed',
  label: 'Event feed',
  subject: 'event feed',
  emptyCampaign: 'Load a campaign to see key moments.',
  icon: 'clock',
  countries: { mode: 'filter', content: 'history' },
  columns: EVENT_FEED_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx): SourcePlan {
    const { sort } = tableSettingsOf(ctx.options[TABLE_SETTINGS_KEY], EVENT_FEED_COLUMNS, LOG_TABLE_BASE);
    const search = typeof ctx.options.search === 'string' ? ctx.options.search : '';
    const categories = CATEGORIES.filter(c => ctx.options[c.value] === true).map(c => c.value);
    const excludedKinds = excludedKindsOf(ctx.options.excludedKinds);
    if (!categories.length) {
      return { owner: `${ctx.track.source.id}:${ctx.branch.id}:event-feed:empty`, entries: [], allowEmptyFollow: true };
    }
    const win = !ctx.following ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const rawFrom = ctx.widget.time.mode === 'range' && win ? Math.max(Math.floor(win.firstHour), ctx.branch.firstHour) : null;
    const boundFrom = rawFrom !== null ? ctx.gateSpan(rawFrom, ctx.branch.lastHour)?.firstHour ?? null : null;
    const selection = occurrenceSelection(ctx);
    const opened = contentSet(ctx.options.expanded).filter(key => key.startsWith(GROUP));
    const expanded = opened.map(key => key.slice(GROUP.length));
    const full = contentSet(ctx.options.full).filter(key => opened.includes(key)).map(key => key.slice(GROUP.length));
    return {
      owner: `${ctx.track.source.id}:${ctx.branch.id}:event-feed:${categories.join(',')}:${excludedKinds.join(',')}:${selection ? JSON.stringify(selection) : 'all'}:${expanded.join(',')}:${full.join(',')}:${JSON.stringify([search, sort])}`,
      loadingLabel: search.trim() || sort.column === 'item' ? 'Preparing titles…' : undefined,
      entries: [
        pagedTailEntry(
          'main',
          ctx.track.source,
          ctx.branch.id,
          ctx.track.axisOffset,
          {
            product: 'event-feed-tail',
            branchId: ctx.branch.id,
            from: boundFrom,
            limit: HEAD_LIMIT,
            ask: { search, sort,
              categories,
              ...(excludedKinds.length ? { exclude: excludedKinds } : {}),
              ...selection,
              ...(expanded.length ? { expanded } : {}),
              ...(full.length ? { full } : {}),
            },
          },
          ctx.following,
          DEFAULT_TAIL_PAGE_SIZE,
          validateEventFeedTail,
        ),
      ],
    };
  },
  projection: eventFeedProjection,
  projectionOptions(ctx): EventFeedProjectionOptions {
    const categories = CATEGORIES.filter(c => ctx.options[c.value] === true).map(c => c.value);
    return { byDate: ctx.settings.sort.column === 'date', search: typeof ctx.options.search === 'string' ? ctx.options.search : '', categories, artwork: ctx.options.artwork !== false, hover: hoverTextOf(ctx.options), expanded: contentSet(ctx.options.expanded) };
  },
};
