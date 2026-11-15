/** Why a country group shows no one: its membership was recorded, and it held no country under the rule chosen. That is
 * what the recording says, an empty selection, never data that is missing. */
export const NO_MEMBERS = {
  ever: 'This group has no members in this campaign.',
  end: 'This group has no members at the end of this campaign.',
  hour: 'This group has no members at this moment.',
  date: 'This group has no members at this date.',
} as const;

const REASONS: ReadonlySet<string> = new Set(Object.values(NO_MEMBERS));

/** Whether `reason` says a recorded group is empty, which shows as an empty state wherever a widget shows it. */
export const isNoMembers = (reason: string | null | undefined): boolean => reason != null && REASONS.has(reason);
