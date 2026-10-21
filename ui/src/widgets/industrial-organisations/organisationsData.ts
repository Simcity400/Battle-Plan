import type { OrganisationHistory, ProductionCatalogueReply } from '../../generated/dashboard';
import { object } from '../../dashboard/validation.ts';
import { bigAt, fixedText, isEconomyRuns, isHolderMap, numberAt, runAt, tagAt, validateEconomyReply, valueAt } from '../../dashboard/economyHistory.ts';

export function validateOrganisations(value: unknown): ProductionCatalogueReply {
  return validateEconomyReply<ProductionCatalogueReply>(value, reply => reply.view === 'organisations' && isEconomyRuns(reply.known) && isHolderMap(reply.holders)
    && Array.isArray(reply.texts) && reply.texts.every(text => typeof text === 'string')
    && object(reply.icons) && Object.values(reply.icons).every(icon => icon === null || typeof icon === 'string')
    && Array.isArray(reply.organisations) && reply.organisations.every(row => object(row) && typeof row.id === 'string' && /^\d{1,20}$/.test(row.id)
      && ['holder', 'size', 'funds', 'points', 'unlocked', 'policy', 'policyName', 'policyUnresolved', 'cooldown'].every(key => isEconomyRuns(row[key]))
      && Array.isArray(row.names) && row.names.every(name => object(name) && Number.isSafeInteger(name.firstHour) && typeof name.token === 'string'
        && (name.name === null || typeof name.name === 'string') && (name.icon === null || typeof name.icon === 'string'))),
  'industrial organisations');
}

/** One organisation at the hour shown. */
export interface OrganisationRow {
  key: string;
  organisation: OrganisationHistory;
  holder: string;
  name: string | null;
  token: string;
  icon: string | null;
  size: number | null;
  funds: bigint | null;
  points: number | null;
  traits: number | null;
  policy: string | null;
  /** The policy's script key, for hover only. */
  policyToken: string | null;
  /** Whole days until the policy can change again; null when none is pending. */
  cooldownDays: number | null;
}

/** The country's organisations at `hour`: every one it holds then, ranked by size,
 * then funds, then native reference (app order, not the game's). */
export function organisationsAt(reply: ProductionCatalogueReply, tag: string, hour: number): OrganisationRow[] {
  const rows: OrganisationRow[] = [];
  for (const organisation of reply.organisations) {
    const holder = valueAt(organisation.holder, hour);
    if (holder === null || tagAt(reply.holders[holder], hour) !== tag) continue;
    let label = organisation.names[0] ?? null;
    for (const name of organisation.names) if (name.firstHour <= hour) label = name;
    const policy = numberAt(organisation.policy, hour), policyName = numberAt(organisation.policyName, hour);
    // A selected policy whose token the recording cannot spell is a policy, not "None".
    const unresolved = runAt(organisation.policyUnresolved, hour) !== null;
    const cooldown = numberAt(organisation.cooldown, hour);
    rows.push({ key: organisation.id, organisation, holder, name: label?.name ?? null, token: label?.token ?? '',
      icon: label?.icon ? reply.icons[label.icon] ?? null : null,
      size: numberAt(organisation.size, hour), funds: bigAt(organisation.funds, hour), points: numberAt(organisation.points, hour),
      traits: numberAt(organisation.unlocked, hour),
      policy: policyName !== null ? reply.texts[policyName] ?? null : policy !== null || unresolved ? 'Policy name unavailable' : null,
      policyToken: policy !== null ? reply.texts[policy] ?? null : null,
      cooldownDays: cooldown !== null && cooldown > hour ? Math.ceil((cooldown - hour) / 24) : null });
  }
  return rows.sort((a, b) => (b.size ?? -Infinity) - (a.size ?? -Infinity)
    || (a.funds === null ? 1 : b.funds === null ? -1 : a.funds === b.funds ? 0 : a.funds > b.funds ? -1 : 1) || (BigInt(a.key) < BigInt(b.key) ? -1 : BigInt(a.key) > BigInt(b.key) ? 1 : 0));
}

export const fundsText = (raw: bigint | null) => raw === null ? '—' : fixedText(raw, 100000n);
/** The selected policy; "None" only where the state was read (an unread state is unknown). */
export function policyText(row: OrganisationRow): string {
  const policy = row.policy ?? (row.size === null ? '—' : 'None');
  return row.cooldownDays !== null ? `${policy} · Cooldown ${row.cooldownDays} days` : policy;
}

/** Whether the organisation roster was read complete at `hour`. */
export const rosterKnown = (reply: Pick<ProductionCatalogueReply, 'known'>, hour: number) => runAt(reply.known, hour) !== null;

