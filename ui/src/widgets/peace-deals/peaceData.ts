import type { HourSpan } from '../../generated/dashboard';
import type { LogDensity, LogRow } from '../../charts/log/logData';
import type { TooltipRow } from '../../charts/tooltip';
import type { NameAt } from '../../dashboard/countryFacts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { StateAt } from '../../dashboard/stateNamesData';
import { peaceActionLabel, peaceDealHour, peaceDealKnownAt, peaceDealAt, peaceDealIsBy } from '../../dashboard/peaceDealsData.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { setToken } from '../../dashboard/options.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { playbackRows } from '../../charts/playbackIntervals.ts';
import { logStates } from '../../charts/log/logStates.ts';
import type { PeaceDeal, PeaceDealsReply, PeaceItem } from '../../generated/dashboard';

/** `vp`: the state's victory points as an exact decimal, null when not recorded. */
export interface PeaceChip { key: string; hour: number; state?: number; vp?: string | null; tag?: string; missing?: string; note?: string; derived: boolean }
/** `warScore`: the recorded cost of this row's captured actions, null when none was captured.
 * `automatic`: the game's own transfer list, with no action or demand. `nameCountry`: the country the name names, at its hour. */
export interface PeaceActionRow { key: string; winner: string | null; name: string | null; token: string | null; outcome: boolean; automatic?: boolean;
  warScore: number | null; chips: PeaceChip[]; nameCountry?: NameAt }
/** `warEnd`: outcomes read from ownership when the war ended joined this conference. */
export interface PeaceCard { deal: PeaceDeal; actions: PeaceActionRow[]; hour: number; knownAt: number; by: boolean; warEnd?: boolean }
export type PeaceRow = LogRow<PeaceCard>;
/** Conference cards newest first, and the merged unrecorded stretches that could hide one. */
export interface PeaceLog { rows: readonly PeaceRow[]; gaps: readonly HourSpan[] }
/** `country` names the country an action's name names, at its hour; without it the game's label stands without the country. */
export const peaceActionName = (row: PeaceActionRow, country?: (at: NameAt) => string) => row.automatic ? 'Automatic transfer'
  : row.outcome ? 'Outcome; action unknown' : row.name == null ? 'Action name not recorded'
    : peaceActionLabel(row.name, row.nameCountry && country ? country(row.nameCountry) : null);
const UNRECORDED_ACTION = JSON.stringify(['unrecorded']);
const NO_ACTIONS = JSON.stringify(['noActions']);
const AUTOMATIC = JSON.stringify(['automatic']);
const actionCategory = (action: PeaceActionRow) => action.automatic ? AUTOMATIC
  : action.outcome || action.name == null ? UNRECORDED_ACTION : JSON.stringify(['action', action.token ?? action.name]);
const emptyCategory = (card: PeaceCard) => card.deal.captured && card.deal.captureUnavailable == null ? NO_ACTIONS : UNRECORDED_ACTION;

export function peaceActionChoices(log: PeaceLog | null): { value: string; label: string }[] {
  const choices = new Map<string, string>();
  for (const { item: card } of log?.rows ?? []) {
    if (!card.actions.length) {
      const category = emptyCategory(card);
      choices.set(category, category === NO_ACTIONS ? 'No actions recorded' : 'Action not recorded');
    }
    for (const action of card.actions) {
      const category = actionCategory(action);
      choices.set(category, category === UNRECORDED_ACTION ? 'Action not recorded' : peaceActionName(action));
    }
  }
  return [...choices].map(([key, label]) => ({ value: setToken(key), label }));
}

/** Filter once when the choice changes; playback keeps its binary cutoff. Gaps
 * are kept, and a matching conference keeps all of its actions. */
export function filterPeaceLog(log: PeaceLog, categories: readonly string[]): PeaceLog {
  if (!categories.length) return log;
  const chosen = new Set(categories);
  return { ...log, rows: log.rows.filter(({ item: card }) => card.actions.length
    ? card.actions.some(action => chosen.has(actionCategory(action))) : chosen.has(emptyCategory(card))) };
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
        countryFlag: action.winner == null ? undefined : country(action.winner), value: peaceActionName(action, at => countryAt(at.hour)(at.tag).name), strong: true },
      ...(action.warScore == null ? [] : [{ label: 'War score', value: usNumber.format(action.warScore), indent: true }]),
      ...action.chips.map(chip => ({ label: chipLabel(chip),
        countryFlag: chip.tag == null ? undefined : countryAt(chip.hour)(chip.tag),
        value: chip.note ?? (chip.vp != null ? `${chip.vp} VP` : undefined), indent: true })),
    ]),
  ];
}

