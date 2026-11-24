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
import type { TableCell, TableModel, TableModelSlot, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { DEFAULT_TAIL_PAGE_SIZE, type DatedLogRow } from '../../families/table/rowSource.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import { formatRecordedPercent, formatScalar } from '../../families/valueFormat.ts';
import type { EventFeedItem, EventFeedPageRow, EventFeedRow, ScriptEdge, ScriptStatus } from '../../generated/dashboard';
import { optionLetter } from '../event-choices/eventChoicesData.ts';
import type { EventDetailSubject } from '../event-choices/EventDetailWindow.tsx';
import { CATEGORIES, EVENT_FEED_COLUMNS } from './definition.ts';
import { validateFeedRow } from './eventFeedData.ts';
import { plainScripted, reasonsOf } from './scriptedGaps.ts';

export interface EventFeedProjectionOptions {
  readonly categories: readonly string[];
  readonly artwork: boolean;
}

const HEAD_LIMIT = 256;
const FOLD_MIN_RUN = 5;

const LOSER_SECOND_KINDS = new Set(['annexed', 'warDeclared', 'warStarted']);
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
  readonly live: boolean;
}

function parsedOptionOrdinal(event: EventFeedRow, choice: ScriptEdge | null): number | null {
  if (choice && typeof choice.option === 'number' && Number.isInteger(choice.option)) return choice.option;
  if (event.kind === 'eventOptionSelected' && event.cause?.notes.length) {
    for (const note of event.cause.notes) {
      const match = /Selected native option ordinal:\s*(-?\d+)/.exec(note);
      if (match) return Number(match[1]);
    }
  }
  return null;
}

const nonBlank = (value: string | null | undefined): string | null => (value && value.trim().length > 0 ? value : null);

function effectiveEventItem(event: EventFeedRow, choice: ScriptEdge | null): EventFeedItem | null {
  const base = event.item;
  const fallback = choice?.row.item ?? null;
  if (!base) return fallback;
  if ((!base.options || base.options.length === 0) && fallback?.options?.length) {
    return { ...base, options: fallback.options };
  }
  return base;
}

