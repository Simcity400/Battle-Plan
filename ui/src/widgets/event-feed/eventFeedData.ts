import type { LogPage, LogRow } from '../../charts/log/logData';
import type { BoundFacetFilter, LogReplyRow } from '../../charts/log/logSource';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { drawableIcon } from '../../dashboard/artwork.ts';
import type { ProductRequest } from '../../dashboard/client';
import { formatDate, formatDateTime, gameDate } from '../../dashboard/calendar.ts';
import { int, object, opaqueTag } from '../../dashboard/validation.ts';
import type { EventFeedItem, EventFeedRow, EventFeedSpanReply, EventFeedStrike, EventFeedUnavailable, FeedKindCode } from '../../generated/dashboard';
import { plainScripted, reasonRows } from './scriptedGaps.ts';
import { productRequest } from '../../dashboard/request.ts';

export const SCRIPT_KINDS = new Set(['scriptEventFired', 'newsEventFired', 'eventOptionSelected', 'decisionSelected', 'decisionRemoved', 'decisionCancelled',
  'decisionTimedOut', 'decisionActivated', 'decisionCompleted', 'decisionReenabled', 'decisionRemovalEffect', 'decisionCancellationEffect',
  'decisionCompletionEffect', 'decisionTimeoutEffect', 'decisionDurationChanged']);
const NATIVE_KINDS = new Set([...SCRIPT_KINDS, 'warDeclared', 'warJoined', 'civilWarWarTransferred', 'annexed', 'warEnded', 'released',
  'stateTransferred', 'peaceConferenceStarted', 'nuclearStrike']);
const CAUSED_KINDS = new Set([...NATIVE_KINDS].filter(kind => kind !== 'nuclearStrike'));
const KINDS = new Set([...NATIVE_KINDS, 'warStarted', 'peace', 'capitulated', 'ceasedToExist', 'factionJoined', 'factionLeft', 'becameSubject',
  'guaranteeGranted', 'rulingPartyChanged', 'lawChanged', 'advisorHired', 'focusCompleted', 'projectFinished', 'nukesIncreased', 'nuclearStrike']);
const CATEGORY_NAMES = new Set(['warsPeace', 'diplomacy', 'politics', 'milestones']);
const text = (value: unknown): value is string | null => value === null || typeof value === 'string';
const amount = (value: unknown): value is number | null => value === null || (typeof value === 'number' && Number.isFinite(value));
const strike = (row: Record<string, unknown>): boolean => {
  if (row.kind !== 'nuclearStrike') return row.strike === undefined;
  const value = row.strike;
  return object(value) && int(value.province) && value.province >= 0 && (value.state === null || int(value.state))
    && amount(value.buildingLevels) && amount(value.warSupport) && (value.units === null || (int(value.units) && value.units >= 0))
    && amount(value.manpowerLost) && amount(value.equipmentLost)
    && Array.isArray(value.notRecorded) && value.notRecorded.every(reason => typeof reason === 'string');
};

const optionalText = (value: unknown) => value === undefined || typeof value === 'string';
const item = (value: unknown): value is EventFeedItem => object(value) && text(value.token) && text(value.name) && text(value.icon) && text(value.description)
  && optionalText(value.nameMissing) && optionalText(value.gaps)
  && (value.options === undefined || (Array.isArray(value.options) && value.options.every(option => object(option) && int(option.ordinal) && text(option.name))));
const roles = (row: Record<string, unknown>): boolean => {
  if (!NATIVE_KINDS.has(row.kind as string)) return row.participants === undefined;
  return row.exact === true && Array.isArray(row.participants) && row.participants.length === 2 && row.participants.every(value => value === null || opaqueTag(value))
    && JSON.stringify(row.participants.filter(value => value !== null)) === JSON.stringify(row.tags);
};
const cause = (row: Record<string, unknown>): boolean => {
  const value = row.cause;
  if (value === undefined) return true;
  return CAUSED_KINDS.has(row.kind as string) && object(value) && text(value.how) && text(value.within) && text(value.conference) && text(value.reason)
    && (value.state === null || (int(value.state) && value.state > 0)) && (value.ally === null || opaqueTag(value.ally))
    && Array.isArray(value.notes) && value.notes.every(note => typeof note === 'string');
};

