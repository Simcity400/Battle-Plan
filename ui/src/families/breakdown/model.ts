import { colour } from '../../charts/palette.ts';
import type { TooltipContent } from '../../charts/tooltip.ts';
import { tintColour } from '../../shell/appearance/color.ts';
import type { AuditRow } from '../../draw/notifications';
import type { BoundedReadView, FamilyTheme, MirrorModel, MirrorRow } from '../contract.ts';
import type { ArtResult } from '../plan.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import { buildTooltipContent, type FamilyTooltipPayload, type TooltipListItem } from '../host/tooltips.ts';

export interface BreakdownPart {
  readonly key: string;
  readonly name: string;
  readonly ordinal: number;
  readonly value: number | null;
  readonly text: string;
  readonly exact?: string;
  readonly share?: number | null;
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

export function breakdownLevel(model: BreakdownModel): ResolvedBreakdownLevel {
  return { parts: model.parts, total: model.total };
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
    if (candidate.notRecorded || !candidate.children?.length) continue;
    if (budget < 3) break;
    expanded[candidate.key] = true;
    budget -= candidate.children!.length;
  }
  return expanded;
}

export function resolveExpandedMap(
  parts: readonly BreakdownPart[],
  visibleRows: number,
  narrow: boolean,
  userToggles: Readonly<Record<string, boolean>>,
): Record<string, boolean> {
  const defaults = computeDefaultExpanded(parts, visibleRows, narrow);
  const resolved: Record<string, boolean> = { ...defaults };
  for (const [key, open] of Object.entries(userToggles)) {
    resolved[key] = open;
  }
  return resolved;
}

export function resolveRingOpenKey(parts: readonly BreakdownPart[]): string | null {
  return parts.find(part => !part.notRecorded && part.children?.length)?.key ?? null;
}

export function findBreakdownPart(
  parts: readonly BreakdownPart[],
  key: string,
): { readonly part: BreakdownPart; readonly parent: BreakdownPart | null } | null {
  for (const part of parts) {
    if (part.key === key) return { part, parent: null };
    if (part.children) {
      for (const child of part.children) {
        if (child.key === key) return { part: child, parent: part };
        if (child.children) {
          const deeper = findBreakdownPart(child.children, key);
          if (deeper) return deeper;
        }
      }
    }
  }
  return null;
}

export function buildBreakdownTooltip(
  model: BreakdownModel,
  level: ResolvedBreakdownLevel,
  targetKey: string,
  theme: FamilyTheme,
): TooltipContent | null {
  if (targetKey === '__partial__') {
    const reason = level.total.reason ?? level.total.shortReason ?? 'Part of this total was not recorded.';
    const payload: FamilyTooltipPayload = {
      title: 'Partial',
      hour: model.hour,
      items: [
        {
          label: level.total.verb,
          value: level.total.unit ? `${level.total.text} ${level.total.unit}` : level.total.text,
          strong: true,
        },
      ],
      note: reason,
    };
    return buildTooltipContent(payload);
  }
  const found = findBreakdownPart(level.parts, targetKey);
  if (!found) return null;
  const { part, parent } = found;
  const items: TooltipListItem[] = [];
  const unit = level.total.unit;
  const swatch = parent ? tintColour(partColour(parent, theme), 0.16) : partColour(part, theme);
  if (part.notRecorded || part.value === null) {
    items.push({
      label: level.total.verb,
      missingReason: part.missingReason ?? 'Not recorded',
      swatch,
    });
  } else {
    const valStr = unit ? `${part.exact ?? part.text} ${unit}` : (part.exact ?? part.text);
    items.push({
      label: level.total.verb,
      value: valStr,
      swatch,
      strong: true,
    });
    if (!level.total.atLeast && part.share != null) {
      items.push({
        label: parent ? `Share of ${parent.name}` : 'Share',
        value: formatShare(part.share),
      });
    }
    if (part.count !== undefined && part.count > 0) {
      items.push({
        label: part.countLabel ?? 'Count',
        value: part.count.toLocaleString('en-US'),
      });
    }
  }
  if (part.partial && part.partialReason) {
    items.push({
      label: 'Coverage',
      value: part.partialReason,
      tone: 'muted',
    });
  }
  if (part.detailRows) {
    for (const row of part.detailRows) items.push(row);
  }
  const shown = part.notRecorded ? [] : (part.children ?? []).slice(0, TOOLTIP_PARTS);
  shown.forEach((child, at) => items.push({
    label: child.name,
    ...(child.notRecorded || child.value === null ? { missingReason: child.missingReason ?? 'Not recorded' } : { value: unit ? `${child.text} ${unit}` : child.text }),
    indent: true,
    ...(at === 0 ? { divider: true } : {}),
  }));
  const subtitle = part.subtitle ?? (parent ? parent.name : undefined);
  const note = [part.note, level.total.atLeast ? level.total.reason : undefined]
    .filter((entry): entry is string => Boolean(entry && entry.trim()))
    .join(' ');
  return buildTooltipContent({
    title: part.name,
    hour: model.hour,
    ...(subtitle ? { subtitle } : {}),
    items,
    ...(note ? { note } : {}),
  });
}