function optionCellOf(item: EventFeedItem | null, ordinal: number): TableCell {
  const options = item?.options ?? [];
  const index = options.findIndex(opt => opt.ordinal === ordinal);
  if (index >= 0) {
    const opt = options[index]!;
    const letter = optionLetter(index);
    const text = nonBlank(opt.name);
    if (text !== null) return { value: knownValue(text), inlinePrefix: letter };
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
    return { value: notRecorded('Choice not recorded (recording paused)'), missingFigure: 'Choice' };
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
      return { primary: named ?? 'Decision selected', secondary: named ? 'Decision' : undefined, missingReason: reasons.nameMissing };
    case 'decisionActivated':
      return { primary: named ?? 'Mission activated', secondary: named ? 'Mission' : undefined, missingReason: reasons.nameMissing };
    case 'decisionCompleted':
      return {
        primary: named ?? 'Decision completed',
        secondary: event.decisionState?.kind === 'failed' ? 'Failed' : 'Completed',
        missingReason: reasons.nameMissing,
      };
    case 'decisionTimedOut':
      return { primary: named ?? 'Decision timed out', secondary: event.decisionState?.kind === 'failed' ? 'Failed' : 'Timed out', missingReason: reasons.nameMissing };
    case 'decisionCancelled':
      return { primary: named ?? 'Decision cancelled', secondary: event.decisionState?.kind === 'failed' ? 'Failed' : 'Cancelled', missingReason: reasons.nameMissing };
    case 'decisionRemoved':
      return { primary: named ?? 'Decision removed', secondary: event.decisionState?.kind === 'failed' ? 'Failed' : 'Removed', missingReason: reasons.nameMissing };
    case 'decisionReenabled':
      return { primary: named ?? 'Decision reenabled', secondary: 'Reenabled' };
    case 'decisionDurationChanged':
      return { primary: named ?? 'Duration changed', secondary: 'Duration changed' };
    case 'decisionRemovalEffect':
    case 'decisionCancellationEffect':
    case 'decisionCompletionEffect':
    case 'decisionTimeoutEffect':
      return { primary: named ?? 'Decision effect', secondary: 'Effect' };
    case 'warStarted':
    case 'warDeclared':
      return { primary: 'War declared', secondary: event.cause?.how ?? undefined };
    case 'warJoined':
      return { primary: 'Joined a war', secondary: event.cause?.reason ?? event.cause?.how ?? undefined };
    case 'civilWarWarTransferred':
      return { primary: 'Civil war transferred' };
    case 'warEnded':
    case 'peace':
      return { primary: 'Peace', secondary: event.cause?.how ?? undefined };
    case 'peaceConferenceStarted':
      return { primary: 'Peace conference' };
    case 'capitulated':
      return { primary: 'Capitulated' };
    case 'annexed':
      return { primary: 'Annexed', secondary: event.cause?.how ?? undefined };
    case 'released':
      return { primary: 'Released', secondary: event.cause?.how ?? undefined };
    case 'stateTransferred':
      return { primary: 'State transferred', secondary: stateLabel(event.cause?.state) };
    case 'ceasedToExist':
      return { primary: 'Ceased to exist' };
    case 'factionJoined':
      return {
        primary: 'Joined faction',
        ...(named ? { secondary: named } : {}),
        ...(named === null ? { detail: { value: notRecorded(reasons.nameMissing ?? 'Faction name not recorded at this hour') } } : {}),
      };
    case 'factionLeft':
      return {
        primary: 'Left faction',
        ...(named ? { secondary: named } : {}),
        ...(named === null ? { detail: { value: notRecorded(reasons.nameMissing ?? 'Faction name not recorded at this hour') } } : {}),
      };
    case 'becameSubject':
      return { primary: 'Became subject', secondary: named ?? undefined };
    case 'guaranteeGranted':
      return { primary: 'Guaranteed' };
    case 'rulingPartyChanged':
      return { primary: 'New government', secondary: named ?? undefined };
    case 'lawChanged':
      return { primary: 'Law changed', secondary: named ?? undefined };
    case 'advisorHired':
      return { primary: 'Advisor hired', secondary: named ?? undefined };
    case 'focusCompleted':
      return { primary: 'Focus completed', secondary: named ?? undefined };
    case 'projectFinished':
      return { primary: 'Project finished', secondary: named ?? undefined };
    case 'nukesIncreased':
      return { primary: 'Nuclear stockpile', secondary: event.amount ? `+${event.amount}` : undefined };
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
        primary: 'Nuclear strike',
        secondary: targetState,
        detail: detailCell,
      };
    }
  }
}

interface FoldMeta {
  readonly day: number;
  readonly country: string;
  readonly group: string;
}

function foldGroupOf(kind: EventFeedRow['kind']): string {
  if (kind === 'scriptEventFired' || kind === 'newsEventFired' || kind === 'eventOptionSelected') return 'script-event';
  if (kind.startsWith('decision')) return 'decision';
  return kind;
}

