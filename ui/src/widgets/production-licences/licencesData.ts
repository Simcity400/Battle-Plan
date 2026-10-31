import type { LicenceHistory, ProductionCatalogueReply } from '../../generated/dashboard';
import { object } from '../../dashboard/validation.ts';
import { bigAt, hexText, isEconomyRuns, isHolderMap, isVariantNames, numberAt, runAt, tagAt, valueAt, validateEconomyReply, variantNameAt } from '../../dashboard/economyHistory.ts';

export const PRODUCTION_CATALOGUE = 'production-catalogue';
export type LicenceWidth = 'licences' | 'factories';

export function validateLicences(value: unknown): ProductionCatalogueReply {
  return validateEconomyReply<ProductionCatalogueReply>(value, reply => reply.view === 'licences' && isEconomyRuns(reply.known) && isHolderMap(reply.holders) && isVariantNames(reply.variants)
    && Array.isArray(reply.licences) && reply.licences.every(row => object(row) && typeof row.licensee === 'string' && Number.isSafeInteger(row.ordinal)
      && ['licensor', 'lent', 'required', 'start'].every(key => isEconomyRuns(row[key])) && Array.isArray(row.variants) && row.variants.every(runs => isEconomyRuns(runs, hexText))),
  'production licences');
}

/** Whether the view's roster was read complete at `hour`. */
export const rosterKnown = (reply: Pick<ProductionCatalogueReply, 'known'>, hour: number) => runAt(reply.known, hour) !== null;

/** One licence standing at an hour. */
export interface LicenceAt {
  key: string;
  licence: LicenceHistory;
  /** Each party's tag; null where it has no identity or no tag at the hour. */
  licensor: string | null;
  licensee: string | null;
  /** The licensor's and licensee's holders: their identity whatever tags they hold; the
   * licensor's null while it has none. */
  licensorHolder: string | null;
  licenseeHolder: string;
  /** Civilian factories lent and required; null where not recorded. */
  lent: bigint | null;
  required: bigint | null;
  /** The native start date's hour. */
  since: number | null;
  designs: { variant: string; name: string | null }[];
}

/** Every licence standing at `hour` whose licensor or licensee the matcher keeps, and how many
 * others it cannot rule out: a party with no tag then may be a country it keeps. */
export function licencesAt(reply: ProductionCatalogueReply, hour: number, matches: (tag: string, hour: number) => boolean): { rows: LicenceAt[]; unattributed: number } {
  const rows: LicenceAt[] = [];
  let unattributed = 0;
  for (const licence of reply.licences) {
    // Its start date is read every hour it stands; its licensor only while that has an identity.
    if (!runAt(licence.start, hour)) continue;
    const giver = valueAt(licence.licensor, hour);
    const licensor = giver === null ? null : tagAt(reply.holders[giver], hour), licensee = tagAt(reply.holders[licence.licensee], hour);
    if (!(licensor !== null && matches(licensor, hour) || licensee !== null && matches(licensee, hour))) {
      if (licensor === null || licensee === null) unattributed++;
      continue;
    }
    rows.push(licenceAt(reply, licence, hour, giver, licensor, licensee));
  }
  return { rows, unattributed };
}

function licenceAt(reply: ProductionCatalogueReply, licence: LicenceHistory, hour: number, giver: string | null, licensor: string | null,
  licensee: string | null): LicenceAt {
  return { key: `${licence.licensee}:${licence.ordinal}`, licence, licensor, licensee, licensorHolder: giver, licenseeHolder: licence.licensee,
    lent: bigAt(licence.lent, hour), required: bigAt(licence.required, hour),
    since: numberAt(licence.start, hour), designs: licence.variants.flatMap(runs => {
      const run = runAt(runs, hour);
      return run ? [{ variant: run[2], name: variantNameAt(reply.variants[run[2]], hour) }] : [];
    }) };
}

/** The holders that held `tag` at `hour`: the countries a node of that tag then stands for. */
export function holdersAt(reply: Pick<ProductionCatalogueReply, 'holders'>, tag: string, hour: number): string[] {
  return Object.keys(reply.holders).filter(holder => tagAt(reply.holders[holder], hour) === tag);
}

/** Every licence standing at `hour` that one of `holders` grants or holds, by whatever tags they hold then,
 * and how many others one of them may grant: their licensor has no identity then. */
export function holderLicencesAt(reply: ProductionCatalogueReply, holders: readonly string[], hour: number): { rows: LicenceAt[]; unattributed: number } {
  const own = new Set(holders);
  const rows: LicenceAt[] = [];
  let unattributed = 0;
  for (const licence of reply.licences) {
    if (!runAt(licence.start, hour)) continue;
    const giver = valueAt(licence.licensor, hour);
    if (own.has(licence.licensee) || giver !== null && own.has(giver))
      rows.push(licenceAt(reply, licence, hour, giver, giver === null ? null : tagAt(reply.holders[giver], hour), tagAt(reply.holders[licence.licensee], hour)));
    else if (giver === null) unattributed++;
  }
  return { rows, unattributed };
}

/** A licensor → licensee pair: its licences and the ribbon's width in the measure. A party
 * is null for the licences that name it with no identity or no tag. */
export interface LicencePair { licensor: string | null; licensee: string | null; licences: LicenceAt[]; width: number }

export function licencePairs(rows: readonly LicenceAt[], width: LicenceWidth): LicencePair[] {
  const pairs = new Map<string, LicencePair>();
  for (const row of rows) {
    const key = JSON.stringify([row.licensor, row.licensee]);
    const pair = pairs.get(key) ?? { licensor: row.licensor, licensee: row.licensee, licences: [], width: 0 };
    pair.licences.push(row);
    pair.width += width === 'licences' ? 1 : Number(row.lent ?? 0n);
    pairs.set(key, pair);
  }
  return [...pairs.values()];
}
