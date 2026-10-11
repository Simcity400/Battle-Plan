import type { LogRow } from '../../charts/log/logData';
import { newestFirst } from '../../charts/log/logData.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { drawableIcon } from '../../dashboard/artwork.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { ideaInfo, localisedText, modifierFormat } from '../../dashboard/ideaContent.ts';
import { ideaTooltip, ideaTooltipSources } from '../../dashboard/ideaTooltip.ts';
import { int, object, opaqueTag } from '../../dashboard/validation.ts';
import type { EventFeedItem, EventFeedReply, EventFeedRow, EventFeedWindow } from '../../generated/dashboard';
import { CATEGORIES } from './definition.ts';

const NATIVE_KINDS = new Set(['warDeclared', 'warJoined', 'civilWarWarTransferred', 'annexed']);
const KINDS = new Set([...NATIVE_KINDS, 'warStarted', 'peace', 'capitulated', 'ceasedToExist', 'factionJoined', 'factionLeft',
  'becameSubject', 'guaranteeGranted', 'rulingPartyChanged', 'lawChanged', 'advisorHired', 'focusCompleted',
  'projectFinished', 'nukesIncreased']);
const text = (value: unknown): value is string | null => value === null || typeof value === 'string';
const roles = (row: Record<string, unknown>): boolean => {
  if (!NATIVE_KINDS.has(row.kind as string)) return row.participants === undefined || row.participants === null;
  return row.exact === true && row.category === 'warsPeace' && Array.isArray(row.participants)
    && row.participants.length === 2 && row.participants.every(value => value === null || opaqueTag(value))
    && JSON.stringify(row.participants.filter(value => value !== null)) === JSON.stringify(row.tags);
};
const span = (value: unknown): value is EventFeedWindow => {
  if (!object(value) || !int(value.firstHour) || !int(value.lastHour) || value.firstHour > value.lastHour
    || !int(value.count) || value.count <= 0 || !object(value.categoryCounts)) return false;
  const counts = value.categoryCounts;
  return CATEGORIES.every(category => { const count = counts[category.value]; return int(count) && count >= 0; })
    && CATEGORIES.reduce((sum, category) => sum + (counts[category.value] as number), 0) === value.count;
};

/** Validate identities, dates, item references and tooltip content before drawing. */
export function validateEventFeedReply(value: unknown): EventFeedReply {
  const categories = new Set<string>(CATEGORIES.map(category => category.value));
  const first = object(value) && int(value.firstHour) ? value.firstHour : null;
  const last = object(value) && int(value.lastHour) ? value.lastHour : null;
  const items = object(value) && Array.isArray(value.items) ? value.items : null;
  if (!object(value) || typeof value.branchId !== 'string' || !int(value.firstHour) || !int(value.lastHour)
    || value.firstHour > value.lastHour || !Array.isArray(value.windows) || !value.windows.every(span)
    || !value.windows.every((window, index, all) => !index || all[index - 1].lastHour < window.firstHour)
    || !Array.isArray(value.items) || !value.items.every(item => object(item) && text(item.token) && text(item.name)
      && text(item.description) && 'icon' in item && (item.idea === null || ideaInfo(item.idea)))
    || !Array.isArray(value.rows) || !value.rows.every(row => object(row) && typeof row.id === 'string' && row.id.length > 0
      && int(row.hour) && int(row.readHour) && int(row.nameHour) && row.hour <= row.readHour
      && row.nameHour <= row.hour && first !== null && last !== null && row.hour >= first && row.hour <= last
      && typeof row.exact === 'boolean' && typeof row.category === 'string' && categories.has(row.category)
      && typeof row.kind === 'string' && KINDS.has(row.kind) && Array.isArray(row.tags)
      && (NATIVE_KINDS.has(row.kind) || row.tags.length > 0) && row.tags.every(opaqueTag) && roles(row)
      && (row.item === null || (int(row.item) && row.item >= 0 && items !== null && row.item < items.length))
      && (row.amount === null || (int(row.amount) && row.amount > 0)))
    || new Set(value.rows.map(row => row.id)).size !== value.rows.length
    || !Array.isArray(value.unavailable) || !value.unavailable.every(item => object(item) && typeof item.kind === 'string'
      && typeof item.label === 'string' && typeof item.reason === 'string' && (item.count === null || (int(item.count) && item.count >= 0)))
    || !Array.isArray(value.modifiers) || !value.modifiers.every(modifierFormat)
    || !Array.isArray(value.texts) || !value.texts.every(localisedText)
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string')) {
    throw new Error('The event feed reply is malformed.');
  }
  const reply = value as unknown as EventFeedReply;
  for (const item of reply.items) item.icon = drawableIcon(item.icon);
  return reply;
}

export interface FeedEvent { event: EventFeedRow; item: EventFeedItem | null; reply: EventFeedReply }
export interface FeedFilter {
  categories: ReadonlySet<string>;
  country: (tag: string, hour: number) => boolean;
  firstHour: number | null;
  allCountries?: boolean;
}

export const eventCountryName = (tag: string, label: { name: string; known?: boolean; nameFailed?: boolean }): string =>
  label.nameFailed ? `${tag} (name not recorded)` : label.known === undefined ? `${tag} (loading name…)`
    : label.known ? label.name : `${tag} (name not known)`;

