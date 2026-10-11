import { runAt, runIndexAt } from '../../charts/series.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { countryIdentityKey, validCountryIdentityRuns } from '../../dashboard/countryIdentity.ts';
import { formatDateTime, isEngineHour } from '../../dashboard/calendar.ts';
import { i32Value, nullableValue, orderedHourSpans as ordered, pngDataValue, raw64Value as raw, recordValue as object, textValue as text, tokenValue as token } from '../../dashboard/payload.ts';
import type { OperativeRosterReply } from '../../generated/dashboard';

type Country = OperativeRosterReply['countries'][number];
type Entry = Country['operatives'][number];
type OperativeRun = Entry['runs'][number];
type Value = OperativeRun['value'];
export interface OperativeRow { key: string; country: string; entry: Entry; run: OperativeRun; readHour: number; assignmentReadHour: number | null; departed: boolean }
export interface RosterView { rows: OperativeRow[]; unknown: string[] }

const hour = (v: unknown): v is number => typeof v === 'number' && isEngineHour(v);
const nullableText = nullableValue(text);
const nullableTag = nullableValue(token);
const nullableHour = nullableValue(hour);
const nullableI32 = nullableValue(i32Value);
const nullableBoolean = (v: unknown) => v === null || typeof v === 'boolean';
const observations = (v: unknown, aliases = false) => ordered(v) && v.every((read, index, reads) => {
  const previous = reads[index - 1];
  return hour(read.readHour) && read.readHour <= read.firstHour
    && (aliases || ((!previous || Number(previous.readHour) <= read.readHour)
      && (previous?.lastHour === read.firstHour - 1 || read.readHour === read.firstHour)));
});