const decisionState = (row: Record<string, unknown>): boolean => {
  const value = row.decisionState;
  if (value === undefined) return true;
  if (!SCRIPT_KINDS.has(row.kind as string) || !object(value) || typeof value.kind !== 'string') return false;
  switch (value.kind) {
    case 'active': case 'available': case 'completed': case 'aborted': case 'reEnableCooldown': return true;
    case 'failed': return int(value.observedHour);
    case 'unsupported': return int(value.raw);
    case 'notApplicable':
    case 'notRecorded': return typeof value.reason === 'string' && value.reason.length > 0;
    default: return false;
  }
};

export function validateFeedRow(value: unknown): EventFeedRow {
  if (!object(value) || typeof value.id !== 'string' || value.id.length === 0 || !int(value.hour) || !int(value.readHour) || !int(value.nameHour)
    || value.hour > value.readHour || typeof value.exact !== 'boolean' || typeof value.category !== 'string' || !CATEGORY_NAMES.has(value.category)
    || typeof value.kind !== 'string' || !KINDS.has(value.kind) || !Array.isArray(value.tags) || !value.tags.every(opaqueTag)
    || (!NATIVE_KINDS.has(value.kind) && value.tags.length === 0) || !roles(value) || !cause(value) || !strike(value) || !decisionState(value)
    || !(value.item === null || item(value.item)) || !(value.amount === null || (int(value.amount) && value.amount > 0))) {
    throw new Error('The event feed row is malformed.');
  }
  const row = value as unknown as EventFeedRow;
  const icon = row.item ? drawableIcon(row.item.icon) : null;
  return row.item && icon !== row.item.icon ? { ...row, item: { ...row.item, icon } } : row;
}

export const FEED_PAGE_SIZE = 64;

export interface FeedEvent { event: EventFeedRow; item: EventFeedItem | null }

export const feedLogRow = (event: EventFeedRow, key: string): LogRow<FeedEvent> =>
  ({ key, id: event.id, hour: event.hour, revealedHour: event.readHour, by: false, item: { event, item: event.item } });

export function feedRow(reply: LogReplyRow): LogRow<FeedEvent> {
  return feedLogRow(validateFeedRow(reply.fields.row), reply.key);
}

export function kindsRequest(track: { source: { id: string; revision: string }; branch: { id: string } }): ProductRequest {
  return productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'event-feed-span',
    params: { branchId: track.branch.id, categories: [], kindsOnly: true } });
}

export function validateSpan(value: unknown): EventFeedSpanReply {
  if (!object(value) || typeof value.branchId !== 'string' || !int(value.firstHour) || !int(value.lastHour) || !Array.isArray(value.kinds)
    || !value.kinds.every(kind => object(kind) && typeof kind.kind === 'string' && KINDS.has(kind.kind) && int(kind.code) && typeof kind.category === 'string'
      && CATEGORY_NAMES.has(kind.category)) || !Array.isArray(value.rows) || !Array.isArray(value.unavailable)
    || !value.unavailable.every(note => object(note) && typeof note.kind === 'string' && typeof note.label === 'string' && typeof note.reason === 'string'
      && (note.count === null || (int(note.count) && note.count >= 0)))
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string')) {
    throw new Error('The event feed span reply is malformed.');
  }
  const rows = value.rows.map(validateFeedRow);
  if (rows.some((row, at) => at > 0 && row.hour < rows[at - 1].hour)) throw new Error('The event feed span reply is malformed.');
  return { ...(value as unknown as EventFeedSpanReply), rows };
}

export type Facet = BoundFacetFilter;