/** Country/group matching for a removed member uses the identity supplying its
 * last recorded membership, rather than the hour it was already absent. */
export const eventCountryHour = (event: EventFeedRow): number =>
  event.kind === 'ceasedToExist' || event.kind === 'factionLeft' ? event.nameHour : event.hour;

/** Prepare each page/filter once. Playback only cuts this ordered result by its
 * supplying reads; it never repeats country matching or sorting on each tick. */
export function filteredEventRows(reply: EventFeedReply, filter: FeedFilter): LogRow<FeedEvent>[] {
  return newestFirst(reply.rows.filter(event => (filter.firstHour === null || event.hour >= filter.firstHour)
    && filter.categories.has(event.category)
    && (filter.allCountries === true || event.tags.some(tag => filter.country(tag, eventCountryHour(event)))))
    .map(event => ({ key: event.id, id: event.id, hour: event.hour, by: !event.exact,
      revealedHour: event.readHour,
      item: { event, item: event.item === null ? null : reply.items[event.item], reply } })));
}

/** Country-filtered manifest counts remain possible matches until loaded. */
export function eventWindowCount(window: EventFeedWindow, categories: ReadonlySet<string>): number {
  return CATEGORIES.reduce((sum, category) => sum + (categories.has(category.value) ? window.categoryCounts[category.value] : 0), 0);
}

/** Sentence parts retain each country's own flag beside its name. */
export type SentencePart = { text: string } | { tag: string };
export function eventSentence(at: FeedEvent): SentencePart[] {
  const { event, item } = at;
  const role = (index: number): SentencePart => {
    const tag = event.participants ? event.participants[index] : event.tags[index];
    return tag == null ? { text: `${index === 0 ? 'A' : 'a'} country not identified` } : { tag };
  };
  const country = role(0);
  const other = role(1);
  const named = item?.name ?? 'an item whose name is unavailable';
  const words = (before: string, after = ''): SentencePart[] => [country, { text: before }, other, ...(after ? [{ text: after }] : [])];
  const counterparts = (before: string): SentencePart[] => [country, { text: before }, ...
    (event.tags.length < 2 ? [{ text: 'a country not known' }] : event.tags.slice(1).flatMap((tag, index, tags): SentencePart[] =>
      [...(index ? [{ text: index === tags.length - 1 ? ' and ' : ', ' }] : []), { tag }]))];
  const single = (action: string): SentencePart[] => [country, { text: action }];
  switch (event.kind) {
    case 'warDeclared': return words(' declared war on ');
    case 'warJoined': return words(' joined the war against ');
    case 'civilWarWarTransferred': return words(' inherited a war against ');
    case 'annexed': return words(' annexed ');
    case 'warStarted': return counterparts(' became at war with ');
    case 'peace': return counterparts(' was no longer at war with ');
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
  }
}

export function eventDescription(at: FeedEvent, name: (tag: string) => string): string {
  return eventSentence(at).map(part => 'tag' in part ? name(part.tag) : part.text).join('');
}

/** Missing captures and unsupported forms have one explicit presentation adapter. */
export function notRecorded(reply: EventFeedReply): string[] {
  return reply.unavailable.map(item => `${item.label}: Not recorded — ${item.reason}${item.count === null ? '' : ` (${item.count.toLocaleString()} counted)`}`);
}

export function eventTooltip(at: FeedEvent, name: (tag: string) => string): TooltipContent[] {
  const { event, item, reply } = at;
  const title = eventDescription(at, name);
  const rows: TooltipRow[] = [
    { label: event.exact ? 'Native date' : 'First observed by', value: formatDateTime(event.hour) },
    { label: 'Read supplying this event', value: formatDateTime(event.readHour) },
  ];
  if (event.nameHour !== event.hour) rows.push({ label: 'Country last observed', value: formatDateTime(event.nameHour) });
  if (!item?.icon) rows.push({ label: 'Artwork', value: 'Not available from the recorded content', tone: 'muted' });
  const note = [!event.exact ? 'The event happened by this observation; its exact hour is not recorded.' : '',
    event.kind === 'warStarted' ? 'The recording does not distinguish a declaration from joining an existing war.' : '',
    event.kind === 'ceasedToExist' ? 'The recorded owned-state count fell to zero. The recording cannot tell whether this was an annexation or another loss of all owned states.' : '',
    event.kind === 'nukesIncreased' ? 'This is a stockpile increase. Nuclear drops are not recorded.' : ''].filter(Boolean).join(' ');
  if (item?.idea) return [ideaTooltip({ title, icon: item.icon }, item.idea, ideaTooltipSources(reply, name),
    item.description, rows, note, event.nameHour)];
  if (item?.description) rows.push({ label: item.description });
  return [{ title, icon: item?.icon ?? undefined, rows, note: note || undefined }];
}

/** Only event-bearing windows overlapping the selected date interval. */
export function feedWindows(windows: readonly EventFeedWindow[], hour: number, firstHour: number | null): { key: string; index: number; window: EventFeedWindow }[] {
  return windows.flatMap((window, index) => window.firstHour <= hour && (firstHour === null || window.lastHour >= firstHour)
    ? [{ key: String(index), index, window }] : []).reverse();
}