export function validateOperativesRosterReply(data: unknown): OperativeRosterReply {
  if (!object(data) || !text(data.branchId) || !/^\d+$/.test(data.branchId)
    || !Array.isArray(data.artwork) || !data.artwork.every(pngDataValue)
    || !Array.isArray(data.reasons) || !data.reasons.every(text)) throw new Error('The operatives roster reply is malformed.');
  const artwork = data.artwork;
  const reasons = data.reasons;
  const icon = nullableValue(v => typeof v === 'number' && Number.isInteger(v) && v >= 0 && v < artwork.length);
  const reason = nullableValue(v => typeof v === 'number' && Number.isInteger(v) && v >= 0 && v < reasons.length);
  const value = (v: unknown) => object(v) && nullableText(v.name) && typeof v.unnamed === 'boolean' && (!v.unnamed || v.name === null)
    && reason(v.nameMissing) && icon(v.portrait) && reason(v.portraitMissing)
    && nullableI32(v.skill) && raw(v.experienceRaw)
    && nullableText(v.experienceText) && reason(v.experienceMissing) && (v.experienceRatio === null || (typeof v.experienceRatio === 'number' && Number.isFinite(v.experienceRatio)))
    && nullableText(v.assignmentName) && nullableText(v.assignmentDuration)
    && (v.assignmentProgress === null || (typeof v.assignmentProgress === 'number' && Number.isFinite(v.assignmentProgress)))
    && reason(v.assignmentMissing) && icon(v.assignmentIcon) && reason(v.assignmentIconMissing) && typeof v.assignmentOperation === 'boolean'
    && nullableTag(v.targetTag) && nullableBoolean(v.captured) && nullableTag(v.capturerTag) && nullableHour(v.capturedHour) && nullableTag(v.ownerTag);
  if (!observations(data.operations) || !ordered(data.agencyAbsent) || !Array.isArray(data.countries)
    || !data.countries.every(country => object(country) && token(country.tag)
      && Array.isArray(country.identities) && validCountryIdentityRuns(country.identities) && observations(country.observed, true)
      && Array.isArray(country.operatives) && country.operatives.every(entry => object(entry) && token(entry.key) && token(entry.id)
        && hour(entry.joinedHour) && nullableHour(entry.departedHour) && nullableText(entry.departedReason) && ordered(entry.runs) && entry.runs.length > 0
        && entry.runs.every(run => value(run.value))))
    || !Array.isArray(data.warnings) || !data.warnings.every(text)) {
    throw new Error('The operatives roster reply is malformed.');
  }
  const reply = data as unknown as OperativeRosterReply;
  if (new Set(reply.countries.map(country => country.tag)).size !== reply.countries.length) throw new Error('The operatives roster reply repeats a country.');
  const reads = new Map<string, { readHour: number; spans: { firstHour: number; lastHour: number }[] }>();
  for (const country of reply.countries) {
    for (const observation of country.observed) {
      let index = runIndexAt(country.identities, observation.firstHour);
      let end = observation.firstHour - 1;
      while (index >= 0 && index < country.identities.length && end < observation.lastHour) {
        const identity = country.identities[index++];
        if (identity.firstHour > end + 1) break;
        const key = JSON.stringify([countryIdentityKey(identity), observation.readHour]);
        let read = reads.get(key);
        if (!read) { read = { readHour: observation.readHour, spans: [] }; reads.set(key, read); }
        const lastHour = Math.min(identity.lastHour, observation.lastHour);
        read.spans.push({ firstHour: end + 1, lastHour });
        end = lastHour;
      }
      if (end < observation.lastHour) throw new Error('The operatives roster reply crosses unobserved native identities.');
    }
  }
  // A held read can move across aliases only with its actual published source
  // census and uninterrupted roster coverage for that exact native holder.
  for (const read of reads.values()) {
    read.spans.sort((a, b) => a.firstHour - b.firstHour);
    let end = read.readHour - 1;
    for (const span of read.spans) {
      if (span.firstHour !== end + 1) throw new Error('The operatives roster reply has an unproved held membership read.');
      end = span.lastHour;
    }
  }
  for (const country of reply.countries) {
    if (new Set(country.operatives.map(entry => entry.key)).size !== country.operatives.length) throw new Error('The operatives roster reply repeats an operative.');
    for (const entry of country.operatives) {
      if (entry.joinedHour > entry.runs[0].firstHour || (entry.departedHour !== null && entry.departedHour < entry.runs.at(-1)!.lastHour)) {
        throw new Error('The operatives roster reply has an invalid membership date.');
      }
      // Begin at this episode's first observation, not the branch's first read:
      // many late recruits must not each scan an entire earlier campaign.
      let index = runIndexAt(country.observed, entry.runs[0].firstHour);
      if (index < 0) throw new Error('The operatives roster reply crosses unobserved hours.');
      if (entry.joinedHour > country.observed[index].readHour) throw new Error('The operatives roster reply has an invalid membership date.');
      for (const run of entry.runs) {
        while (index < country.observed.length && country.observed[index].lastHour < run.firstHour) index++;
        let end = run.firstHour - 1;
        for (let at = index; at < country.observed.length && end < run.lastHour; at++) {
          const read = country.observed[at];
          if (read.firstHour > end + 1) break;
          end = read.lastHour;
        }
        if (end < run.lastHour) throw new Error('The operatives roster reply crosses unobserved hours.');
      }
    }
  }
  return reply;
}

/** Live is a roster at the shown moment. Campaign adds departed entries only
 * once a recorded read has shown their departure; a missing run stays missing. */
export function rosterAt(reply: OperativeRosterReply, hour: number, includeDeparted: boolean,
  matches: (tag: string, hour: number) => boolean = () => true): RosterView {
  const view: RosterView = { rows: [], unknown: [] };
  for (const country of reply.countries) {
    const observation = runAt(country.observed, hour);
    if (!observation && matches(country.tag, hour)) view.unknown.push(country.tag);
    for (const entry of country.operatives) {
      const current = runAt(entry.runs, hour);
      const departed = entry.departedHour !== null && entry.departedHour <= hour;
      if (departed && !includeDeparted) continue;
      if (!departed && !observation) continue;
      const run = current ?? (departed ? entry.runs.at(-1) : null);
      if (!run) continue;
      if (!matches(country.tag, departed ? run.lastHour : hour)) continue;
      const read = departed ? runAt(country.observed, run.lastHour) : observation;
      if (!read) continue;
      const operation = run.value.assignmentOperation && (run.value.assignmentName !== null || run.value.assignmentIcon !== null)
        ? runAt(reply.operations, departed ? run.lastHour : hour) : null;
      view.rows.push({ key: JSON.stringify([country.tag, entry.key]), country: country.tag, entry, run,
        readHour: read.readHour, assignmentReadHour: operation?.readHour ?? null, departed: !!departed });
    }
  }
  return view;
}

