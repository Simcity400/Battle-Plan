import { formatAmount, rescale } from '../charts/valueAxis.ts';
import { airChoiceKey } from './airOptions.ts';
import { countryFromKey } from './countryKey.ts';
import { contentKey, contentSet, type OptionValue } from './options.ts';
import { int, object } from './validation.ts';
import { readRegionChoice, regionParams } from '../families/regions/regionChoice.ts';
import { validActiveRegions, validNoRegions } from '../families/regions/regionRows.ts';
import { factionNameAt } from './factionNames.ts';
import type { AirChoicesReply, AirSide, AirSuperiorityReply, FactionGroup } from '../generated/dashboard';

export function airMode(options: Readonly<Record<string, OptionValue>>, campaign: string, currentCountry: string | null) {
  const mode = options.mode === 'viewpoint' ? 'viewpoint' as const : 'sides' as const;
  const chosen = options.viewpoint ? String(options.viewpoint) : '';
  const viewpoint = mode !== 'viewpoint' ? '' : chosen ? contentKey(chosen).startsWith('country:') ? contentKey(chosen) : airChoiceKey(campaign, chosen) ?? ''
    : currentCountry ? `country:${currentCountry}` : '';
  return { mode, viewpoint, sides: contentSet(options.sides).flatMap(value => airChoiceKey(campaign, value) ?? []),
    regions: readRegionChoice(options.regions),
    theatres: contentSet(options.theatres).flatMap(value => airChoiceKey(campaign, value) ?? []) };
}
export type AirModeSettings = ReturnType<typeof airMode>;

export function airParams(mode: AirModeSettings) {
  return { mode: mode.mode, ...(mode.viewpoint ? { viewpoint: mode.viewpoint.startsWith('country:') ? countryFromKey(mode.viewpoint.slice(8)) : mode.viewpoint,
    ...(mode.viewpoint.startsWith('country:') ? { viewpointCountry: true } : {}) } : {}),
    ...(mode.sides.length ? { sides: mode.sides } : {}), ...regionParams(mode.regions),
    ...(mode.theatres.length ? { theatres: mode.theatres } : {}) };
}

export function airPercent(score: number, total: number): string { return total > 0 ? `${Math.round(score * 100 / total)}%` : '—'; }
export const airScore = (score: number, scale: number): string => formatAmount(rescale(BigInt(Math.trunc(score)), scale, 2), 2);

export function factionNamer(dated: readonly FactionGroup[], campaign: readonly FactionGroup[], own: ReadonlyMap<string, string>, at: number | 'not-recorded' | null) {
  const datedById = new Map(dated.map(faction => [faction.id, faction]));
  const campaignById = new Map(campaign.map(faction => [faction.id, faction]));
  return (id: string) => {
    const mine = own.get(id);
    if (mine) return mine;
    if (at === null) return campaignById.get(id)?.name ?? `Faction ${id}`;
    const faction = datedById.get(id);
    if (at === 'not-recorded' || !faction) return 'Name not recorded';
    return factionNameAt(faction, at).name ?? 'Name not recorded';
  };
}

export function factionNameChanges(factions: readonly FactionGroup[]): number[] {
  const hours = new Set<number>();
  for (const faction of factions) for (const run of faction.names) hours.add(run.firstHour).add(run.lastHour + 1);
  return [...hours].sort((a, b) => a - b);
}

export function airSideName(side: AirSide, country: (tag: string) => string, factions: (id: string) => string): string {
  if (side.factions.length) return side.factions.map(factions).join(' / ');
  const tag = side.tag ?? side.leader;
  const name = tag ? country(tag) : 'Coalition (name not recorded)';
  return side.key.startsWith('w:') && side.coBelligerents > 0 ? `${name} and allies` : name;
}

const text = (item: unknown): item is string => typeof item === 'string';
const optionalText = (item: unknown) => item === null || text(item);
const natural = (item: unknown): item is number => int(item) && item >= 0;

function validSide(side: unknown): boolean {
  return object(side) && text(side.key) && optionalText(side.faction) && optionalText(side.colourFaction) && Array.isArray(side.factions) && side.factions.every(text)
    && optionalText(side.tag) && optionalText(side.leader) && natural(side.coBelligerents)
    && (side.anchor === null || object(side.anchor) && text(side.anchor.tag) && int(side.anchor.hour));
}

function validCell(cell: unknown, sides: number): boolean {
  const index = (item: unknown) => natural(item) && item < sides;
  if (!object(cell)) return false;
  if (cell.kind === 'reason') return text(cell.reason);
  if (cell.kind === 'view') return (cell.side === null || index(cell.side)) && optionalText(cell.leader) && optionalText(cell.live);
  return cell.kind === 'sides' && Array.isArray(cell.parts) && cell.parts.every(part => object(part) && index(part.side) && text(part.tag) && text(part.live))
    && Array.isArray(cell.viewpoints) && cell.viewpoints.every(entry => Array.isArray(entry) && entry.length === 2 && index(entry[0]) && text(entry[1]))
    && Array.isArray(cell.unknown) && cell.unknown.every(index) && Array.isArray(cell.noAirObject) && cell.noAirObject.every(index)
    && typeof cell.filtered === 'boolean' && natural(cell.excluded);
}

export function validateAirSuperiority(value: unknown): AirSuperiorityReply {
  const invalid = (): never => { throw new Error('Invalid air-superiority reply'); };
  if (!object(value) || !optionalText(value.missing) || !Array.isArray(value.sides) || !value.sides.every(validSide) || !Array.isArray(value.regions)
    || !Array.isArray(value.warnings) || !value.warnings.every(text) || !natural(value.scale) || value.scale === 0 || !natural(value.badBelow) || !natural(value.goodAbove)
    || value.badBelow > value.goodAbove || value.goodAbove > value.scale) return invalid();
  const sides = value.sides.length;
  if (!(value.viewpoint === null || natural(value.viewpoint) && value.viewpoint < sides) || !validCell(value.fallback, sides) || !validNoRegions(value.empty) || !validActiveRegions(value.activeUnlisted)) return invalid();
  for (const region of value.regions) {
    if (!object(region) || !natural(region.id) || !optionalText(region.name) || !optionalText(region.nameMissing) || typeof region.active !== 'boolean' || typeof region.pinned !== 'boolean'
      || !['live', 'none', 'unknown'].includes(region.wings as string) || !validCell(region.cell, sides)) return invalid();
  }
  return value as AirSuperiorityReply;
}

export function validateAirChoices(value: unknown): AirChoicesReply {
  const choice = (item: unknown) => object(item) && text(item.id) && text(item.name);
  if (!object(value) || !Array.isArray(value.theatres) || !value.theatres.every(choice)
    || !Array.isArray(value.sides) || !value.sides.every(validSide) || !optionalText(value.theatresMissing) || !Array.isArray(value.countries)
    || !value.countries.every(text) || !Array.isArray(value.warnings) || !value.warnings.every(text)) throw new Error('Invalid air-superiority-choices reply');
  return value as AirChoicesReply;
}
