import { IntervalIndex } from '../charts/intervalIndex.ts';
import { clipToSpans, runAt } from '../charts/series.ts';
import type { Run, Span } from '../charts/series';
import { tooltipHeight, tooltipRoom, tooltipRowHeight } from '../charts/tooltip.ts';
import type { TooltipContent, TooltipRow } from '../charts/tooltip';
import type { FactionGroup, FactionTotalsParams, FactionTotalsReply, MembershipRun } from '../generated/dashboard';
import type { CountryLabel } from './countryFlagsAt';
import type { CountrySelection } from './model';
import { compareTags } from './tagOrder.ts';
import { formatDateTime } from './calendar.ts';
import { factionNameAt as recordedFactionNameAt, factionNameLabel } from './factionNames.ts';
import type { FactionName } from './factionNames';

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value != null;
const hour = (value: unknown) => Number.isInteger(value) && Number(value) >= -2147483648 && Number(value) <= 2147483647;
const memberValue = (value: unknown): boolean => value === null || (typeof value === 'string' && /^-?(0|[1-9]\d{0,18})$/.test(value)
  && BigInt(value) >= -(1n << 63n) && BigInt(value) < (1n << 63n));
const spans = (value: unknown, ordered = true): boolean => Array.isArray(value) && value.every((span, index) => object(span)
  && hour(span.firstHour) && hour(span.lastHour) && Number(span.firstHour) <= Number(span.lastHour)
  && (!ordered || index === 0 || value[index - 1].lastHour < Number(span.firstHour)));

/** All faction measures use one product contract and one dated series adapter. */
export function validateFactionTotalsReply(data: unknown): FactionTotalsReply {
  const reply = data as FactionTotalsReply;
  if (!object(reply) || typeof reply.branchId !== 'string'
    || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string')
    || !(reply.unit === null || typeof reply.unit === 'string')
    || !(reply.unavailableReason === null || typeof reply.unavailableReason === 'string')
    || !['hour', 'end', 'ever'].includes(reply.membershipMode)
    || !spans(reply.recorded) || !spans(reply.disjoint)
    || !Array.isArray(reply.series) || new Set(reply.series.map(series => series?.id)).size !== reply.series.length
    || !reply.series.every(series => object(series) && typeof series.id === 'string'
      && (series.unavailableReason === null || typeof series.unavailableReason === 'string')
      && (series.dissolutionHour === null || hour(series.dissolutionHour)) && spans(series.runs)
      && series.runs.every(run => Number.isSafeInteger(run.value) && typeof run.startsSegment === 'boolean')
      && spans(series.names) && series.names.every(run => (run.name === null || typeof run.name === 'string')
        && (run.reason === null || typeof run.reason === 'string'))
      && spans(series.countedMembers, false) && series.countedMembers.every(run => typeof run.tag === 'string')
      && spans(series.members, false) && series.members.every(run => typeof run.tag === 'string') && spans(series.membership))
    || (reply.memberTracks !== undefined && (!Array.isArray(reply.memberTracks)
      || new Set(reply.memberTracks.map(track => track?.tag)).size !== reply.memberTracks.length
      || !reply.memberTracks.every(track => object(track) && typeof track.tag === 'string' && spans(track.runs)
        && track.runs.every(run => memberValue(run.value) && typeof run.startsSegment === 'boolean')
        && spans(track.identities) && track.identities.every(identity => [identity.slot, identity.countryId]
          .every(id => Number.isInteger(id) && id >= 0 && id <= 4294967295)))))) {
    throw new Error('The faction totals reply is malformed.');
  }
  return reply;
}

export function factionTotalsRequest(branchId: string, selection: CountrySelection,
  source: FactionTotalsParams['source'], max: number): FactionTotalsParams | null {
  if (selection.kind === 'factions') return { branchId, source, membership: selection.membership, top: Math.max(1, Math.min(max, selection.count)) };
  if (selection.kind === 'group' && selection.aggregate && selection.group.type === 'faction') {
    return { branchId, source, membership: selection.membership, top: 1, factionId: selection.group.id };
  }
  return null;
}

/** Member and name history stay independent of numeric/top-N runs, so hover can
 * describe the counted roster and recorded name at its own hour. */
export function factionTotalsModel(reply: FactionTotalsReply) {
  const series = new Map(reply.series.map(series => [series.id, {
    runs: series.runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour, value: run.value,
      ...(run.startsSegment ? { breakBefore: true } : {}) })) as Run[],
    membership: series.membership, names: series.names, members: new IntervalIndex<MembershipRun>(series.countedMembers), unavailableReason: series.unavailableReason, dissolutionHour: series.dissolutionHour,
  }] as const));
  return { series, membershipMode: reply.membershipMode, memberLookups: new Map<string, { hour: number; tags: string[] }>() };
}
export type FactionTotalsModel = ReturnType<typeof factionTotalsModel>;

export function factionLines(model: FactionTotalsModel | null, range: Span) {
  return model ? [...model.series].flatMap(([tag, series]) => {
    const runs = clipToSpans(clipToSpans(series.runs, series.membership), [range]);
    return runs.length ? [{ tag, runs }] : [];
  }) : [];
}

export interface FactionNameSettings {
  factions: ReadonlyMap<string, FactionGroup>;
  names(faction: FactionGroup, hour?: number): FactionName;
}

function namedFaction(model: FactionTotalsModel | null, id: string, hour: number, settings?: FactionNameSettings): FactionName {
  const faction = settings?.factions.get(id);
  return faction && settings ? settings.names(faction, hour)
    : recordedFactionNameAt({ name: null, nameReason: 'Not recorded', names: model?.series.get(id)?.names ?? [] }, hour);
}

