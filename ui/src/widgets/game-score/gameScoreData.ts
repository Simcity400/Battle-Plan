import type { GameScoreCell, GameScoreReply as Generated, GameScoreTop } from '../../generated/dashboard';

export type GameScoreReply = Omit<Generated, 'hour'>;

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
const topOk = (top: unknown) => top === undefined || (!!top && typeof top === 'object' && Array.isArray((top as GameScoreTop).tags)
  && (top as GameScoreTop).tags.every(tag => typeof tag === 'string')
  && ((top as GameScoreTop).missing === null || typeof (top as GameScoreTop).missing === 'string'));

export function validateGameScore(value: unknown): GameScoreReply {
  const reply = value as Partial<GameScoreReply> | null;
  const ok = !!reply && typeof reply === 'object' && Array.isArray(reply.warnings)
    && (reply.missing === null || typeof reply.missing === 'string') && !!reply.hints && typeof reply.hints === 'object'
    && topOk(reply.top)
    && Array.isArray(reply.rows) && reply.rows.every(row => row && typeof row.tag === 'string' && SCORE_PARTS.every(part => cellOk(row[part])))
    && Array.isArray(reply.multipliers) && reply.multipliers.every(multiplier => multiplier && typeof multiplier.part === 'string'
      && (multiplier.raw === null ? typeof multiplier.missing === 'string' : typeof multiplier.raw === 'string'));
  if (!ok) throw new Error('The game score reply is malformed.');
  return reply as GameScoreReply;
}

export const scorePart = (value: unknown): ScorePart => SCORE_PARTS.find(part => part === value) ?? 'total';

export function scoreHint(reply: Pick<GameScoreReply, 'hints'> | undefined, column: ScorePart | 'country'): string | undefined {
  return reply?.hints[column] ?? (column === 'country' ? undefined : SCORE_HINTS[column]);
}