/** Group native actions in their recorded order, or the dated fallback's items. */
export function peaceActions(deal: PeaceDeal): PeaceActionRow[] {
  const rows = new Map<string, PeaceActionRow>();
  const seen = new Map<string, Set<string>>();
  const covered = new Set<number>();
  const vp = stateVictoryPoints(deal);
  const add = (winner: string | null, name: string | null, token: string | null, kind: string, chip: PeaceChip, cost: number | null = null, nameCountry?: NameAt) => {
    const key = JSON.stringify([winner, kind, token, name, nameCountry?.tag ?? null]);
    let row = rows.get(key);
    if (!row) {
      row = { key, winner, name, token, outcome: kind === 'capturedOutcome' || kind === 'automaticOutcome', warScore: null, chips: [],
        ...(kind === 'automaticOutcome' ? { automatic: true } : {}), ...(nameCountry ? { nameCountry } : {}) };
      rows.set(key, row); seen.set(key, new Set());
    }
    if (cost != null) row.warScore = (row.warScore ?? 0) + cost;
    if (chip.state != null && chip.vp === undefined) chip.vp = vp.get(chip.state) ?? null;
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
      add(action.takerTag, action.name, action.nameKey ?? action.tokenName, `captured:${action.token}`, chip, action.cost,
        action.nameCountry != null ? { hour, tag: action.nameCountry } : undefined);
    }
  }
  // Captured outcomes are usable at the finalizer hour even when the graph
  // needs a later ownership comparison to establish their former owner.
  for (const transfer of deal.captured?.transfers ?? []) {
    if (transfer.state < 0 || covered.has(transfer.state)) continue;
    covered.add(transfer.state);
    add(transfer.to, null, null, transfer.automatic ? 'automaticOutcome' : 'capturedOutcome',
      { key: `state:${transfer.state}`, hour: transfer.hour, state: transfer.state, derived: false });
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
    const chip: PeaceChip = change.kind === 'stateTransfer' ? { key: `state:${change.state}`, hour, state: change.state, vp: change.victoryPoints, derived: item.derived }
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

/** Each transferred state's recorded victory points, from the deal's ownership
 * outcomes and transfer graph; a state recorded without them stays unknown. */
function stateVictoryPoints(deal: PeaceDeal): Map<number, string | null> {
  const points = new Map<number, string | null>();
  const note = (state: number, value: string | null) => { if (points.get(state) == null) points.set(state, value); };
  for (const item of deal.items) if (item.change.kind === 'stateTransfer') note(item.change.state, item.change.victoryPoints);
  for (const link of deal.transfers.links) for (const state of link.states) note(state.state, state.victoryPoints);
  return points;
}

/** An action row's states, their victory points (only when every state's is
 * recorded) and its war score. */
export function peaceActionTotals(row: PeaceActionRow): { states: number; vp: bigint | null; warScore: number | null } {
  const states = row.chips.filter(chip => chip.state != null);
  const vp = states.length && states.every(chip => chip.vp != null)
    ? states.reduce((sum, chip) => sum + BigInt(chip.vp!), 0n) : null;
  return { states: states.length, vp, warScore: row.warScore };
}

/** States most valuable first; one without recorded points follows them. */
export function chipsByValue(chips: readonly PeaceChip[]): PeaceChip[] {
  const value = (chip: PeaceChip) => chip.vp == null ? null : BigInt(chip.vp);
  return chips.map((chip, index) => ({ chip, index })).sort((a, b) => {
    const x = value(a.chip), y = value(b.chip);
    if (x == null || y == null) return x == null && y == null ? a.index - b.index : x == null ? 1 : -1;
    return x === y ? a.index - b.index : x > y ? -1 : 1;
  }).map(entry => entry.chip);
}

/** A grouped war-end outcome at the hour of a recorded conference that names each
 * of its winners as a winner and each loser as a loser is that conference's
 * settlement: one card, never two. An outcome matching no conference, or more
 * than one, stays its own card, whatever other outcomes join; a conference
 * takes every outcome matching it alone. */
export function joinWarEndOutcomes(deals: readonly PeaceDeal[]): { deal: PeaceDeal; warEnds: PeaceDeal[] }[] {
  const conferences = deals.filter(deal => deal.observation === 'recordedConference' && deal.captured != null);
  const joined = new Map<PeaceDeal, PeaceDeal[]>();
  const used = new Set<PeaceDeal>();
  for (const outcome of deals) {
    if (outcome.observation !== 'conferenceNotRecorded') continue;
    const hour = peaceDealHour(outcome);
    const matches = conferences.filter(conference => peaceDealHour(conference) === hour
      && outcome.winners.every(tag => conference.winners.includes(tag)) && outcome.losers.every(tag => conference.losers.includes(tag)));
    if (matches.length !== 1) continue;
    joined.set(matches[0], [...joined.get(matches[0]) ?? [], outcome]);
    used.add(outcome);
  }
  return deals.filter(deal => !used.has(deal)).map(deal => {
    const warEnds = joined.get(deal) ?? [];
    return { deal: warEnds.length ? { ...deal, items: [...deal.items, ...warEnds.flatMap(warEnd => warEnd.items)] } : deal, warEnds };
  });
}

/** Unrecorded hours that could hide a conference: those of the hourly finalizer
 * capture and ownership/war reads, which include every hour the recording missed.
 * The conference roster is read once a day; its hours between reads are its
 * cadence, not missing data. A conference that opens and closes between roster
 * reads still reaches the hourly finalizer capture or ownership comparison. */
export function peaceGaps(reply: PeaceDealsReply): HourSpan[] {
  const sorted = [...reply.unobserved.settlements, ...reply.unobserved.ownership, ...reply.unobserved.wars]
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
  const rows: PeaceRow[] = joinWarEndOutcomes(reply.deals).map(({ deal, warEnds }) => {
    const card: PeaceCard = { deal, actions: peaceActions(deal), hour: peaceDealHour(deal),
      knownAt: Math.max(peaceDealKnownAt(deal), ...warEnds.map(peaceDealKnownAt)), by: peaceDealIsBy(deal),
      ...(warEnds.length ? { warEnd: true } : {}) };
    return { key: JSON.stringify(['conference', deal.id]), hour: card.hour, by: card.by, item: card };
  });
  rows.sort((a, b) => b.hour - a.hour);
  return { rows, gaps: peaceGaps(reply) };
}
/** Binary cutoff: whole-campaign shaping and sorting never happen on a playback tick. */
export function peaceRowsAt(log: PeaceLog, until: number | null, from = -Infinity): readonly PeaceRow[] {
  if (until == null && from === -Infinity) return log.rows;
  const end = until ?? Infinity;
  let lo = 0, hi = log.rows.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (log.rows[mid].hour > end) lo = mid + 1; else hi = mid; }
  return log.rows.slice(lo).filter(row => row.hour >= from && row.item.knownAt <= end).map(row => {
    const deal = peaceDealAt(row.item.deal, end);
    return deal === row.item.deal ? row : { ...row, item: { ...row.item, deal } };
  });
}

