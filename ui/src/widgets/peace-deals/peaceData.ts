import type { HourSpan, PeaceDeal, PeaceDealsReply, PeaceItem } from '../../generated/dashboard';
import type { LogDensity, LogRow } from '../../charts/log/logData';
import type { TooltipRow } from '../../charts/tooltip';
import type { NameAt } from '../../dashboard/countryFacts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { StateAt } from '../../dashboard/stateNamesData';
import { peaceDealHour, peaceDealKnownAt, peaceDealAt, peaceDealIsBy, peaceWarName } from '../../dashboard/peaceDealsData.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { setToken } from '../../dashboard/options.ts';

export interface PeaceChip { key: string; hour: number; state?: number; tag?: string; missing?: string; note?: string; derived: boolean }
export interface PeaceActionRow { key: string; winner: string | null; name: string | null; token: string | null; outcome: boolean; chips: PeaceChip[] }
export interface PeaceCard { deal: PeaceDeal; actions: PeaceActionRow[]; hour: number; knownAt: number; by: boolean }
export type PeaceLogEntry = { kind: 'card'; card: PeaceCard } | { kind: 'gap'; span: HourSpan };
export interface PeaceLog { rows: readonly LogRow<PeaceLogEntry>[]; gaps: readonly HourSpan[] }
export const peaceHasConferences = (rows: readonly LogRow<PeaceLogEntry>[]) => rows.some(row => row.item.kind === 'card');
export const peaceActionName = (row: PeaceActionRow) => row.outcome ? 'Outcome; action unknown' : row.name ?? 'Action name not recorded';
const UNRECORDED_ACTION = JSON.stringify(['unrecorded']);
const NO_ACTIONS = JSON.stringify(['noActions']);
const actionCategory = (action: PeaceActionRow) => action.outcome || action.name == null ? UNRECORDED_ACTION : JSON.stringify(['action', action.token ?? action.name]);
const emptyCategory = (card: PeaceCard) => card.deal.captured && card.deal.captureUnavailable == null ? NO_ACTIONS : UNRECORDED_ACTION;

export function peaceActionChoices(log: PeaceLog | null): { value: string; label: string }[] {
  const choices = new Map<string, string>();
  for (const row of log?.rows ?? []) {
    if (row.item.kind !== 'card') continue;
    if (!row.item.card.actions.length) {
      const category = emptyCategory(row.item.card);
      choices.set(category, category === NO_ACTIONS ? 'No actions recorded' : 'Action not recorded');
    }
    for (const action of row.item.card.actions) {
      const category = actionCategory(action);
      choices.set(category, category === UNRECORDED_ACTION ? 'Action not recorded' : peaceActionName(action));
    }
  }
  return [...choices].map(([key, label]) => ({ value: setToken(key), label }));
}

/** Filter once when the choice changes; playback keeps its binary cutoff. Gaps
 * remain visible, and a matching conference keeps all of its actions. */
export function filterPeaceLog(log: PeaceLog, categories: readonly string[]): PeaceLog {
  if (!categories.length) return log;
  const chosen = new Set(categories);
  return { ...log, rows: log.rows.filter(row => row.item.kind === 'gap' || (row.item.card.actions.length
    ? row.item.card.actions.some(action => chosen.has(actionCategory(action))) : chosen.has(emptyCategory(row.item.card)))) };
}

/** Width fitting never cuts the complete participant or outcome tooltip. */
export function peaceTooltipRows(card: PeaceCard, countryAt: (hour: number) => CountryLabel, chipLabel: (chip: PeaceChip) => string): TooltipRow[] {
  const country = countryAt(card.hour);
  return [
    { label: 'Winners', strong: true },
    ...card.deal.winners.map(tag => ({ label: country(tag).name, countryFlag: country(tag), indent: true })),
    { label: 'Losers', strong: true },
    ...card.deal.losers.map(tag => ({ label: country(tag).name, countryFlag: country(tag), indent: true })),
    ...card.actions.flatMap(action => [
      { label: action.winner == null ? 'Taker not recorded' : country(action.winner).name,
        countryFlag: action.winner == null ? undefined : country(action.winner), value: peaceActionName(action), strong: true },
      ...action.chips.map(chip => ({ label: chipLabel(chip),
        countryFlag: chip.tag == null ? undefined : countryAt(chip.hour)(chip.tag),
        value: chip.note, indent: true })),
    ]),
  ];
}

