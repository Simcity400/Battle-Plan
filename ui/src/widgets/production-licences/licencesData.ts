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
  licensor: string;
  licensee: string;
  /** The licensor's and licensee's holders: their identity whatever tags they hold. */
  licensorHolder: string;
  licenseeHolder: string;
  /** Civilian factories lent and required; null where not recorded. */
  lent: bigint | null;
  required: bigint | null;
  /** The native start date's hour. */
  since: number | null;
  designs: { variant: string; name: string | null }[];
}

/** Every licence standing at `hour` whose licensor or licensee the matcher keeps. */
export function licencesAt(reply: ProductionCatalogueReply, hour: number, matches: (tag: string, hour: number) => boolean): LicenceAt[] {
  const rows: LicenceAt[] = [];
  for (const licence of reply.licences) {
    const giver = valueAt(licence.licensor, hour);
    if (giver === null) continue;
    const licensor = tagAt(reply.holders[giver], hour), licensee = tagAt(reply.holders[licence.licensee], hour);
    if (licensor === null || licensee === null || !(matches(licensor, hour) || matches(licensee, hour))) continue;
    rows.push({ key: `${licence.licensee}:${licence.ordinal}`, licence, licensor, licensee, licensorHolder: giver, licenseeHolder: licence.licensee,
      lent: bigAt(licence.lent, hour), required: bigAt(licence.required, hour),
      since: numberAt(licence.start, hour), designs: licence.variants.flatMap(runs => {
        const run = runAt(runs, hour);
        return run ? [{ variant: run[2], name: variantNameAt(reply.variants[run[2]], hour) }] : [];
      }) });
  }
  return rows;
}

/** The holders that held `tag` at `hour`: the countries a node of that tag then stands for. */
export function holdersAt(reply: Pick<ProductionCatalogueReply, 'holders'>, tag: string, hour: number): string[] {
  return Object.keys(reply.holders).filter(holder => tagAt(reply.holders[holder], hour) === tag);
}

/** Every licence standing at `hour` that one of `holders` grants or holds, by whatever tags they hold then. */
export function holderLicencesAt(reply: ProductionCatalogueReply, holders: readonly string[], hour: number): LicenceAt[] {
  const own = new Set(holders);
  return licencesAt(reply, hour, () => true).filter(row => own.has(row.licensorHolder) || own.has(row.licenseeHolder));
}

/** A licensor → licensee pair: its licences and the ribbon's width in the measure. */
export interface LicencePair { licensor: string; licensee: string; licences: LicenceAt[]; width: number }

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
