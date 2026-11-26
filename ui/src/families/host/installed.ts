import { displayedHour } from '../boundedView.ts';
import type { InstalledAction, InstalledPicture } from '../contract.ts';
import { REGION_ADD_KEY, regionRowOf, type RegionRowRef, type RegionRowsField } from '../regions/regionRows.ts';
import { parseFamilyHit, type FamilyContextHit, type FamilyHostAdapter, type ParsedFamilyHit } from './adapter.ts';
import { toggleExpansion, type ExpansionToggle, type FamilyExpansion } from './expansion.ts';
import type { FamilyNavTuple } from './keyboard.ts';
import type { FamilyTooltipEntry, FamilyTooltipField } from './tooltips.ts';

export interface InstalledRecord {
  readonly picture: InstalledPicture;
  readonly fields: Readonly<Record<string, unknown>>;
  readonly nav: readonly FamilyNavTuple[];
  readonly selectedKey: string | null;
}

export type DrawnKeys = Readonly<Record<string, readonly string[]>>;

export function displayedHoverKeys(
  name: string,
  drawn: DrawnKeys | undefined,
  tiled: Readonly<Record<string, ReadonlyMap<string, unknown>>> | undefined,
): readonly string[] | undefined {
  const keys = drawn?.[name];
  if (keys !== undefined) return keys;
  const series = tiled?.[name];
  return series ? [...series.keys()] : undefined;
}

const RECORDS = new WeakMap<InstalledPicture, InstalledRecord>();
const EMPTY_NAV: readonly FamilyNavTuple[] = [];

export function recordInstalled(
  picture: InstalledPicture,
  fields: Readonly<Record<string, unknown>>,
  selectedKey: string | null,
): InstalledRecord {
  const record: InstalledRecord = {
    picture,
    fields,
    nav: (fields.nav as readonly FamilyNavTuple[] | undefined) ?? EMPTY_NAV,
    selectedKey,
  };
  RECORDS.set(picture, record);
  return record;
}

export function installedRecordOf(picture: InstalledPicture | null | undefined): InstalledRecord | null {
  return picture ? RECORDS.get(picture) ?? null : null;
}

export interface HostInputGate {
  readonly current: InstalledRecord | null;
  readonly held: boolean;
  readonly expired: boolean;
}

export function inputBlocked(gate: HostInputGate): boolean {
  return gate.held || gate.expired || gate.current === null;
}

export type HostCommand = 'activate' | 'toggle' | 'menu' | 'pin';

export interface HostInput {
  readonly key: string | null;
  readonly hit: ParsedFamilyHit | null;
  readonly picture: InstalledPicture | null;
}

export interface HostInputOptions {
  readonly expansion: FamilyExpansion;
  readonly takesToggles: boolean;
  readonly regions: boolean;
  readonly menu: boolean;
}

export type HostEffect =
  | { readonly kind: 'activate'; readonly key: string; readonly action: InstalledAction }
  | { readonly kind: 'toggle'; readonly action: InstalledAction; readonly expansion: ExpansionToggle | null }
  | { readonly kind: 'menu'; readonly hit: FamilyContextHit }
  | { readonly kind: 'pin'; readonly tip: FamilyTooltipEntry; readonly hour: number }
  | { readonly kind: 'region-open'; readonly field: RegionRowsField }
  | { readonly kind: 'region-remove'; readonly row: RegionRowRef };

function gridHas(adapter: FamilyHostAdapter<any, any, any, any>, record: InstalledRecord, key: string, parseGridKey: (key: string | null) => { readonly row: number; readonly column: number } | null): boolean {
  const metrics = adapter.getGridMetrics?.(record.fields);
  const cell = (adapter.parseGridKey ?? parseGridKey)(key);
  return !!metrics && !!cell && cell.row < metrics.rows && cell.column < metrics.columns;
}

function actionOf(adapter: FamilyHostAdapter<any, any, any, any>, record: InstalledRecord, hit: ParsedFamilyHit, toggle: boolean): InstalledAction | null {
  const installed = record.picture;
  if (adapter.resolveHitAction) return adapter.resolveHitAction(hit, installed, record.fields);
  const target = installed.actionKeys.get(hit.key);
  const label = hit.label ?? target?.label;
  const meta = hit.meta ?? target?.meta;
  return {
    installedId: installed.id,
    owner: installed.owner,
    subject: hit.subject ?? target?.subject ?? hit.key,
    hour: displayedHour(hit.hour ?? target?.hour ?? installed.hour),
    ...(toggle || target?.toggle ? { toggle: true } : {}),
    ...(label !== undefined ? { label } : {}),
    ...(meta !== undefined ? { meta } : {}),
  };
}

export function hoveredSubject(adapter: FamilyHostAdapter<any, any, any, any>, record: InstalledRecord | null, hit: ParsedFamilyHit | null): string | null {
  return record && hit ? actionOf(adapter, record, hit, false)?.subject ?? null : null;
}

