/** The Game score widget's data: every country's six end-screen scores at an hour (`game-score`): the rows and why a
 * score is not recorded are the reply, each recorded score a live cell (row the tag, column the part) in score points;
 * the multipliers and the hints; a faction by the shared groups. */
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import type { TableGroup } from '../../charts/table/logic';
import type { MeasureParts } from '../../charts/race/measureParts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import type { CountryGroupReply, GameScoreCell, GameScoreReply as Generated, GameScoreRow } from '../../generated/dashboard';

/** The reply holds no hour: a panel is pushed only when it differs, and its push (or the request) gives the hour. */
export type GameScoreReply = Omit<Generated, 'hour'>;
import { resolveCountries } from '../../dashboard/groups.ts';
import type { ResolvedCountries } from '../../dashboard/groups.ts';
import type { CountrySelection } from '../../dashboard/model';

export const SCORE_PARTS = ['total', 'army', 'navy', 'air', 'industry', 'land'] as const;
export type ScorePart = typeof SCORE_PARTS[number];
export const SCORE_LABELS: Readonly<Record<ScorePart | 'country', string>> = {
  country: 'Country', total: 'Total score', army: 'Army', navy: 'Navy', air: 'Air', industry: 'Industry', land: 'Land',
};
export const SCORE_HINTS: Readonly<Record<ScorePart, string>> = {
  total: 'Army, navy and air scores times the total multiplier; industry and land are not in it.',
  army: 'The game’s army count times the army multiplier.', navy: 'The game’s navy count times the navy multiplier.',
  air: 'Planes counted over the air bases the country can use, weighted by each wing’s missions, times the air multiplier.',
  industry: 'Factories counted as the game counts them, times the industry multiplier.', land: 'The game’s land count times the land multiplier.',
};

const cellOk = (cell: unknown) => !!cell && typeof cell === 'object' && ((cell as GameScoreCell).missing === null || typeof (cell as GameScoreCell).missing === 'string');

export function validateGameScore(value: unknown): GameScoreReply {
  const reply = value as Partial<GameScoreReply> | null;
  const ok = !!reply && typeof reply === 'object' && Array.isArray(reply.warnings)
    && (reply.missing === null || typeof reply.missing === 'string') && !!reply.hints && typeof reply.hints === 'object'
    && Array.isArray(reply.rows) && reply.rows.every(row => row && typeof row.tag === 'string' && SCORE_PARTS.every(part => cellOk(row[part])))
    && Array.isArray(reply.multipliers) && reply.multipliers.every(multiplier => multiplier && typeof multiplier.part === 'string'
      && (multiplier.raw === null ? typeof multiplier.missing === 'string' : typeof multiplier.raw === 'string'));
  if (!ok) throw new Error('The game score reply is malformed.');
  return reply as GameScoreReply;
}

export const scorePart = (value: unknown): ScorePart => SCORE_PARTS.find(part => part === value) ?? 'total';
/** A score in points as the end screen shows it: truncated to whole points (or `digits` places). */
export const formatScore = (value: number, digits = 0) => (Math.trunc(value * 10 ** digits) / 10 ** digits).toLocaleString('en-US', { maximumFractionDigits: digits });
/** A row's score as the live cells hold it now: undefined while it is not known. */
export type ScoreOf = (tag: string, part: ScorePart) => number | undefined;

/** The rows shown: every country, the chosen ones, a group's members at the hour, or the top `top` by `by` (a
 * country whose score is not known is not ranked). */
/** The countries a choice names: all countries are the reply's rows, so choosing all names no tag and is no missing choice. */
export function scoreCountries(selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, lastHour: number): ResolvedCountries {
  const resolved = resolveCountries(selection, fallback, groups, lastHour, Number.MAX_SAFE_INTEGER);
  return selection.kind === 'all' ? { ...resolved, reason: null } : resolved;
}