function foldConsecutiveSlots(
  slots: readonly TableModelSlot[],
  foldByKey: ReadonlyMap<string, FoldMeta>,
): { readonly slots: TableModelSlot[]; readonly rows: TableRow[] } {
  const outSlots: TableModelSlot[] = [];
  const outRows: TableRow[] = [];
  let i = 0;
  while (i < slots.length) {
    const slot = slots[i]!;
    if (slot.kind !== 'data') {
      outSlots.push(slot);
      i++;
      continue;
    }
    const meta = foldByKey.get(slot.row.key);
    if (!meta || !meta.country) {
      outSlots.push(slot);
      outRows.push(slot.row);
      i++;
      continue;
    }
    let end = i + 1;
    while (end < slots.length) {
      const next = slots[end]!;
      if (next.kind !== 'data') break;
      const nextMeta = foldByKey.get(next.row.key);
      if (!nextMeta || nextMeta.day !== meta.day || nextMeta.country !== meta.country || nextMeta.group !== meta.group) break;
      end++;
    }
    const runLen = end - i;
    if (runLen >= FOLD_MIN_RUN) {
      const children: TableRow[] = [];
      for (let j = i; j < end; j++) {
        const entry = slots[j]!;
        if (entry.kind === 'data') children.push(entry.row);
      }
      const first = children[0]!;
      const label = `${formatScalar(runLen, 'integer')} events`;
      const parent: TableRow = {
        key: `fold:${first.key}`,
        name: label,
        rowHour: first.rowHour,
        initiallyOpen: false,
        children,
        cells: {
          date: first.cells.date!,
          kind: first.cells.kind!,
          country: first.cells.country!,
          item: { value: knownValue(label) },
          detail: { value: notApplicable('Collapsed group'), blank: true },
          action: { value: notApplicable('Expand group'), blank: true },
        },
        tooltipTitle: label,
        tooltipAction: 'Click to expand',
      };
      outSlots.push({ kind: 'data', row: parent });
      outRows.push(parent);
    } else {
      for (let j = i; j < end; j++) {
        const entry = slots[j]!;
        outSlots.push(entry);
        if (entry.kind === 'data') outRows.push(entry.row);
      }
    }
    i = end;
  }
  return { slots: outSlots, rows: outRows };
}