function pointsHas(adapter: FamilyHostAdapter<any, any, any, any>, record: InstalledRecord, key: string): boolean {
  if (record.picture.actionKeys.has(key)) return true;
  const parsed = parseFamilyHit(adapter, key);
  if (!parsed || parsed.kind !== 'plot' || parsed.hour === undefined) return false;
  const transform = (record.fields.meta as { readonly transform?: { readonly axisFrom?: number; readonly axisTo?: number } } | undefined)?.transform;
  if (!transform || transform.axisFrom === undefined || transform.axisTo === undefined) return false;
  return parsed.hour >= transform.axisFrom && parsed.hour <= transform.axisTo;
}

export function focusedSubject(
  adapter: FamilyHostAdapter<any, any, any, any>,
  record: InstalledRecord | null,
  key: string | null,
  parseGridKey: (key: string | null) => { readonly row: number; readonly column: number } | null,
): string | null {
  if (!record || key === null) return null;
  const shown = adapter.navigation === 'grid'
    ? gridHas(adapter, record, key, parseGridKey)
    : adapter.navigation === 'points'
      ? pointsHas(adapter, record, key)
      : record.picture.actionKeys.has(key);
  return shown ? actionOf(adapter, record, { key }, false)?.subject ?? key : null;
}

export function resolveHostInput(
  adapter: FamilyHostAdapter<any, any, any, any>,
  command: HostCommand,
  input: HostInput,
  gate: HostInputGate,
  options: HostInputOptions,
  parseGridKey: (key: string | null) => { readonly row: number; readonly column: number } | null,
): HostEffect | null {
  const record = installedRecordOf(input.picture);
  if (inputBlocked(gate) || !record || record.picture.owner !== gate.current!.picture.owner) return null;
  const picture = record.picture;
  const { key, hit } = input;
  const hasKey = key === null || !!hit || (
    adapter.navigation === 'grid'
      ? gridHas(adapter, record, key, parseGridKey)
      : adapter.navigation === 'points'
        ? pointsHas(adapter, record, key)
        : picture.actionKeys.has(key)
  );
  if (!hasKey) return null;
  if (command === 'menu') {
    if (!options.menu) return null;
    const target = key ? picture.actionKeys.get(key) : undefined;
    const parsedHit = key !== null && !hit ? parseFamilyHit(adapter, key) : null;
    const resolved = key !== null && !hit && adapter.resolveHitAction ? adapter.resolveHitAction(parsedHit ?? { key }, picture, record.fields) : null;
    const hitHour = hit?.hour ?? parsedHit?.hour ?? resolved?.hour ?? target?.hour ?? null;
    return {
      kind: 'menu',
      hit: {
        key,
        subject: hit?.subject ?? parsedHit?.subject ?? target?.subject ?? key,
        label: hit?.label ?? target?.label ?? key,
        hour: hitHour ?? picture.hour,
        hitHour,
        ...(hit?.kind !== undefined ? { kind: hit.kind } : {}),
        ...(hit?.meta !== undefined ? { meta: hit.meta } : target?.meta !== undefined ? { meta: target.meta } : {}),
        installed: picture,
      },
    };
  }
  if (key === null) return null;
  if (command === 'pin') {
    const tip = (record.fields.focusTooltip as FamilyTooltipField | undefined)?.text;
    return tip && tip.rowKey === key ? { kind: 'pin', tip, hour: tip.hour ?? picture.hour } : null;
  }
  if (command === 'activate' && options.regions) {
    const field = record.fields.regions as RegionRowsField | undefined;
    if (field && (key === REGION_ADD_KEY || hit?.kind === 'region-add')) return { kind: 'region-open', field };
    if (field && hit?.kind === 'region-remove') {
      const row = regionRowOf(field, hit.key);
      return row ? { kind: 'region-remove', row } : null;
    }
  }
  const toggle = command === 'toggle';
  const action = actionOf(adapter, record, hit ?? parseFamilyHit(adapter, key) ?? { key }, toggle);
  if (!action) return null;
  if (!toggle && !action.toggle) return { kind: 'activate', key, action };
  const toggled = { ...action, toggle: true };
  if (options.takesToggles) return { kind: 'toggle', action: toggled, expansion: null };
  const expansion = toggleExpansion(options.expansion, record.nav, toggled.subject, adapter.exclusiveExpansion?.(record.fields) ?? false);
  return expansion ? { kind: 'toggle', action: toggled, expansion } : null;
}

export class BoundSelections {
  #log: (readonly [generation: number, key: string | null])[] = [];

  bound(generation: number, key: string | null): void {
    const last = this.#log.at(-1);
    if (last && last[0] >= generation) this.#log = this.#log.filter(entry => entry[0] < generation);
    this.#log.push([generation, key]);
  }

  at(generation: number): string | null {
    let found = -1;
    for (let i = 0; i < this.#log.length && this.#log[i]![0] <= generation; i++) found = i;
    if (found < 0) return null;
    if (found > 0) this.#log = this.#log.slice(found);
    return this.#log[0]![1];
  }
}