function* peaceChanges(rows: readonly PeaceRow[]) {
  for (const row of rows) {
    yield Math.max(row.hour, row.item.knownAt);
    const unavailable = (row.item.deal as PeaceDeal & { captureUnavailableHour?: number }).captureUnavailableHour;
    if (unavailable != null) yield unavailable;
  }
}

export function peacePlayback(log: PeaceLog, from: number) {
  const selected = { ...log, rows: log.rows.filter(row => row.hour >= from) };
  return playbackRows(peaceChanges(selected.rows), hour => peaceRowsAt(selected, hour, from));
}

/** Every hour the log's rows change from `from` on (a deal listed, a capture's detail lost), ascending, once each. */
export function peaceChangeHours(log: PeaceLog, from: number): number[] {
  return [...new Set(peaceChanges(log.rows.filter(row => row.hour >= from)))].sort((a, b) => a - b);
}

/** Following, the log over a block from `hour` through `last` (`changes` ascending): each conference's states, each from
 * the hour it shows to the hour the next replaces it (`logStates`), so the list draws the block once and its clock shows
 * at every hour exactly that hour's cards, newest first. */
export function peaceBlockRows(log: PeaceLog, from: number, hour: number, last: number, changes: readonly number[]): PeaceRow[] {
  const hours = [hour, ...changes.filter(change => change > hour && change <= last)];
  const place = new Map(peaceRowsAt(log, last, from).map((row, index) => [row.key, index]));
  const at = (row: PeaceRow) => place.get(row.id ?? row.key) ?? -1;
  return logStates(hours, step => peaceRowsAt(log, step, from), row => JSON.stringify(row.item.deal), (a, b) => b.hour - a.hour || at(a) - at(b));
}
/** The gaps within the shown time, clipped so a later hour never shows. */
export function peaceGapsIn(gaps: readonly HourSpan[], until: number, from = -Infinity): HourSpan[] {
  const shown: HourSpan[] = [];
  for (const gap of gaps) {
    if (gap.firstHour > until) break;
    if (gap.lastHour >= from) shown.push({ firstHour: Math.max(from, gap.firstHour), lastHour: Math.min(until, gap.lastHour) });
  }
  return shown;
}
/** One hint for every gap in the shown time, never one row per gap. */
export function peaceGapHint(gaps: readonly HourSpan[]): string | null {
  if (!gaps.length) return null;
  const between = `between ${formatDateTime(gaps[0].firstHour)} and ${formatDateTime(gaps.at(-1)!.lastHour)}`;
  return gaps.length === 1 ? `Not fully recorded ${between}.` : `${gaps.length} stretches not fully recorded ${between}.`;
}
export function derivedNote(deal: PeaceDeal): string {
  if (!deal.derived) return deal.captureUnavailable != null ? 'Outcomes not recorded' : '';
  let before = Infinity, after = -Infinity;
  for (const item of deal.items) { before = Math.min(before, item.dates.beforeHour); after = Math.max(after, item.dates.afterHour); }
  return Number.isFinite(before) ? `Estimated between ${formatDateTime(before)} and ${formatDateTime(after)}`
    : 'Outcomes not recorded';
}
export const conferenceLabel = (deal: PeaceDeal) => deal.observation === 'recordedConference' ? null
  : deal.observation === 'conferenceNotRecorded' ? 'War-end outcomes' : 'Conference unknown';
