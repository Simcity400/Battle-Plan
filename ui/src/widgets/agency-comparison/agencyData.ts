import type { TooltipContent } from '../../charts/tooltip';
import { formatFixed } from '../../charts/valueAxis.ts';
import { compareCountries } from '../../dashboard/intelligence.ts';
import type { AgencyRankField } from './types';
import type { Agency, AgencyComparisonReply, Upgrades } from '../../generated/dashboard';

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const integer = (value: unknown): value is number => Number.isSafeInteger(value);
const text = (value: unknown): value is string | null => value === null || typeof value === 'string';
const raw = (value: unknown): value is string => typeof value === 'string' && /^-?\d{1,20}$/.test(value);
const country = (value: unknown) => object(value) && integer(value.id) && (value.tag === null || (typeof value.tag === 'string' && value.tag.length > 0));
const upgrades = (value: unknown): value is Upgrades => object(value) && text(value.unavailable)
  && (value.nonnegativeLevelSum === null || (typeof value.nonnegativeLevelSum === 'string' && /^(0|[1-9]\d*)$/.test(value.nonnegativeLevelSum)))
  && (value.unavailable === null) === (value.nonnegativeLevelSum !== null)
  && Array.isArray(value.items) && value.items.every((item, index, items) => object(item) && integer(item.ordinal) && item.ordinal >= 0
    && (!index || (items[index - 1] as { ordinal: number }).ordinal < item.ordinal) && text(item.branch) && text(item.token) && integer(item.level)
    && text(item.name) && text(item.missing) && (item.name === null) !== (item.missing === null));

export function validateAgencies(value: unknown): AgencyComparisonReply {
  if (!object(value) || !Array.isArray(value.agencies) || !value.agencies.every(item => object(item) && country(item.country) && typeof item.created === 'boolean'
    && text(item.name) && text(item.nameMissing) && raw(item.defenseRaw) && text(item.defenseText) && text(item.defenseValue) && integer(item.activeCount)
    && integer(item.maximum) && (item.upgrades === null || upgrades(item.upgrades))) || !text(value.defenseLabel) || !text(value.upgradesMissing)
    || !text(value.missing) || !Array.isArray(value.warnings) || !value.warnings.every(item => typeof item === 'string')) {
    throw new Error('The agency comparison reply is malformed.');
  }
  return value as unknown as AgencyComparisonReply;
}

export const agencyStructure = (reply: AgencyComparisonReply) => JSON.stringify(reply);

const rankValue = (agency: Agency, field: AgencyRankField): bigint => field === 'defense' ? BigInt(agency.defenseRaw)
  : BigInt(field === 'active' ? agency.activeCount : agency.maximum);

export function agencyRows(reply: AgencyComparisonReply | undefined, hour: number, matches: (tag: string, hour: number) => boolean, all: boolean,
  top: { count: number; field: AgencyRankField } | null): Agency[] {
  if (!reply) return [];
  if (top) {
    return [...reply.agencies].sort((a, b) => {
      const x = rankValue(a, top.field), y = rankValue(b, top.field);
      return x === y ? compareCountries(a.country, b.country) : x > y ? -1 : 1;
    }).slice(0, top.count);
  }
  return reply.agencies.filter(agency => agency.country.tag ? matches(agency.country.tag, hour) : all);
}

export const agencyName = (agency: Agency) => agency.name ?? 'Unknown';

export function upgradesCell(agency: Agency): string {
  const held = agency.upgrades;
  if (!held) return 'Not recorded';
  return held.nonnegativeLevelSum === null ? 'Unknown' : formatFixed(BigInt(held.nonnegativeLevelSum), 0, 0);
}

export const operativesCell = (agency: Agency): string => `${formatFixed(BigInt(agency.activeCount), 0, 0)} / ${formatFixed(BigInt(agency.maximum), 0, 0)}`;

export function agencyTooltip(agency: Agency, name: string, upgradesMissing: string | null): TooltipContent {
  return { title: name, subtitle: agency.name ?? 'Agency name unknown', rows: [
    { label: 'Agency', value: agency.created ? 'Created' : 'Not created' },
    { label: 'Defence', value: agency.defenseText ?? 'Unknown' },
    { label: 'Operatives active / maximum', value: operativesCell(agency) },
    { label: 'Agency upgrades', value: upgradesCell(agency) },
    ...(agency.upgrades?.items ?? []).map(item => ({ label: item.name ?? 'Upgrade name unavailable', value: `Level ${item.level}`,
      ...(item.missing ? { detail: item.missing } : {}) })),
  ], note: [agency.nameMissing, agency.upgrades?.unavailable ?? (agency.upgrades ? null : upgradesMissing)].filter(Boolean).join('\n') || undefined };
}