function buildFeedTableRow(
  readView: BoundedReadView,
  event: EventFeedRow,
  page: FeedPageInfo,
  key: string,
  artwork: boolean,
  displayedHour: number,
  boundary: ReturnType<typeof newIdentityBoundary>,
): { readonly row: TableRow; readonly fold: FoldMeta } {
  const item = effectiveEventItem(event, page.choice);
  const { primary, secondary, detail, missingReason } = eventItemAndSubtext(event, page, readView);
  const reasons = reasonsOf(item);
  const spriteKey = !item?.icon ? KIND_SPRITES[event.kind] : undefined;
  const artHandle = artwork
    ? (urlArtHandle(item?.icon ?? null) ?? (spriteKey ? { kind: 'sprite' as const, key: spriteKey, frame: 0 } : null))
    : null;
  const artResult = artHandle ? readView.art(artHandle) : null;
  const icon = artResult?.status === 'ready' ? artResult.url : undefined;
  const iconPending = artResult?.status === 'pending' ? true : undefined;

  const rawRoles = event.participants ?? event.tags;
  const countryEntries: { readonly tag: string | null; readonly dimmed?: boolean; readonly missingLabel?: string }[] = [];
  for (let idx = 0; idx < rawRoles.length; idx++) {
    const tag = rawRoles[idx] ?? null;
    if (tag === null && event.participants === undefined) continue;
    if (idx > 0 && tag !== null && tag === rawRoles[0]) continue;
    if (tag === null && idx === 1 && rawRoles[0] !== null && !['scriptEventFired', 'newsEventFired', 'warDeclared', 'warJoined', 'annexed', 'released'].includes(event.kind)) {
      continue;
    }
    countryEntries.push({
      tag,
      dimmed: idx === 1 && LOSER_SECOND_KINDS.has(event.kind) || (idx === 0 && (event.kind === 'capitulated' || event.kind === 'ceasedToExist')),
      missingLabel: 'Country not identified',
    });
  }
  if (!countryEntries.length && event.tags.length) {
    for (const tag of event.tags) countryEntries.push({ tag });
  }

  const viewable = VIEWABLE_KINDS.has(event.kind);
  const chosenOrdinal = parsedOptionOrdinal(event, page.choice);
  const chosenHour = page.choice?.row.hour ?? (event.kind === 'eventOptionSelected' ? event.hour : null);
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
      : awaiting
        ? 'No option chosen yet'
        : page.status === 'notRecorded'
          ? 'No choice recorded'
          : page.choice
            ? 'Chosen option not recorded'
            : 'Event fired';
    detailSubject = {
      key,
      hour: event.hour,
      firedHour: event.hour,
      asOfHour: displayedHour,
      title: item?.name ?? null,
      countryTag: recipient,
      countryName,
      flagUrl: flag?.icon,
      flagPending: flag?.pending,
      item,
      chosenOrdinal,
      awaiting,
      chosenHour,
      noChoiceReason,
    };
  }

  const titleLabel = primary !== null ? plainScripted(primary) : (missingReason ?? 'Not recorded');
  const tooltipExtra = (): TooltipExtraRow[] => {
    const out: TooltipExtraRow[] = [
      { label: 'Time', value: knownValue(formatDateTime(event.hour)) },
    ];
    if (secondary) out.push({ label: 'Detail', value: knownValue(plainScripted(secondary)) });
    if (event.cause?.how) out.push({ label: 'Cause', value: knownValue(event.cause.how) });
    if (event.strike) {
      if (event.strike.manpowerLost != null) out.push({ label: 'Casualties', value: knownValue(formatScalar(event.strike.manpowerLost, 'integer')) });
      if (event.strike.buildingLevels != null) out.push({ label: 'Building levels', value: knownValue(formatScalar(event.strike.buildingLevels, 'decimal1')) });
      if (event.strike.warSupport != null) out.push({ label: 'War support', value: knownValue(formatRecordedPercent(event.strike.warSupport, { signed: true })) });
      if (event.strike.units != null) out.push({ label: 'Units', value: knownValue(formatScalar(event.strike.units, 'integer')) });
      if (event.strike.equipmentLost != null) out.push({ label: 'Equipment lost', value: knownValue(formatScalar(event.strike.equipmentLost, 'integer')) });
      for (const reason of event.strike.notRecorded) {
        out.push({ label: 'Effect not recorded', value: notRecorded(reason) });
      }
    }
    if (item?.token) out.push({ label: 'Content identity', value: knownValue(item.token) });
    if (reasons.nameMissing) out.push({ label: 'Name not recorded', value: notRecorded(reasons.nameMissing) });
    if (reasons.gaps) out.push({ label: 'Text not recorded', value: notRecorded(reasons.gaps) });
    if (item?.description) out.push({ label: 'Description', value: knownValue(plainScripted(item.description)), divider: true });
    if (item?.options?.length) {
      item.options.forEach((opt, i) => {
        out.push({
          label: `Option ${optionLetter(i)}`,
          value: opt.name !== null ? knownValue(plainScripted(opt.name)) : notRecorded('Option text not recorded'),
        });
      });
    }
    return out;
  };

  const primaryTag = countryEntries[0]?.tag ?? '';
  return {
    fold: {
      day: Math.floor(event.hour / 24),
      country: primaryTag,
      group: foldGroupOf(event.kind),
    },
    row: {
      key,
      name: titleLabel,
      rowHour: event.hour,
      cells: {
        date: logDateCell(event.hour, event.exact ? undefined : 'by'),
        kind: artwork
          ? {
              value: knownValue(titleLabel),
              icon,
              iconPending,
              iconSlot: true,
              artShape: viewable ? 'wide' : 'square',
              artName: titleLabel,
            }
          : { value: notApplicable('Game artwork hidden'), blank: true },
        country: logCountryCell(readView, countryEntries, event.nameHour, boundary),
        item: primary !== null
          ? { value: knownValue(primary), ...(secondary ? { inlineSubtext: secondary } : {}) }
          : { value: notRecorded(missingReason ?? 'Not recorded') },
        detail: detail ?? { value: notApplicable('No extra detail'), blank: true },
        action: viewable ? { value: knownValue('View'), actionLabel: 'View' } : { value: notApplicable('No event window for this entry'), blank: true },
      },
      tooltipTitle: titleLabel,
      tooltipExtra,
      ...(viewable ? { tooltipAction: 'Click to view event', meta: detailSubject } : {}),
    },
  };
}

