import { object } from '../../dashboard/validation.ts';
import { groupMembers, groupRuns, groupUnobserved } from '../../dashboard/groups.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { CountryGroupReply, LicenceDesign, LicenceNow, LicenceParty, ProductionLicencesReply } from '../../generated/dashboard';

export const PRODUCTION_LICENCES = 'production-licences';
export type LicenceWidth = 'licences' | 'factories';

const isParty = (party: LicenceParty) => object(party) && (party.slot === null || Number.isInteger(party.slot)) && (party.tag === null || typeof party.tag === 'string');

export function validateLicences(value: unknown): ProductionLicencesReply {
  const reply = value as ProductionLicencesReply;
  if (!object(reply) || typeof reply.branchId !== 'string' || (reply.missing !== null && typeof reply.missing !== 'string') || !Array.isArray(reply.warnings)
    || !Array.isArray(reply.licences) || !reply.licences.every(row => object(row) && typeof row.key === 'string' && isParty(row.licensor) && isParty(row.licensee)
      && [row.lent, row.required, row.since].every(Number.isInteger) && Array.isArray(row.designs)
      && row.designs.every(design => object(design) && /^\d{1,20}$/.test(design.variant) && (design.name === null || typeof design.name === 'string')))) {
    throw new Error('Invalid production licences reply');
  }
  return reply;
}

/** One licence standing at the hour shown, by its parties' tags (null where a party has no country then) and canonical
 * slots (null where not linked). */
export interface LicenceRow {
  key: string;
  licensor: string | null;
  licensee: string | null;
  licensorSlot: number | null;
  licenseeSlot: number | null;
  lent: number;
  required: number;
  since: number;
  designs: LicenceDesign[];
}

const row = (licence: LicenceNow): LicenceRow => ({ key: licence.key, licensor: licence.licensor.tag, licensee: licence.licensee.tag,
  licensorSlot: licence.licensor.slot, licenseeSlot: licence.licensee.slot, lent: licence.lent, required: licence.required, since: licence.since, designs: licence.designs });

/** Every licence whose licensor or licensee the matcher keeps at `hour`, and how many others it cannot rule out: a
 * party with no country then may be one it keeps. */
export function licencesAt(reply: ProductionLicencesReply, hour: number, matches: (tag: string, hour: number) => boolean): { rows: LicenceRow[]; unattributed: number } {
  const rows: LicenceRow[] = [];
  let unattributed = 0;
  for (const licence of reply.licences) {
    const { licensor, licensee } = row(licence);
    if (licensor !== null && matches(licensor, hour) || licensee !== null && matches(licensee, hour)) rows.push(row(licence));
    else if (licensor === null || licensee === null) unattributed++;
  }
  return { rows, unattributed };
}

/** A country picked from the diagram: its canonical slot where its node's licences name one, so the drill keeps it
 * through a change of tag, and the tag it was picked by. */
export interface PickedCountry { slot: number | null; tag: string }

/** The canonical slot the licences give `tag`'s country, null where none of its licences is linked. */
export function slotOf(reply: ProductionLicencesReply, tag: string): number | null {
  for (const licence of reply.licences) for (const party of [licence.licensor, licence.licensee]) if (party.tag === tag && party.slot !== null) return party.slot;
  return null;
}

const holds = (party: LicenceParty, picked: PickedCountry) => picked.slot !== null ? party.slot === picked.slot : party.tag === picked.tag;

/** The licences one country grants or holds, and how many more it may grant: their licensor has no country then. */
export function countryLicences(reply: ProductionLicencesReply, picked: PickedCountry): { rows: LicenceRow[]; unattributed: number } {
  const rows = reply.licences.filter(licence => holds(licence.licensor, picked) || holds(licence.licensee, picked)).map(row);
  return { rows, unattributed: reply.licences.filter(licence => licence.licensor.slot === null && licence.licensor.tag === null && !holds(licence.licensee, picked)).length };
}

/** The tag the picked country holds in `reply`, null where none of its licences names its country. */
export function pickedTag(reply: ProductionLicencesReply, picked: PickedCountry): string | null {
  if (picked.slot === null) return picked.tag;
  for (const licence of reply.licences) for (const party of [licence.licensor, licence.licensee]) if (party.slot === picked.slot && party.tag !== null) return party.tag;
  return null;
}

/** Why a group selection shows no licence at `hour`: its membership was not recorded, or it has no members; null where
 * it has members or is not a group. */
export function groupReasonAt(selection: CountrySelection, groups: CountryGroupReply | undefined, hour: number,
  lastHour: number): { kind: 'empty' | 'unavailable'; message: string } | null {
  if (selection.kind !== 'group' || selection.group.type === 'top' || !groups) return null;
  const runs = groupRuns(groups, selection.group);
  if (!runs) return null;
  const members = groupMembers(runs, selection.membership, hour, lastHour, Infinity, groupUnobserved(groups, selection.group));
  if (!members) return { kind: 'unavailable', message: 'This group’s membership was not recorded at this date.' };
  if (members.length) return null;
  return { kind: 'empty', message: selection.membership === 'hour' ? 'This group has no members at this date.'
    : selection.membership === 'end' ? 'This group has no members at the end of this campaign.' : 'This group has no members in this campaign.' };
}

/** A licensor → licensee pair: its licences and the ribbon's width in the measure. */
export interface LicencePair { licensor: string | null; licensee: string | null; licences: LicenceRow[]; width: number }

export function licencePairs(rows: readonly LicenceRow[], width: LicenceWidth): LicencePair[] {
  const pairs = new Map<string, LicencePair>();
  for (const licence of rows) {
    const key = JSON.stringify([licence.licensor, licence.licensee]);
    const pair = pairs.get(key) ?? { licensor: licence.licensor, licensee: licence.licensee, licences: [], width: 0 };
    pair.licences.push(licence);
    pair.width += width === 'licences' ? 1 : licence.lent;
    pairs.set(key, pair);
  }
  return [...pairs.values()];
}

export const licenceRevision = (licence: LicenceRow) => [licence.licensor, licence.licensee, licence.lent, licence.required, licence.since, ...licence.designs.map(design => design.name)];
