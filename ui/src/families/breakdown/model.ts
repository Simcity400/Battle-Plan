import { colour } from '../../charts/palette.ts';
import type { TooltipContent } from '../../charts/tooltip.ts';
import { mix } from '../../shell/appearance/color.ts';
import type { BoundedReadView, FamilyTheme } from '../contract.ts';
import type { ArtResult } from '../plan.ts';
import { buildTooltipContent, type TooltipListItem } from '../host/tooltips.ts';
import type { BreakdownViewMode } from './options.ts';

export interface BreakdownPart {
  readonly key: string;
  readonly name: string;
  readonly ordinal: number;
  readonly value: number | null;
  readonly text: string;
  readonly exact?: string;
  readonly share?: number | null;
  readonly parentShare?: number | null;
  readonly color?: string;
  readonly neutral?: boolean;
  readonly flag?: string;
  readonly flagArt?: ArtResult;
  readonly icon?: string | null;
  readonly notRecorded?: boolean;
  readonly missingReason?: string;
  readonly partial?: boolean;
  readonly atLeast?: boolean;
  readonly partialReason?: string;
  readonly count?: number;
  readonly countLabel?: string;
  readonly subtitle?: string;
  readonly note?: string;
  readonly detailRows?: readonly TooltipListItem[];
  readonly meta?: unknown;
  readonly children?: readonly BreakdownPart[];
}

export interface BreakdownTotal {
  readonly known: boolean;
  readonly atLeast?: boolean;
  readonly partial?: boolean;
  readonly value: number | null;
  readonly text: string;
  readonly exact?: string;
  readonly unit: string;
  readonly verb: string;
  readonly reason?: string;
  readonly shortReason?: string;
}

export interface BreakdownComparison {
  readonly label: string;
  readonly value: string;
  readonly note?: string;
}

export interface BreakdownModel {
  readonly hour: number;
  readonly view?: BreakdownViewMode;
  readonly rootLabel: string;
  readonly periodLabel?: string;
  readonly total: BreakdownTotal;
  readonly comparison?: BreakdownComparison;
  readonly parts: readonly BreakdownPart[];
  readonly empty?: string;
  readonly emptyDetail?: string;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly warnings?: readonly string[];
}

export interface BreakdownProjection<TOptions = unknown> {
  readonly family: 'breakdown';
  readonly kind: string;
  project(readView: BoundedReadView, options: TOptions): BreakdownModel;
}

export interface ResolvedBreakdownLevel {
  readonly parts: readonly BreakdownPart[];
  readonly total: BreakdownTotal;
}

export const PATH = '\u001f';
export const PARTIAL_KEY = '__partial__';
export const COMPARISON_KEY = '__comparison__';

const TOOLTIP_PARTS = 8;

export function formatShare(share: number | null | undefined): string {
  if (share == null || !Number.isFinite(share) || share < 0) return '';
  const pct = share * 100;
  if (pct === 0) return '0%';
  if (pct < 0.05) return '<0.1%';
  if (pct >= 99.95) return '100%';
  if (pct >= 10) return `${Math.round(pct)}%`;
  return `${pct.toFixed(1)}%`;
}

export function partColour(part: BreakdownPart, theme: FamilyTheme): string {
  if (part.color) return part.color;
  if (part.neutral) return theme.grey;
  return colour(theme.surface, part.ordinal);
}

export function tintColour(hex: string, amount: number): string {
  return mix(hex, '#ffffff', amount);
}

export function shadeColour(hex: string, amount: number): string {
  return mix(hex, '#000000', amount);
}

export function childTint(parent: string, child: BreakdownPart, theme: FamilyTheme, tints: readonly number[]): string {
  if (child.neutral) return theme.grey;
  return tintColour(parent, tints[child.ordinal % tints.length]!);
}

export function breakdownLevel(model: BreakdownModel): ResolvedBreakdownLevel {
  return { parts: model.parts, total: model.total };
}

export function rowsBelow(part: BreakdownPart): number {
  if (part.notRecorded || !part.children) return 0;
  let rows = 0;
  for (const child of part.children) rows += 1 + (child.notRecorded ? 0 : child.children?.length ?? 0);
  return rows;
}