export function factionNameAt(model: FactionTotalsModel | null, id: string, hour: number, settings?: FactionNameSettings): string {
  const named = namedFaction(model, id, hour, settings);
  return factionNameLabel({ name: named.name, nameReason: named.reason, names: [] });
}

export function factionNameReasonAt(model: FactionTotalsModel | null, id: string, hour: number, settings?: FactionNameSettings): string | null {
  return namedFaction(model, id, hour, settings).reason;
}

/** Fixed end rosters retain their supplying hour when a faction dissolved. */
export function factionMembershipNote(model: FactionTotalsModel | null, id: string): string | null {
  const hour = model?.series.get(id)?.dissolutionHour;
  return hour == null ? null : `last members before it dissolved on ${formatDateTime(hour)}`;
}

/** The existing name label also carries a series note where the kit has room. */
export function factionLabelAt(model: FactionTotalsModel | null, id: string, hour: number, settings?: FactionNameSettings): string {
  const name = factionNameAt(model, id, hour, settings);
  const note = factionMembershipNote(model, id);
  return note ? `${name} · ${note}` : name;
}

export function factionMembersAt(model: FactionTotalsModel | null, id: string, hour: number): readonly string[] {
  if (!model) return [];
  const at = Math.floor(hour);
  const cached = model.memberLookups.get(id);
  if (cached?.hour === at) return cached.tags;
  const tags = [...new Set(model.series.get(id)?.members.at(at).map(run => run.tag) ?? [])].sort(compareTags);
  // Tooltip redraws as visible labels arrive; the roster at its hour is unchanged.
  model.memberLookups.set(id, { hour: at, tags });
  return tags;
}

/** The visible member preview follows the tooltip room in this window. */
export function factionMemberLabelLimit(height: number, factions: number, title: string): number {
  const header = tooltipHeight({ title, subtitle: 'Date and measure', rows: [] }) + 34;
  const rows = Math.floor(Math.max(0, tooltipRoom(height) - header) / tooltipRowHeight({ label: '', short: true }));
  return Math.max(0, rows - factions * 3);
}

export function factionVisibleMemberTags(model: FactionTotalsModel | null, hour: number, ids: readonly string[]): string[] {
  return [...new Set(ids.flatMap(id => factionMembersAt(model, id, hour)))];
}

/** Totals and roster counts stay ahead of the window-fitted preview. The spoken
 * form names every recorded member, even when the window fits only a few. */
export function factionTooltipForms(model: FactionTotalsModel, ids: readonly string[], hour: number, label: CountryLabel,
  color: (id: string) => string, format: (value: number) => string, title: string, subtitle: string, note?: string, maxMembers = Infinity,
  names?: FactionNameSettings): TooltipContent[] {
  const compact: TooltipRow[] = [];
  const rows: TooltipRow[] = [];
  const spoken: string[] = [title, subtitle];
  let remaining = maxMembers;
  for (const id of ids) {
    const series = model.series.get(id);
    const run = runAt(series?.runs ?? [], Math.floor(hour));
    const name = factionNameAt(model, id, hour, names);
    if (!run) {
      if (series?.unavailableReason) {
        const unavailable: TooltipRow[] = [{ label: name, value: 'Total unavailable', strong: true, short: true },
          { label: model.membershipMode === 'end' ? 'Members at the end' : model.membershipMode === 'ever' ? 'Anyone ever a member' : 'Members at this hour', tone: 'muted' }];
        const membershipNote = factionMembershipNote(model, id);
        if (membershipNote) unavailable.push({ label: membershipNote, indent: true, tone: 'muted' });
        compact.push(...unavailable); rows.push(...unavailable);
        spoken.push(...unavailable.map(row => `${row.label} ${row.value ?? ''}`.trim()));
      }
      continue;
    }
    const total = { label: name, value: format(run.value), swatch: color(id), strong: true, short: true };
    const members = factionMembersAt(model, id, hour);
    const memberLabel = model.membershipMode === 'end' ? 'Counted: members at the end'
      : model.membershipMode === 'ever' ? 'Counted: anyone ever a member' : 'Members at this hour';
    const count: TooltipRow = { label: memberLabel, value: String(members.length), indent: true, tone: 'muted', short: true };
    const membershipNote = factionMembershipNote(model, id);
    compact.push(total); rows.push(total);
    if (membershipNote) {
      const row: TooltipRow = { label: membershipNote, indent: true, tone: 'muted' };
      compact.push(row); rows.push(row); spoken.push(membershipNote);
    }
    compact.push(count); rows.push(count);
    const shown = members.slice(0, remaining);
    remaining -= shown.length;
    for (const tag of shown) { const country = label(tag); rows.push({ label: country.name, countryFlag: country, indent: true, short: true }); }
    if (shown.length < members.length) rows.push({ label: `${members.length - shown.length} more members`, indent: true, tone: 'muted', short: true });
    const memberNames = members.map(tag => label(tag).name);
    spoken.push(`${total.label} ${total.value}`, `${memberLabel} (${members.length}): ${memberNames.join(', ')}`);
  }
  const empty: TooltipRow = { label: 'Not recorded', tone: 'muted' };
  const accessibility = rows.length ? [...spoken, note].filter(Boolean).join('; ') : undefined;
  return [{ title, subtitle, note, spoken: accessibility, rows: rows.length ? rows : [empty] },
    { title, subtitle, note, spoken: accessibility, rows: compact.length ? compact : [empty] }];
}