export function deriveBreakdownNavAndMirror(
  level: ResolvedBreakdownLevel,
  expanded: Readonly<Record<string, boolean>>,
): { readonly nav: readonly FamilyNavTuple[]; readonly mirror: MirrorModel } {
  const nav: FamilyNavTuple[] = [];
  const rows: MirrorRow[] = [];
  const unit = level.total.unit;
  let index = 0;
  for (const part of level.parts) {
    const hasChildren = Boolean(part.children && part.children.length > 0);
    const isOpen = hasChildren && expanded[part.key] === true;
    const valueSpoken = part.notRecorded || part.value === null
      ? `not recorded${part.missingReason ? `: ${part.missingReason}` : ''}`
      : `${part.exact ?? part.text}${unit ? ` ${unit}` : ''}${!level.total.atLeast && part.share != null ? `, ${formatShare(part.share)}` : ''}`;
    nav.push([part.key, part.name, index, hasChildren ? (isOpen ? 1 : 0) : -1, '', null]);
    rows.push({ key: part.key, name: part.name, rowIndex: index, level: 0, ...(hasChildren ? { group: true, open: isOpen } : {}), cells: [{ column: 'value', spoken: valueSpoken }] });
    index++;
    if (isOpen && part.children) {
      for (const child of part.children) {
        const childSpoken = child.notRecorded || child.value === null
          ? `not recorded${child.missingReason ? `: ${child.missingReason}` : ''}`
          : `${child.exact ?? child.text}${unit ? ` ${unit}` : ''}${!level.total.atLeast && child.share != null ? `, ${formatShare(child.share)}` : ''}`;
        nav.push([child.key, child.name, index, -1, part.key, null]);
        rows.push({ key: child.key, name: `${part.name}, ${child.name}`, rowIndex: index, level: 1, cells: [{ column: 'value', spoken: childSpoken }] });
        index++;
      }
    }
  }
  return { nav, mirror: { first: 0, total: rows.length, rows } };
}

export function deriveBreakdownAudit(
  model: BreakdownModel,
): readonly AuditRow[] {
  const level = breakdownLevel(model);
  const rows: AuditRow[] = [];
  const totalVal = level.total.value === null
    ? `notRecorded:${level.total.reason ?? 'Not recorded'}`
    : level.total.atLeast
      ? `partial:${level.total.exact ?? level.total.text}`
      : level.total.partial
        ? `partial:${level.total.exact ?? level.total.text}`
        : (level.total.exact ?? level.total.text);
  rows.push({ row: '__total__', key: '__total__', value: totalVal });
  for (const part of level.parts) {
    const val = part.notRecorded || part.value === null
      ? `notRecorded:${part.missingReason ?? 'Not recorded'}`
      : part.partial
        ? `partial:${part.exact ?? part.text}`
        : (part.exact ?? part.text);
    rows.push({ row: part.key, key: part.key, value: val });
  }
  return rows;
}