export function operativeName(value: Value): string {
  return value.unnamed ? 'Unnamed operative' : value.name ?? 'Name unknown';
}
export function operativeLabelHour(row: OperativeRow, shownHour: number): number {
  return row.departed ? row.run.lastHour : shownHour;
}
export function departureText(row: OperativeRow): string | null {
  return row.departed && row.entry.departedHour !== null
    ? `${row.entry.departedReason ?? 'Left (reason not recorded)'} by ${formatDateTime(row.entry.departedHour)}` : null;
}
export interface OperativeMoment { kind: 'joined' | 'captured' | 'departed'; hour: number; label: string }
export function operativeMoments(row: OperativeRow): OperativeMoment[] {
  const moments: OperativeMoment[] = [{ kind: 'joined', hour: row.entry.joinedHour, label: `Jump to presence by ${formatDateTime(row.entry.joinedHour)}` }];
  const captured = new Set<number>();
  for (const run of row.entry.runs) {
    if (run.firstHour > row.run.firstHour) break;
    if (run.value.captured === true && run.value.capturedHour !== null) captured.add(run.value.capturedHour);
  }
  for (const hour of [...captured].sort((a, b) => a - b)) moments.push({ kind: 'captured', hour, label: `Jump to capture ${formatDateTime(hour)}` });
  if (row.departed && row.entry.departedHour !== null) moments.push({ kind: 'departed', hour: row.entry.departedHour, label: `Jump to departure by ${formatDateTime(row.entry.departedHour)}` });
  return moments;
}

export function operativeTooltip(reply: OperativeRosterReply, row: OperativeRow, shownHour: number, country: (tag: string, hour: number) => ReturnType<CountryLabel>): TooltipContent {
  const value = row.run.value;
  const labelHour = operativeLabelHour(row, shownHour);
  const countryRow = (label: string, tag: string | null): TooltipRow => {
    if (tag === null) return { label, value: 'Unknown' };
    const value = country(tag, labelHour);
    return { label, value: value.name, countryFlag: {
      flag: value.flag, flagReason: value.flagReason, flagPending: value.flagPending,
    } };
  };
  const rows: TooltipRow[] = [
    countryRow('Roster country', row.country), countryRow('Owner', value.ownerTag),
    { label: 'Skill', value: value.skill === null ? 'Unknown' : String(value.skill) },
    { label: 'Experience', value: value.experienceText ?? 'Unknown' },
    { label: 'Assignment', value: value.assignmentName ?? 'Unknown' },
    ...(value.assignmentDuration === null ? [] : [{ label: 'Assignment duration', value: value.assignmentDuration }]),
    ...(row.assignmentReadHour === null || row.assignmentReadHour === row.readHour ? [] : [{ label: 'Assignment read', value: formatDateTime(row.assignmentReadHour) }]),
    ...(value.targetTag === null ? [] : [countryRow('Target', value.targetTag)]),
    { label: 'Captured', value: value.captured === null ? 'Unknown' : value.captured ? 'Yes' : 'No' },
    ...(value.captured === true ? [
      countryRow('Capturer', value.capturerTag),
      { label: 'Captured on', value: value.capturedHour === null ? 'Unknown' : formatDateTime(value.capturedHour) },
    ] : []),
    { label: 'Present', value: `by ${formatDateTime(row.entry.joinedHour)}` },
    ...(row.departed && row.entry.departedHour !== null ? [{ label: row.entry.departedReason ?? 'Left (reason not recorded)', value: `by ${formatDateTime(row.entry.departedHour)}` }] : []),
  ];
  const notes = [...[value.nameMissing, value.portraitMissing, value.experienceMissing, value.assignmentMissing, value.assignmentIconMissing]
      .map(index => index === null ? null : reply.reasons[index])];
  if (row.departed) notes.push(`Last operative details read ${formatDateTime(row.readHour)}; country identities last observed ${formatDateTime(labelHour)}. Departure cause is only shown when recorded.`);
  return { title: operativeName(value), icon: value.portrait === null ? undefined : reply.artwork[value.portrait], rows, note: notes.filter(Boolean).join(' ') || undefined };
}