/** Where a joined card's states and victory points came from. */
export const WAR_END_NOTE = 'States and victory points read from ownership when the war ended.';
export function peaceWanted(rows: readonly PeaceRow[]): { countries: NameAt[]; states: StateAt[] } {
  const countries = new Map<string, NameAt>();
  const states = new Map<string, StateAt>();
  const country = (hour: number, tag: string) => countries.set(JSON.stringify([hour, tag]), { hour, tag });
  for (const { item: card } of rows) {
    for (const tag of [...card.deal.winners, ...card.deal.losers]) country(card.hour, tag);
    for (const action of card.actions) {
      if (action.winner != null) country(card.hour, action.winner);
      if (action.nameCountry) country(action.nameCountry.hour, action.nameCountry.tag);
      for (const chip of action.chips) {
        if (chip.tag != null) country(chip.hour, chip.tag);
        if (chip.state != null) states.set(JSON.stringify([chip.hour, chip.state]), { hour: chip.hour, state: chip.state });
      }
    }
  }
  return { countries: [...countries.values()], states: [...states.values()] };
}
/** Offscreen estimate; the log measures actual wrapping only for drawn cards. */
export function peaceRowHeight({ item: card }: PeaceRow, density: LogDensity): number {
  const compact = density === 'compact';
  if (!card.actions.length) return compact ? 30 : 36;
  return (compact ? 50 : 60) + card.actions.length * (compact ? 50 : 58) + (card.warEnd || derivedNote(card.deal) ? 18 : 0);
}
/** No war name is recorded; the widget's own description leads with the sides. */
export function peaceDescription({ item: card }: PeaceRow): string {
  return [`${card.by ? 'by ' : ''}${formatDateTime(card.hour)}`, conferenceLabel(card.deal), derivedNote(card.deal),
    card.warEnd ? WAR_END_NOTE : null].filter(Boolean).join(' · ');
}