export function computeDefaultExpanded(
  parts: readonly BreakdownPart[],
  visibleRows: number,
  narrow: boolean,
): Record<string, boolean> {
  const expanded: Record<string, boolean> = {};
  if (narrow) return expanded;
  let budget = visibleRows - parts.length;
  if (budget < 3) return expanded;
  for (const candidate of parts) {
    const below = rowsBelow(candidate);
    if (!below) continue;
    if (budget < 3) break;
    expanded[candidate.key] = true;
    budget -= below;
  }
  return expanded;
}

export function resolveExpandedMap(
  parts: readonly BreakdownPart[],
  visibleRows: number,
  narrow: boolean,
  userToggles: Readonly<Record<string, boolean>>,
): Record<string, boolean> {
  return { ...computeDefaultExpanded(parts, visibleRows, narrow), ...userToggles };
}

export function resolveRingOpenKey(parts: readonly BreakdownPart[]): string | null {
  return parts.find(part => rowsBelow(part) > 0)?.key ?? null;
}

export function findBreakdownPart(
  parts: readonly BreakdownPart[],
  key: string,
  parent: BreakdownPart | null = null,
): { readonly part: BreakdownPart; readonly parent: BreakdownPart | null } | null {
  for (const part of parts) {
    if (part.key === key) return { part, parent };
    if (part.children && key.startsWith(`${part.key}${PATH}`)) {
      const deeper = findBreakdownPart(part.children, key, part);
      if (deeper) return deeper;
    }
  }
  return null;
}

function withUnit(text: string, unit: string): string {
  return unit ? `${text} ${unit}` : text;
}

export function buildBreakdownTooltip(
  model: BreakdownModel,
  level: ResolvedBreakdownLevel,
  targetKey: string,
  theme: FamilyTheme,
): TooltipContent | null {
  if (targetKey === PARTIAL_KEY) {
    return buildTooltipContent({
      title: 'Partial',
      hour: model.hour,
      items: [{ label: level.total.verb, value: withUnit(level.total.text, level.total.unit), strong: true }],
      note: level.total.reason ?? level.total.shortReason ?? 'Part of this total was not recorded.',
    });
  }
  if (targetKey === COMPARISON_KEY) {
    const comparison = model.comparison;
    if (!comparison) return null;
    return buildTooltipContent({
      title: comparison.label,
      hour: model.hour,
      items: [{ label: comparison.label, value: comparison.value, strong: true }],
      ...(comparison.note ? { note: comparison.note } : {}),
    });
  }
  const found = findBreakdownPart(level.parts, targetKey);
  if (!found) return null;
  const { part, parent } = found;
  const items: TooltipListItem[] = [];
  const unit = level.total.unit;
  const swatch = parent ? tintColour(partColour(parent, theme), 0.16) : partColour(part, theme);
  if (part.notRecorded || part.value === null) {
    items.push({ label: level.total.verb, missingReason: part.missingReason ?? 'Not recorded', swatch });
  } else {
    items.push({ label: level.total.verb, value: withUnit(part.exact ?? part.text, unit), swatch, strong: true });
    if (!level.total.atLeast && part.share != null) items.push({ label: 'Share', value: formatShare(part.share) });
    if (!level.total.atLeast && parent && part.parentShare != null) items.push({ label: `Share of ${parent.name}`, value: formatShare(part.parentShare) });
    if (part.count !== undefined && part.count > 0) items.push({ label: part.countLabel ?? 'Count', value: part.count.toLocaleString('en-US') });
  }
  if (part.partial && part.partialReason) items.push({ label: 'Coverage', value: part.partialReason, tone: 'muted' });
  if (part.detailRows) items.push(...part.detailRows);
  const children = part.notRecorded ? [] : part.children ?? [];
  children.slice(0, TOOLTIP_PARTS).forEach((child, at) => items.push({
    label: child.name,
    ...(child.notRecorded || child.value === null ? { missingReason: child.missingReason ?? 'Not recorded' } : { value: withUnit(child.text, unit) }),
    indent: true,
    ...(at === 0 ? { divider: true } : {}),
  }));
  if (children.length > TOOLTIP_PARTS) items.push({ label: `${children.length - TOOLTIP_PARTS} more in the list`, tone: 'muted', indent: true });
  const subtitle = part.subtitle ?? parent?.name;
  const note = [part.note, level.total.atLeast ? level.total.reason : undefined].filter((entry): entry is string => Boolean(entry && entry.trim())).join(' ');
  return buildTooltipContent({
    title: part.name,
    hour: model.hour,
    ...(subtitle ? { subtitle } : {}),
    items,
    ...(note ? { note } : {}),
  });
}
