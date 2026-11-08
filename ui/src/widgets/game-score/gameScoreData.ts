import { runAt, isOrderedSpans, runIndexAt, spansCovered } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import type { CountryRaceEntry } from '../../charts/race/countryRace';
import type { CountryFlagLook } from '../../components/CountryFlag';
import type { FlagRead } from '../../dashboard/countryFlags';
import { flagReadMetadata } from '../../dashboard/countryFlagsAt.ts';
import { countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import type { FAILED_FACT } from '../../dashboard/countryFacts';
import { formatNativeFixed5 } from '../../charts/valueAxis.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { sortRows } from '../../charts/table/logic.ts';
import type { Sort, TableGroup } from '../../charts/table/logic';
import { fixed5At, validateFixed5Runs } from '../../dashboard/fixed5Runs.ts';
import type { NativeFixed5Run, GameScoreReply, GameScoreIdentity, GameScoreMissing } from '../../generated/dashboard';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { readAt } from '../../dashboard/reads.ts';
import { int, object, unsigned32 } from '../../dashboard/validation.ts';

export const SCORE_PARTS = ['total', 'army', 'navy', 'air', 'industry', 'land'] as const;
export type ScorePart = typeof SCORE_PARTS[number];
export const BAR_PARTS = ['army', 'navy', 'air'] as const;
export const SCORE_LABELS = { country: 'Country', total: 'Total score', army: 'Army', navy: 'Navy', air: 'Air',
  industry: 'Industry', land: 'Land', faction: 'Faction', multiplier: 'Score multiplier' } as const;
const missing = (value: unknown): value is GameScoreMissing => object(value)
  && (value.kind === 'recording' || value.kind === 'content') && typeof value.reason === 'string';
const nullableMissing = (value: unknown) => value === null || missing(value);
const nullableString = (value: unknown) => value === null || typeof value === 'string';
const nullableInt = (value: unknown) => value === null || unsigned32(value);
const factionId = (value: unknown) => typeof value === 'string' && /^(0|[1-9]\d*)$/.test(value) && BigInt(value) < (1n << 64n);
function identity(value: unknown): value is GameScoreIdentity {
  if (!object(value) || !nullableInt(value.canonicalSlot) || !nullableInt(value.nativeCountry)
    || !nullableString(value.name) || !nullableMissing(value.nameMissing) || !nullableMissing(value.colorMissing)
    || !nullableMissing(value.factionMissing)
    || !(value.color === null || (Array.isArray(value.color) && value.color.length === 3 && value.color.every(channel => int(channel) && channel >= 0 && channel <= 255)))) return false;
  return value.faction === null || (object(value.faction) && factionId(value.faction.id)
    && nullableString(value.faction.name) && nullableMissing(value.faction.nameMissing) && int(value.faction.ordinal) && value.faction.ordinal >= 0);
}
export function validateGameScoreReply(value: unknown): GameScoreReply {
  if (!object(value) || typeof value.branchId !== 'string' || value.scale !== 100000 || !Array.isArray(value.reads)
    || !value.reads.every((hour, index) => int(hour) && (index === 0 || (value.reads as number[])[index - 1] < hour))
    || !Array.isArray(value.reasons) || !value.reasons.every(missing) || !Array.isArray(value.countries)
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string')
    || !Array.isArray(value.scoreWarnings) || !value.scoreWarnings.every(warning => object(warning) && int(warning.observations) && warning.observations > 0 && missing(warning))
    || !object(value.hints) || !Object.values(value.hints).every(hint => typeof hint === 'string')) throw new Error('The game score reply is malformed.');
  const reasonCount = value.reasons.length;
  const observed: Span[] = [];
  for (const hour of value.reads as number[]) {
    const previous = observed.at(-1);
    if (previous && previous.lastHour + 1 === hour) previous.lastHour = hour;
    else observed.push({ firstHour: hour, lastHour: hour });
  }
  const series = (item: unknown) => object(item) && SCORE_PARTS.every(part => {
    const runs = item[part];
    return validateFixed5Runs(runs, reasonCount) && spansCovered(runs, observed);
  });
  if (!series(value.multipliers)) throw new Error('The game score multipliers are malformed.');
  const tags = new Set<string>();
  for (const country of value.countries) {
    if (!object(country) || typeof country.tag !== 'string' || tags.has(country.tag) || !isOrderedSpans(country.identity)
      || !country.identity.every(run => object(run) && identity(run.value)) || !spansCovered(country.identity, observed)
      || !series(country.scores)) throw new Error('A game score country is malformed.');
    tags.add(country.tag);
  }
  return value as unknown as GameScoreReply;
}
export interface ScoreCell { raw: string | null; readHour: number | null; reason: string | null; contentEvaluated: boolean }
export interface ScoreRow {
  key: string; tag: string; hour: number; stretch: string; identity: GameScoreIdentity; cells: Record<ScorePart, ScoreCell>;
  multipliers: Record<ScorePart, ScoreCell>;
}
export function gameScoreModel(reply: GameScoreReply) {
  const entityRuns = new Map<string, Span[]>();
  for (const country of reply.countries) {
    let scoreIndex = 0;
    for (const run of country.identity) {
      const { canonicalSlot, nativeCountry } = run.value;
      while (scoreIndex < country.scores.total.length && country.scores.total[scoreIndex].lastHour < run.firstHour) scoreIndex++;
      if (canonicalSlot === null || nativeCountry === null) continue;
      const key = countryIdentityKey({ slot: canonicalSlot, countryId: nativeCountry });
      for (let index = scoreIndex; index < country.scores.total.length && country.scores.total[index].firstHour <= run.lastHour; index++) {
        const score = country.scores.total[index];
        if (score.raw == null) continue;
        const span = { firstHour: Math.max(run.firstHour, score.firstHour), lastHour: Math.min(run.lastHour, score.lastHour) };
        const previous = entityRuns.get(key);
        if (previous) previous.push(span); else entityRuns.set(key, [span]);
      }
    }
  }
  for (const [key, runs] of entityRuns) {
    const merged: Span[] = [];
    for (const run of runs.sort((a, b) => a.firstHour - b.firstHour)) {
      const last = merged.at(-1);
      if (last && run.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, run.lastHour);
      else merged.push({ firstHour: run.firstHour, lastHour: run.lastHour });
    }
    entityRuns.set(key, merged);
  }
  return { reply, countries: new Map(reply.countries.map(country => {
    const starts: (number | null)[] = [];
    country.scores.total.forEach((run, index) => starts.push(run.raw == null ? null
      : index > 0 && starts[index - 1] !== null && country.scores.total[index - 1].lastHour + 1 === run.firstHour
        ? starts[index - 1] : run.firstHour));
    const identityStarts: number[] = [];
    country.identity.forEach((run, index) => {
      const previous = country.identity[index - 1];
      identityStarts.push(previous && gameScoreCountryKnown(run.value) && previous.lastHour + 1 === run.firstHour
        && previous.value.canonicalSlot === run.value.canonicalSlot && previous.value.nativeCountry === run.value.nativeCountry
        ? identityStarts[index - 1] : run.firstHour);
    });
    return [country.tag, { ...country, starts, identityStarts }];
  })), entityRuns, tags: reply.countries.map(country => country.tag),
    identities: reply.countries.map(country => ({ tag: country.tag, identities: country.identity.flatMap(run => {
      const { canonicalSlot, nativeCountry } = run.value;
      return canonicalSlot === null || nativeCountry === null ? [] : [{ firstHour: run.firstHour, lastHour: run.lastHour, slot: canonicalSlot, countryId: nativeCountry }];
    }) })) };
}
export type GameScoreModel = ReturnType<typeof gameScoreModel>;
/** Looks never decode score fields; this is also the identity source for a row. */
export function gameScoreIdentityAt(model: GameScoreModel | null, tag: string, hour: number): GameScoreIdentity | null {
  return runAt(model?.countries.get(tag)?.identity ?? [], hour)?.value ?? null;
}
export function gameScoreCountryKnown(value: GameScoreIdentity | null | undefined): boolean {
  return value?.canonicalSlot != null && value.nativeCountry != null;
}
type ScoreFlagAt = (tag: string, hour: number) => FlagRead | null | undefined | typeof FAILED_FACT | Error;
export function gameScoreCountryCell(row: ScoreRow, flagAt: ScoreFlagAt) {
  if (!gameScoreCountryKnown(row.identity)) return null;
  return { name: row.identity.name ?? 'Country name unavailable', ...flagReadMetadata(flagAt(row.tag, row.hour)) };
}
/** Product-owned race looks use one exact read; names can resolve another dated tooltip. */
export function gameScoreLooks(model: GameScoreModel | null, moment: number, flagAt: ScoreFlagAt) {
  const hour = Math.floor(moment);
  const identities = new Map(model?.tags.map(tag => [tag, gameScoreIdentityAt(model, tag, hour)]) ?? []);
  const countryFlag = (tag: string, at = hour): CountryFlagLook | undefined => {
    if (Math.floor(at) !== hour || !gameScoreCountryKnown(identities.get(tag))) return undefined;
    return flagReadMetadata(flagAt(tag, hour));
  };
  return {
    name: (tag: string, atHour = hour) => {
      const at = Math.floor(atHour);
      return (at === hour ? identities.get(tag) : gameScoreIdentityAt(model, tag, at))?.name ?? 'Country name unavailable';
    },
    color: (tag: string) => identities.get(tag)?.color ?? null,
    flag: (tag: string) => countryFlag(tag)?.flag,
    countryFlag,
    matches: (tag: string, at: number) => Math.floor(at) === hour && identities.get(tag) != null,
  };
}

function cell(reply: GameScoreReply, runs: readonly NativeFixed5Run[], hour: number): ScoreCell {
  const at = fixed5At(runs, hour);
  const readHour = at?.readEachHour ? readAt(reply.reads, hour) : null;
  return { raw: at?.raw ?? null, readHour,
    reason: at?.missing != null ? reply.reasons[at.missing].reason : at ? null : 'Not recorded at this hour; the recording has a gap.',
    contentEvaluated: at?.raw != null && !at.readEachHour };
}
export function gameScoreRow(model: GameScoreModel, tag: string, hour: number): ScoreRow | null {
  const country = model.countries.get(tag);
  const identityIndex = country ? runIndexAt(country.identity, hour) : -1;
  if (!country || identityIndex < 0) return null;
  const at = country.identity[identityIndex];
  const read = readAt(model.reply.reads, hour);
  if (read === null || read < at.firstHour) return null;
  const index = runIndexAt(country.scores.total, hour);
  const stretch = index < 0 ? null : country.starts[index];
  const { canonicalSlot, nativeCountry } = at.value;
  const key = canonicalSlot === null || nativeCountry === null ? JSON.stringify([tag, canonicalSlot, nativeCountry])
    : countryIdentityKey({ slot: canonicalSlot, countryId: nativeCountry });
  return { key, tag, hour,
    stretch: gameScoreCountryKnown(at.value) ? JSON.stringify([key, runAt(model.entityRuns.get(key) ?? [], hour)?.firstHour ?? null])
      : JSON.stringify([country.identityStarts[identityIndex], stretch]), identity: at.value,
    cells: Object.fromEntries(SCORE_PARTS.map(part => [part, cell(model.reply, country.scores[part], hour)])) as Record<ScorePart, ScoreCell>,
    multipliers: Object.fromEntries(SCORE_PARTS.map(part => [part, cell(model.reply, model.reply.multipliers[part], hour)])) as Record<ScorePart, ScoreCell> };
}
export function scoreValue(row: ScoreRow, column: string): bigint | string | null {
  if (column === 'country') return row.identity.name;
  const raw = SCORE_PARTS.includes(column as ScorePart) ? row.cells[column as ScorePart].raw : null;
  return raw == null ? null : BigInt(raw);
}
export function gameScoreFaction(row: ScoreRow): TableGroup {
  const faction = row.identity.faction;
  if (faction) return { key: `faction:${faction.id}`, label: faction.name ?? 'Faction name unavailable', order: faction.name === null ? 1 : 0, orderKey: BigInt(faction.id) };
  return row.identity.factionMissing
    ? { key: 'faction-not-recorded', label: 'Faction not recorded', order: 2 }
    : { key: 'no-faction', label: 'No faction', order: 3 };
}
export function gameScoreRows(model: GameScoreModel, tags: readonly string[], hour: number, ranking: Sort, top?: number): ScoreRow[] {
  const rows = tags.flatMap(tag => { const row = gameScoreRow(model, tag, hour); return row ? [row] : []; });
  const ordered = sortRows(rows, row => scoreValue(row, ranking.column), ranking.direction, row => row.tag);
  return top === undefined ? ordered : ordered.slice(0, top);
}
export function formatScore(raw: string | null, digits = 0) { return raw === null ? '—' : formatNativeFixed5(raw, digits); }
/** Wrapping and signed scores do not have an honest nonnegative share geometry. */
export function scoreInsetReason(row: ScoreRow): string | null {
  const parts = BAR_PARTS.map(part => row.cells[part].raw);
  const total = row.cells.total.raw;
  const multiplier = row.multipliers.total.raw;
  if (parts.some(part => part === null) || total === null || multiplier === null) return 'Score parts or the Total multiplier are unavailable.';
  const rawParts = parts.map(part => BigInt(part!));
  const sum = rawParts.reduce((a, b) => a + b, 0n);
  const product = sum * BigInt(multiplier);
  if (rawParts.some(part => part < 0n) || BigInt(total) < 0n || BigInt(multiplier) < 0n
    || sum !== BigInt.asIntN(64, sum) || product !== BigInt.asIntN(64, product)) return 'Signed or wrapped scores are shown as an unsplit Total bar.';
  if (sum === 0n && BigInt(total) !== 0n) return 'The score parts cannot form shares of this Total.';
  return null;
}
export function gameScoreRace(rows: readonly ScoreRow[], split: boolean): CountryRaceEntry[] {
  return rows.filter(row => row.cells.total.raw !== null).map(row => {
    const total = Number(BigInt(row.cells.total.raw!)) / 100000;
    const parts = split && scoreInsetReason(row) === null
      ? BAR_PARTS.map(id => ({ id, raw: BigInt(row.cells[id].raw!) })) : null;
    const sum = parts?.reduce((sum, part) => sum + part.raw, 0n) ?? 0n;
    return { tag: row.tag, entityKey: row.key, hour: row.hour, value: total, label: formatScore(row.cells.total.raw), stretch: row.stretch,
      // Total score applies its multiplier and truncation after summing.
      // Only plot widths scale to that recorded result; raw part cells stay exact.
      ...(parts ? { parts: parts.map(part => ({ id: part.id,
        value: sum === 0n ? 0 : total * (Number(part.raw) / Number(sum)) })) } : {}),
    };
  });
}
function cellRows(label: string, value: ScoreCell, digits = 0, strong = false): TooltipRow[] {
  return [{ label, value: value.raw === null ? 'Unavailable' : formatScore(value.raw, digits), strong }];
}
export function gameScoreTooltip(row: ScoreRow): TooltipContent {
  return { title: row.identity.name ?? 'Country name unavailable', subtitle: formatDateTime(row.hour),
    rows: [{ label: SCORE_LABELS.faction, value: gameScoreFaction(row).label },
      ...SCORE_PARTS.flatMap(part => cellRows(SCORE_LABELS[part], row.cells[part], 0, part === 'total')),
      { label: SCORE_LABELS.multiplier, value: row.multipliers.total.raw === null ? 'Unavailable' : formatScore(row.multipliers.total.raw, 5), divider: true },
      ...SCORE_PARTS.filter(part => part !== 'total').flatMap(part => cellRows(`${SCORE_LABELS[part]} multiplier`, row.multipliers[part], 5))] };
}

/** Every score and multiplier appears once, without acquisition metadata. */
export function* gameScoreTooltips(row: ScoreRow): Generator<TooltipContent> {
  yield gameScoreTooltip(row);
}