export function feedFacets(kinds: readonly FeedKindCode[], categories: ReadonlySet<string>): Facet[] | null {
  const values = kinds.filter(kind => categories.has(kind.category)).map(kind => kind.code);
  return values.length ? [{ name: 'kind', values, countries: [] }] : null;
}

export const eventCountryHour = (event: EventFeedRow): number =>
  event.kind === 'ceasedToExist' || event.kind === 'factionLeft' ? event.nameHour : event.hour;

export const rowMatches = (event: EventFeedRow, country: (tag: string, hour: number) => boolean): boolean =>
  eventTags(event).some(tag => country(tag, eventCountryHour(event)));

export function familyRows(span: EventFeedSpanReply, categories: ReadonlySet<string>, firstHour: number | null,
  country: ((tag: string, hour: number) => boolean) | null): LogRow<FeedEvent>[] {
  return span.rows.filter(row => categories.has(row.category) && (firstHour === null || row.hour >= firstHour) && (!country || rowMatches(row, country)))
    .map(row => feedLogRow(row, `family:${row.id}`));
}

export const familyReveals = (rows: readonly LogRow<FeedEvent>[]): number[] => rows.map(row => row.revealedHour ?? row.hour).sort((a, b) => a - b);

export const familyShown = (rows: readonly LogRow<FeedEvent>[], hour: number | null): number =>
  hour === null ? rows.length : rows.filter(row => (row.revealedHour ?? row.hour) <= hour).length;

export function withFamilyRows<T>(pages: readonly LogPage<T>[], family: readonly LogRow<T>[], shown: number): readonly LogPage<T>[] {
  const rows = family.slice(0, shown);
  if (!rows.length) return pages;
  if (!pages.length) return [{ key: 'family', start: 0, end: 0, count: rows.length, rows }];
  return pages.map((page, index) => {
    if (!page.rows) return page;
    const first = index === 0 ? -Infinity : page.rows[0]?.hour ?? Infinity;
    const next = pages[index + 1];
    const end = !next ? Infinity : next.rows ? next.rows[0]?.hour ?? Infinity : (page.rows.at(-1)?.hour ?? -Infinity) + 1;
    const taken = rows.filter(row => row.hour >= first && row.hour < end);
    return taken.length ? { ...page, rows: [...page.rows, ...taken], count: page.count + taken.length } : page;
  });
}

export function mergeSpanReplies(replies: readonly EventFeedSpanReply[]): EventFeedSpanReply | null {
  if (!replies.length) return null;
  if (replies.length === 1) return replies[0];
  const sorted = [...replies].sort((a, b) => a.firstHour - b.firstHour);
  const rows: EventFeedRow[] = [];
  const seen = new Set<string>();
  const warnings = new Set<string>();
  const unavailable = new Map<string, EventFeedUnavailable>();
  for (const reply of sorted) {
    for (const row of reply.rows) {
      if (!seen.has(row.id)) {
        seen.add(row.id);
        rows.push(row);
      }
    }
    for (const warning of reply.warnings) warnings.add(warning);
    for (const note of reply.unavailable) {
      const key = JSON.stringify([note.kind, note.label, note.reason]);
      const held = unavailable.get(key);
      unavailable.set(key, held ? { ...held, count: held.count === null || note.count === null ? null : held.count + note.count } : note);
    }
  }
  rows.sort((a, b) => a.hour - b.hour || a.readHour - b.readHour);
  return {
    branchId: sorted[0].branchId,
    firstHour: sorted[0].firstHour,
    lastHour: sorted[sorted.length - 1].lastHour,
    kinds: sorted[0].kinds,
    rows,
    unavailable: [...unavailable.values()],
    warnings: [...warnings],
  };
}

export function spanNotes(span: EventFeedSpanReply): string[] {
  return [...span.warnings, ...notRecorded(span.unavailable)];
}