/** Group native actions in their recorded order, or the dated fallback's items. */
export function peaceActions(deal: PeaceDeal): PeaceActionRow[] {
  const rows = new Map<string, PeaceActionRow>();
  const seen = new Map<string, Set<string>>();
  const covered = new Set<number>();
  const add = (winner: string | null, name: string | null, token: string | null, kind: string, chip: PeaceChip) => {
    const key = JSON.stringify([winner, kind, token, name]);
    let row = rows.get(key);
    if (!row) { row = { key, winner, name, token, outcome: kind === 'capturedOutcome', chips: [] }; rows.set(key, row); seen.set(key, new Set()); }
    if (!seen.get(key)!.has(chip.key)) { seen.get(key)!.add(chip.key); row.chips.push(chip); }
  };
  if (deal.captured) {
    const hour = deal.captured.hour;
    for (const action of deal.captured.actions) {
      const state = action.state != null && action.state >= 0 ? action.state : undefined;
      const tag = action.countryOperand != null ? action.countryOperandTag ?? undefined
        : action.state == null ? action.giverTag ?? undefined : undefined;
      if (state != null) covered.add(state);
      const chip: PeaceChip = state != null ? { key: `state:${state}`, hour, state, derived: false }
        : tag != null ? { key: JSON.stringify(['country', tag]), hour, tag, derived: false }
          : { key: JSON.stringify(['unrecorded', action.country, action.turn, action.ordinal]), hour, missing: 'Target not recorded', derived: false };
      add(action.takerTag, action.name, action.nameKey ?? action.tokenName, `captured:${action.token}`, chip);
    }
  }
  // Captured outcomes are usable at the finalizer hour even when the graph
  // needs a later ownership comparison to establish their former owner.
  for (const transfer of deal.captured?.transfers ?? []) {
    if (transfer.state < 0 || covered.has(transfer.state)) continue;
    covered.add(transfer.state);
    add(transfer.to, null, null, 'capturedOutcome', { key: `state:${transfer.state}`, hour: transfer.hour, state: transfer.state, derived: false });
  }
  // The final subject may be newly allocated, so an action's country operand
  // cannot identify this output or establish which action produced it.
  for (const subject of deal.captured?.subjects ?? []) {
    add(subject.overlordTag, null, null, 'capturedOutcome', {
      key: JSON.stringify(['subject', subject.subject]), hour: deal.captured!.hour,
      ...(subject.subjectTag != null ? { tag: subject.subjectTag } : { missing: 'Subject country not recorded' }),
      note: 'Subject', derived: false,
    });
  }
  const addItem = (item: PeaceItem) => {
    const hour = item.dates.afterHour;
    const change = item.change;
    const chip: PeaceChip = change.kind === 'stateTransfer' ? { key: `state:${change.state}`, hour, state: change.state, derived: item.derived }
      : { key: JSON.stringify(['country', change.kind === 'newSubject' ? change.subject : change.country]), hour,
        tag: change.kind === 'newSubject' ? change.subject : change.country, derived: item.derived };
    add(item.winner, item.action.name, item.action.key, item.action.kind, chip);
  };
  for (const item of deal.items) {
    if (deal.captured && item.change.kind === 'stateTransfer' && covered.has(item.change.state)) continue;
    addItem(item);
    if (item.change.kind === 'stateTransfer') covered.add(item.change.state);
  }
  return [...rows.values()];
}

