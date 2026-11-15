import { object } from '../../dashboard/validation.ts';
import { formatAmount, formatFixed } from '../../charts/valueAxis.ts';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { IndustrialOrganisationsReply } from '../../generated/dashboard';

const TEXT = (value: unknown) => value === null || typeof value === 'string';
const COUNT = (value: unknown) => value === null || Number.isInteger(value);

export function validateOrganisations(value: unknown): IndustrialOrganisationsReply {
  const reply = value as IndustrialOrganisationsReply;
  if (!object(reply) || typeof reply.tag !== 'string' || !TEXT(reply.missing) || !TEXT(reply.detailsMissing) || !Array.isArray(reply.warnings)
    || !Array.isArray(reply.icons) || !reply.icons.every(icon => object(icon) && typeof icon.key === 'string' && typeof icon.image === 'string')
    || !Array.isArray(reply.organisations) || !reply.organisations.every(row => object(row) && /^\d{1,20}$/.test(row.id) && typeof row.token === 'string'
      && [row.name, row.nameUnknown, row.icon, row.iconUnknown, row.policyToken, row.policyName, row.policyUnknown].every(TEXT)
      && [row.size, row.traits, row.cooldownHour].every(COUNT))) {
    throw new Error('Invalid industrial organisations reply');
  }
  return reply;
}

/** One organisation as its row shows it; its funds and unspent points are live cells `(key, funds|points)`. */
export interface OrganisationRow {
  key: string;
  name: string | null;
  token: string;
  icon: string | null;
  size: number | null;
  traits: number | null;
  /** The selected policy as shown; null for none, or where the state was not read. */
  policy: string | null;
  policyToken: string | null;
  /** The hour the policy may change again; null when none is set. */
  cooldownHour: number | null;
  /** The state (size, traits, policy) was read. */
  read: boolean;
}

export function organisationRows(reply: IndustrialOrganisationsReply): OrganisationRow[] {
  return reply.organisations.map(row => ({
    key: row.id, name: row.name, token: row.token, icon: row.icon ? reply.icons.find(icon => icon.key === row.icon)?.image ?? null : null,
    size: row.size, traits: row.traits,
    // A selected policy the recording cannot name is a policy, not "None".
    policy: row.policyName ?? (row.policyToken !== null || row.policyUnknown !== null ? 'Policy name unavailable' : null),
    policyToken: row.policyToken, cooldownHour: row.cooldownHour, read: row.size !== null,
  }));
}

/** Funds (a fixed5 raw) as the game's organisation screen shows them: whole numbers. */
export const fundsText = (raw: bigint | null) => raw === null ? '—' : formatAmount(raw, 5, 0);
export const exactFundsText = (raw: bigint) => formatFixed(raw, 5, 5);
/** A funds cell's value as its fixed5 raw. */
export const fundsRaw = (value: number | undefined) => value === undefined ? null : BigInt(Math.round(value));

/** Whole days until a policy may change again, as the game counts them: its end day less the shown hour's; null when
 * none are left. */
export function cooldownDays(cooldownHour: number | null, hour: number): number | null {
  if (cooldownHour === null) return null;
  const days = Math.floor(cooldownHour / 24) - Math.floor(hour / 24);
  return days > 0 ? days : null;
}

/** The selected policy; "None" only where the state was read (an unread state is unknown). */
export function policyText(row: OrganisationRow, days: number | null): string {
  const policy = row.policy ?? (row.read ? 'None' : '—');
  return days !== null ? `${policy} · Cooldown ${days} days` : policy;
}

/** Each row's cooldown days at `hour`, keyed by row: what the policy column paints. */
export function cooldownValues(rows: readonly OrganisationRow[], hour: number): LiveValues<number> {
  const days = new Map(rows.map(row => [row.key, cooldownDays(row.cooldownHour, hour) ?? 0]));
  return { get: key => days.get(key), missing: () => 'Not shown' };
}

/** A row's funds and points from the live cells, null where a cell has no value. */
export function organisationNumbers(values: LiveValues<number>, key: string): { funds: bigint | null; points: number | null } {
  return { funds: fundsRaw(values.get(liveKey(key, 'funds'))), points: values.get(liveKey(key, 'points')) ?? null };
}

export const organisationRevision = (row: OrganisationRow) => [row.name, row.token, row.icon, row.size, row.traits, row.policy, row.policyToken, row.cooldownHour];