const isNote = (note: unknown): note is EventFeedUnavailable => object(note) && typeof note.kind === 'string' && typeof note.label === 'string'
  && typeof note.reason === 'string' && (note.count === null || Number.isSafeInteger(note.count));

export function pageNotes(page: unknown): EventFeedUnavailable[] {
  if (!object(page) || !Array.isArray(page.unavailable) || !page.unavailable.every(isNote) || !Array.isArray(page.warnings)
    || !page.warnings.every(warning => typeof warning === 'string')) throw new Error('The event feed page reply is malformed.');
  return [...(page.warnings as string[]).map(reason => ({ kind: 'warning', label: '', reason, count: null })), ...page.unavailable];
}

export function pagesNoted(notes: readonly EventFeedUnavailable[]): string[] {
  const merged = new Map<string, EventFeedUnavailable>();
  for (const note of notes) {
    const key = JSON.stringify([note.kind, note.label, note.reason]), held = merged.get(key);
    merged.set(key, held ? { ...held, count: held.count === null || note.count === null ? null : held.count + note.count } : note);
  }
  const all = [...merged.values()];
  return [...all.filter(note => note.kind === 'warning').map(note => note.reason), ...notRecorded(all.filter(note => note.kind !== 'warning'))];
}

export const eventCountryName = (tag: string, label: { name: string; known?: boolean; nameFailed?: boolean }): string =>
  label.nameFailed ? `${tag} (name not recorded)` : label.known === undefined ? `${tag} (loading name…)`
    : label.known ? label.name : `${tag} (name not known)`;

export const eventTags = (event: EventFeedRow): string[] => event.cause?.ally ? [...event.tags, event.cause.ally] : event.tags;

export type StateName = (hour: number, state: number) => string | null;
const stateNumber: StateName = () => null;

export type SentencePart = { text: string } | { tag: string };
export function eventSentence(at: FeedEvent, stateName: StateName = stateNumber): SentencePart[] {
  const parts = actionSentence(at, stateName);
  const how = at.event.cause?.how;
  if (!how) return parts;
  const ally = at.event.kind === 'warJoined' ? at.event.cause?.ally : null;
  return [...parts, { text: ` through ${how}` }, ...(ally ? [{ text: ', alongside ' }, { tag: ally }] : [])];
}