/** Only the inputs that establish conferences or transfers define this log's gaps. */
export function peaceGaps(reply: PeaceDealsReply): HourSpan[] {
  const sorted = [...reply.unobserved.conferences, ...reply.unobserved.settlements, ...reply.unobserved.ownership, ...reply.unobserved.wars]
    .sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
  const spans: HourSpan[] = [];
  for (const span of sorted) {
    const last = spans.at(-1);
    if (last && span.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, span.lastHour);
    else spans.push({ ...span });
  }
  return spans;
}
export function preparePeaceLog(reply: PeaceDealsReply): PeaceLog {
  const gaps = peaceGaps(reply);
  const rows: LogRow<PeaceLogEntry>[] = reply.deals.map(deal => {
    const card = { deal, actions: peaceActions(deal), hour: peaceDealHour(deal), knownAt: peaceDealKnownAt(deal), by: peaceDealIsBy(deal) };
    return { key: JSON.stringify(['conference', deal.id]), hour: card.hour, by: card.by, item: { kind: 'card', card } };
  });
  for (const span of gaps) rows.push({ key: JSON.stringify(['gap', span.firstHour, span.lastHour]), hour: span.lastHour, item: { kind: 'gap', span } });
  rows.sort((a, b) => b.hour - a.hour);
  return { rows, gaps };
}
/** Binary cutoff: whole-campaign shaping and sorting never happen on a playback tick. */
export function peaceRowsAt(log: PeaceLog, until: number | null, from = -Infinity): readonly LogRow<PeaceLogEntry>[] {
  if (until == null && from === -Infinity) return log.rows;
  const end = until ?? Infinity;
  let lo = 0, hi = log.rows.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (log.rows[mid].hour > end) lo = mid + 1; else hi = mid; }
  const rows = log.rows.slice(lo).filter(row => row.hour >= from
    && (row.item.kind === 'gap' || row.item.card.knownAt <= end))
    .map(row => row.item.kind === 'gap' && row.item.span.firstHour < from
      ? { ...row, item: { kind: 'gap' as const, span: { ...row.item.span, firstHour: from } } } : row)
    .map(row => {
      if (row.item.kind !== 'card') return row;
      const deal = peaceDealAt(row.item.card.deal, end);
      return deal === row.item.card.deal ? row : { ...row, item: { kind: 'card' as const, card: { ...row.item.card, deal } } };
    });
  lo = 0; hi = log.gaps.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (log.gaps[mid].firstHour <= end) lo = mid + 1; else hi = mid; }
  const gap = log.gaps[lo - 1];
  if (gap && gap.lastHour > end && end >= from) rows.unshift({ key: JSON.stringify(['gap', gap.firstHour, gap.lastHour]), hour: end, item: { kind: 'gap', span: { firstHour: Math.max(from, gap.firstHour), lastHour: end } } });
  return rows;
}
export const gapLabel = (span: HourSpan) => `Not recorded between ${formatDateTime(span.firstHour)} and ${formatDateTime(span.lastHour)}`;
export function derivedNote(deal: PeaceDeal): string {
  if (!deal.derived) return deal.captureUnavailable != null ? 'Outcomes not recorded' : '';
  let before = Infinity, after = -Infinity;
  for (const item of deal.items) { before = Math.min(before, item.dates.beforeHour); after = Math.max(after, item.dates.afterHour); }
  return Number.isFinite(before) ? `Estimated between ${formatDateTime(before)} and ${formatDateTime(after)}`
    : 'Outcomes not recorded';
}
export const conferenceLabel = (deal: PeaceDeal) => deal.observation === 'recordedConference' ? null
  : deal.observation === 'conferenceNotRecorded' ? 'War-end outcomes' : 'Conference unknown';
export function peaceWanted(rows: readonly LogRow<PeaceLogEntry>[]): { countries: NameAt[]; states: StateAt[] } {
  const countries = new Map<string, NameAt>();
  const states = new Map<string, StateAt>();
  const country = (hour: number, tag: string) => countries.set(JSON.stringify([hour, tag]), { hour, tag });
  for (const row of rows) {
    if (row.item.kind !== 'card') continue;
    const { card } = row.item;
    for (const tag of [...card.deal.winners, ...card.deal.losers]) country(card.hour, tag);
    for (const action of card.actions) {
      if (action.winner != null) country(card.hour, action.winner);
      for (const chip of action.chips) {
        if (chip.tag != null) country(chip.hour, chip.tag);
        if (chip.state != null) states.set(JSON.stringify([chip.hour, chip.state]), { hour: chip.hour, state: chip.state });
      }
    }
  }
  return { countries: [...countries.values()], states: [...states.values()] };
}
/** Offscreen estimate; the log measures actual wrapping only for drawn cards. */
export function peaceRowHeight(row: LogRow<PeaceLogEntry>, density: LogDensity): number {
  const compact = density === 'compact';
  if (row.item.kind === 'gap') return compact ? 38 : 46;
  const { card } = row.item;
  return (compact ? 70 : 88) + card.actions.length * (compact ? 24 : 28) + (derivedNote(card.deal) ? 20 : 0) + (conferenceLabel(card.deal) ? 20 : 0);
}
export function peaceDescription(row: LogRow<PeaceLogEntry>): string {
  if (row.item.kind === 'gap') return gapLabel(row.item.span);
  const { card } = row.item;
  return [peaceWarName(card.deal), `${card.by ? 'by ' : ''}${formatDateTime(card.hour)}`, conferenceLabel(card.deal), derivedNote(card.deal)].filter(Boolean).join(' · ');
}