export const eventFeedProjection: TableProjection<EventFeedProjectionOptions> = registerFamilyProjection({
  kind: 'event-feed',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    const empty = { columns: EVENT_FEED_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true } as const;
    if (!options.categories.length) return { ...empty, empty: 'No recorded events match these settings.' };
    const refused = readView.refused('main');
    if (refused !== undefined) return { ...empty, refused };
    const source = readView.rows<EventFeedPageRow & DatedLogRow>('main');
    if (!source) return empty;
    if (source.refused) return { ...empty, refused: source.refused };

    const boundary = newIdentityBoundary();
    const foldByKey = new Map<string, FoldMeta>();
    const paged = projectPagedSlots(
      source,
      hour,
      readView.rowWindow,
      rawRow => {
        if (!rawRow.row) return null;
        let valid: EventFeedRow;
        try {
          valid = validateFeedRow(rawRow.row);
        } catch {
          return null;
        }
        const page: FeedPageInfo = {
          choice: rawRow.choice ?? null,
          status: rawRow.status,
          live: rawRow.live === true,
        };
        const built = buildFeedTableRow(readView, valid, page, rawRow.key, options.artwork, hour, boundary);
        foldByKey.set(built.row.key, built.fold);
        return built.row;
      },
    );
    const rawHead = source.rawHead as { readonly warnings?: readonly string[]; readonly unavailable?: readonly { readonly label: string; readonly reason: string }[] } | undefined;
    const warnings = [...(rawHead?.warnings ?? []), ...(rawHead?.unavailable ?? []).map(u => `${u.label}: ${u.reason}`)];
    const folded = foldConsecutiveSlots(paged.slots, foldByKey);
    return {
      ...empty,
      rows: folded.rows,
      slots: folded.slots,
      windowOffset: paged.windowOffset,
      totalRows: paged.totalRows,
      nextChangeAt: boundary.next,
      rowTimeBoundaries: false,
      ...(warnings.length ? { warnings: [...new Set(warnings)] } : {}),
      ...(folded.slots.length === 0 ? { empty: 'No recorded events match these settings.' } : {}),
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
  countries: { mode: 'filter', labels: true },
  columns: EVENT_FEED_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx): SourcePlan {
    const categories = CATEGORIES.filter(c => ctx.options[c.value] === true).map(c => c.value);
    const excludedKinds = excludedKindsOf(ctx.options.excludedKinds);
    if (!categories.length) {
      return { owner: `${ctx.track.source.id}:${ctx.branch.id}:event-feed:empty`, entries: [], allowEmptyFollow: true };
    }
    const win = !ctx.following ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const rawFrom = ctx.widget.time.mode === 'range' && win ? Math.max(Math.floor(win.firstHour), ctx.branch.firstHour) : null;
    const boundFrom = rawFrom !== null ? ctx.gateSpan(rawFrom, ctx.branch.lastHour)?.firstHour ?? null : null;
    const selection = occurrenceSelection(ctx);
    return {
      owner: `${ctx.track.source.id}:${ctx.branch.id}:event-feed:${categories.join(',')}:${excludedKinds.join(',')}:${selection ? JSON.stringify(selection) : 'all'}`,
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
            ask: { categories, ...(excludedKinds.length ? { exclude: excludedKinds } : {}), ...selection },
          },
          ctx.following,
          DEFAULT_TAIL_PAGE_SIZE,
        ),
      ],
    };
  },
  projection: eventFeedProjection,
  projectionOptions(ctx): EventFeedProjectionOptions {
    const categories = CATEGORIES.filter(c => ctx.options[c.value] === true).map(c => c.value);
    return { categories, artwork: ctx.options.artwork !== false };
  },
};