function actionSentence(at: FeedEvent, stateName: StateName): SentencePart[] {
  const { event, item } = at;
  const role = (index: number): SentencePart => {
    const tag = event.participants ? event.participants[index] : event.tags[index];
    return tag == null ? { text: `${index === 0 ? 'A' : 'a'} country not identified` } : { tag };
  };
  const country = role(0);
  const other = role(1);
  const named = item?.name ?? 'an item whose name is unavailable';
  const words = (before: string, after = ''): SentencePart[] => [country, { text: before }, other, ...(after ? [{ text: after }] : [])];
  const single = (action: string): SentencePart[] => [country, { text: action }];
  switch (event.kind) {
    case 'scriptEventFired': return [country, { text: ` triggered ${named} for ` }, other];
    case 'newsEventFired': return [country, { text: ` triggered news ${named} for ` }, other];
    case 'eventWindowPresented': return single(` was shown the event window ${named}`);
    case 'eventOptionSelected': return single(` selected an option in ${named}`);
    case 'decisionSelected': return single(` selected decision ${named}`);
    case 'decisionRemoved': return single(event.decisionState?.kind === 'failed' ? ` failed mission ${named} (ended)`
      : event.decisionState?.kind === 'unsupported' ? ` ended decision ${named} (unsupported native state ${event.decisionState.raw})` : ` ended decision ${named}`);
    case 'decisionCancelled': return single(event.decisionState?.kind === 'failed' ? ` failed mission ${named} (cancelled)`
      : event.decisionState?.kind === 'unsupported' ? ` cancelled decision ${named} (unsupported native state ${event.decisionState.raw})` : ` cancelled decision ${named}`);
    case 'decisionTimedOut': return single(event.decisionState?.kind === 'failed' ? ` failed mission ${named} (reached the deadline)`
      : event.decisionState?.kind === 'unsupported' ? ` reached the deadline of ${named} (unsupported native state ${event.decisionState.raw})` : ` reached the deadline of ${named}`);
    case 'decisionActivated': return single(` activated ${named}`);
    case 'decisionCompleted': return single(event.decisionState?.kind === 'failed' ? ` failed mission ${named}`
      : event.decisionState?.kind === 'unsupported' ? ` completed mission ${named} (unsupported native state ${event.decisionState.raw})` : ` completed mission ${named}`);
    case 'decisionReenabled': return single(` reenabled ${named}`);
    case 'decisionRemovalEffect': return single(` ran removal effects for ${named}`);
    case 'decisionCancellationEffect': return single(` ran cancellation effects for ${named}`);
    case 'decisionCompletionEffect': return single(` ran completion effects for ${named}`);
    case 'decisionTimeoutEffect': return single(` ran deadline effects for ${named}`);
    case 'decisionDurationChanged': return single(event.decisionState?.kind === 'failed' ? ` updated the duration of failed mission ${named}`
      : event.decisionState?.kind === 'unsupported' ? ` updated the duration of ${named} (unsupported native state ${event.decisionState.raw})` : ` updated the duration of ${named}`);
    case 'warDeclared': return words(' declared war on ');
    case 'warJoined': return words(' joined the war against ');
    case 'civilWarWarTransferred': return words(' inherited a war against ');
    case 'annexed': return words(' annexed ');
    case 'warEnded': return words(' and ', ' were no longer at war');
    case 'released': return words(' released ');
    case 'stateTransferred': return event.cause?.state == null ? words(' took a state from ')
      : words(` took ${stateName(event.hour, event.cause.state) ?? `state ${event.cause.state}`} from `);
    case 'peaceConferenceStarted': return [{ text: 'A peace conference began between ' }, country, { text: ' and ' }, other];
    case 'warStarted': return words(' became at war with ');
    case 'peace': return words(' was no longer at war with ');
    case 'capitulated': return single(' capitulated');
    case 'ceasedToExist': return single(' ceased to exist');
    case 'factionJoined': return single(` joined ${named}`);
    case 'factionLeft': return single(` left ${named}`);
    case 'becameSubject': return words(' became a subject of ');
    case 'guaranteeGranted': return words(' guaranteed the independence of ');
    case 'rulingPartyChanged': return single(` changed its ruling party to ${named}`);
    case 'lawChanged': return single(` adopted ${named}`);
    case 'advisorHired': return single(` appointed ${named}`);
    case 'focusCompleted': return single(` completed focus ${named}`);
    case 'projectFinished': return single(` completed special project ${named}`);
    case 'nukesIncreased': return single(event.amount === null ? ' increased its nuclear stockpile (amount not known)'
      : ` increased its nuclear stockpile by ${event.amount.toLocaleString()} ${event.amount === 1 ? 'bomb' : 'bombs'}`);
    case 'nuclearStrike': {
      const bomb = item ? named : 'a nuclear bomb of a type not known';
      return event.strike?.state === null ? single(` dropped ${bomb} on a province without a state`) : words(` dropped ${bomb} on `);
    }
  }
}

export function eventDescription(at: FeedEvent, name: (tag: string) => string, stateName?: StateName): string {
  return eventSentence(at, stateName).map(part => 'tag' in part ? name(part.tag) : plainScripted(part.text)).join('');
}

export function notRecorded(unavailable: readonly EventFeedUnavailable[]): string[] {
  return unavailable.map(item => `${item.label}: Not recorded — ${item.reason}${item.count === null ? '' : ` (${item.count.toLocaleString()} counted)`}`);
}

const notFullyRecorded = 'Not fully recorded';
const total = (value: number | null, unit: (value: number) => string): string => value === null ? notFullyRecorded : unit(value);
const whole = (value: number) => value.toLocaleString(undefined, { maximumFractionDigits: 0 });

