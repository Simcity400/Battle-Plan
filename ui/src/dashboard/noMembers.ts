export const NO_MEMBERS = {
  ever: 'This group has no members in this campaign.',
  hour: 'This group has no members at this moment.',
  date: 'This group has no members at this date.',
  yet: 'This group has no members yet.',
  span: 'This group has no members during this time.',
} as const;

const REASONS: ReadonlySet<string> = new Set(Object.values(NO_MEMBERS));

export const isNoMembers = (reason: string | null | undefined): boolean => reason != null && REASONS.has(reason);