/** `choice.hour`: the hour the rows are shown at, which a group's membership is read at. */
export function scoreRows(reply: GameScoreReply, choice: { all: boolean; tags: readonly string[]; top: number | null; hour: number },
  member: ((tag: string, hour: number) => boolean) | null, by: ScorePart, score: ScoreOf): GameScoreRow[] {
  if (choice.top != null) {
    return reply.rows.filter(row => row[by].missing === null && score(row.tag, by) !== undefined)
      .sort((a, b) => score(b.tag, by)! - score(a.tag, by)! || (a.tag < b.tag ? -1 : a.tag > b.tag ? 1 : 0))
      .slice(0, choice.top);
  }
  if (choice.all) return reply.rows;
  const chosen = new Set(choice.tags);
  return reply.rows.filter(row => (member ? member(row.tag, choice.hour) : chosen.has(row.tag)));
}

/** The faction `tag` belonged to at `hour`, by the groups' recorded memberships. */
export function factionGroup(groups: CountryGroupReply | undefined, tag: string, hour: number): TableGroup {
  if (!groups) return { key: 'faction-not-recorded', label: 'Faction not known yet', order: 2 };
  const index = countryIdentityIndex(groups.countryIdentities);
  const tags = new Set(typeof countryFromKey(tag) === 'number' ? index.aliasesAt(index.selected([tag]), hour) : [tag]);
  for (const faction of groups.factions) {
    if (!faction.members.some(run => tags.has(run.tag) && run.firstHour <= hour && hour <= run.lastHour)) continue;
    const name = faction.names.find(run => run.firstHour <= hour && hour <= run.lastHour)?.name ?? faction.name;
    return { key: `faction:${faction.id}`, label: name ?? 'Faction name unavailable', order: name === null ? 1 : 0, orderKey: BigInt(faction.id) };
  }
  return { key: 'no-faction', label: 'No faction', order: 3 };
}

const missingRow = (label: string, missing: string | null | undefined, strong = false): TooltipRow =>
  ({ label, value: 'Not recorded', detail: missing ?? undefined, tone: 'muted', strong });

/** One country's scores (each missing one with its reason, a recorded one not yet read as such), then the
 * multipliers the game read. */
export function scoreTooltip(reply: GameScoreReply, row: GameScoreRow, name: string, date: string, faction: string, score: ScoreOf): TooltipContent {
  return { title: name, subtitle: date, rows: [
    { label: 'Faction', value: faction },
    ...SCORE_PARTS.map(part => {
      const value = score(row.tag, part), strong = part === 'total';
      return row[part].missing !== null ? missingRow(SCORE_LABELS[part], row[part].missing, strong)
        : value === undefined ? { label: SCORE_LABELS[part], value: 'Not read yet', tone: 'muted' as const, strong } : { label: SCORE_LABELS[part], value: formatScore(value), strong };
    }),
    ...reply.multipliers.map((multiplier, index) => ({ ...(multiplier.raw === null ? missingRow(`${SCORE_LABELS[scorePart(multiplier.part)]} multiplier`, multiplier.missing)
      : { label: `${SCORE_LABELS[scorePart(multiplier.part)]} multiplier`, value: formatRaw(multiplier.raw, 5, 5) }), divider: index === 0 })),
  ] };
}

/** A column's hint: the end screen's own sort control in the recording's language where the content words it, else
 * ours. */
export function scoreHint(reply: Pick<GameScoreReply, 'hints'> | undefined, column: ScorePart | 'country'): string | undefined {
  return reply?.hints[column] ?? (column === 'country' ? undefined : SCORE_HINTS[column]);
}

/** The parts Total score is split into on a race bar: army, navy and air, in proportion to their scores. */
export const scoreParts = (): MeasureParts => ({ kind: 'measures', label: 'Score parts', decimals: 5, proportional: true,
  items: (['army', 'navy', 'air'] as const).map(part => ({ id: part, label: SCORE_LABELS[part], measure: { kind: 'country-game-scores', measure: part } })) });