function strikeRows(strike: EventFeedStrike, hour: number, stateName?: StateName): TooltipRow[] {
  const rows: TooltipRow[] = [
    { label: 'State', value: strike.state === null ? 'None' : stateName?.(hour, strike.state) ?? `State ${strike.state} (name not recorded)` },
    { label: 'Province', value: String(strike.province) },
    { label: 'Building levels destroyed', value: total(strike.buildingLevels, value => value.toLocaleString(undefined, { maximumFractionDigits: 2 })) },
    { label: 'War support', value: total(strike.warSupport, value => `${value > 0 ? '+' : ''}${value.toLocaleString(undefined, { maximumFractionDigits: 1 })}%`) },
    { label: 'Units hit', value: total(strike.units, whole) },
    { label: 'Manpower lost', value: total(strike.manpowerLost, whole) },
    { label: 'Equipment destroyed', value: total(strike.equipmentLost, whole) },
  ];
  for (const reason of strike.notRecorded) rows.push({ label: 'Not recorded', value: reason, tone: 'muted' });
  return rows;
}

const dayOf = (hour: number) => { const date = gameDate(hour); return date ? formatDate(date) : formatDateTime(hour); };

export function eventTooltip(at: FeedEvent, name: (tag: string) => string, stateName?: StateName): TooltipContent[] {
  const { event, item } = at;
  const title = eventDescription(at, name, stateName);
  const rows: TooltipRow[] = [
    { label: 'Date', value: SCRIPT_KINDS.has(event.kind) ? formatDateTime(event.hour) : dayOf(event.hour) },
    ...(SCRIPT_KINDS.has(event.kind) && item?.token ? [{ label: 'Content identity', value: item.token }] : []),
  ];
  const struck = event.strike;
  if (struck) rows.push(...strikeRows(struck, event.hour, stateName));
  if (event.decisionState) {
    switch (event.decisionState.kind) {
      case 'failed':
        rows.push({ label: 'Mission state', value: 'Failed' });
        rows.push({ label: 'Failure observed', value: formatDateTime(event.decisionState.observedHour) });
        break;
      case 'unsupported':
        rows.push({ label: 'Mission state', value: `Unsupported (native state ${event.decisionState.raw})`, tone: 'muted' });
        break;
      case 'notRecorded':
        rows.push({ label: 'Mission failure state', value: `Not recorded — ${event.decisionState.reason}`, tone: 'muted' });
        break;
    }
  }
  const cause = event.cause;
  if (cause) {
    rows.push({ label: 'How', value: cause.how ?? 'No observed operation' });
    if (cause.within) rows.push({ label: 'During', value: cause.within });
    if (cause.reason) rows.push({ label: 'Hostility reason', value: cause.reason });
    if (cause.conference) rows.push({ label: 'Peace conference', value: cause.conference });
    if (cause.state !== null) rows.push({ label: 'State', value: stateName?.(event.hour, cause.state) ?? `State ${cause.state} (name not recorded)` });
    for (const note of cause.notes) rows.push({ label: note });
  } else if (CAUSED_KINDS.has(event.kind)) {
    rows.push({ label: 'Cause', value: 'Not recorded in this recording', tone: 'muted' });
  }
  const note = [event.kind === 'warStarted' ? 'War entry; declaration or joining unknown.' : '',
    event.kind === 'ceasedToExist' ? 'Lost all owned states.' : '',
    event.kind === 'nukesIncreased' ? 'Stockpile increase.' : '',
    event.kind === 'nuclearStrike' ? 'Totals cover what the impact did before its scripted on_nuke_drop effects.' : ''].filter(Boolean).join(' ');
  rows.push(...reasonRows(item));
  if (item?.description) rows.push({ label: plainScripted(item.description) });
  return [{ title, icon: item?.icon ?? undefined, rows, note: note || undefined }];
}
