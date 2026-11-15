import type { TooltipRow } from '../tooltip';

/** Shared part rows keep unknown values separate from exact zero and corrections. */
export interface DrillPart { id: string; label: string; detail?: string; value: number | null; raw?: bigint | null; shareText?: string; color?: string; text?: string; icon?: string; countryFlag?: TooltipRow['countryFlag']; muted?: boolean; selectable?: boolean }

export function orderedDrillParts(parts: readonly DrillPart[]): DrillPart[] {
  return [...parts].sort((a, b) => {
    const unknownA = a.raw === null || a.value == null; const unknownB = b.raw === null || b.value == null;
    if (unknownA || unknownB) return Number(unknownA) - Number(unknownB)
      || (a.raw !== undefined && b.raw !== undefined ? a.id.localeCompare(b.id) : a.label.localeCompare(b.label));
    if (a.raw != null && b.raw != null) {
      const left = a.raw < 0n ? -a.raw : a.raw; const right = b.raw < 0n ? -b.raw : b.raw;
      return (right > left ? 1 : right < left ? -1 : 0) || a.id.localeCompare(b.id);
    }
    return Math.abs(b.value!) - Math.abs(a.value!) || a.label.localeCompare(b.label);
  });
}

/** The part of a total no recorded source explains. */
export interface DrillUnattributed { label: string; value: number; raw?: bigint | null; text?: string; shareText?: string; color?: string }

/** The rows a breakdown shows: its parts largest first, then what no source explains when it is not zero. */
export function drillRows(parts: readonly DrillPart[], unattributed?: DrillUnattributed): { key: string; item: DrillPart }[] {
  const ordered = orderedDrillParts(parts).map(item => ({ key: JSON.stringify(['part', item.id]), item }));
  if (unattributed && (unattributed.raw === undefined ? unattributed.value !== 0 : unattributed.raw !== 0n)) ordered.push({ key: 'unattributed',
    item: { id: '', label: unattributed.label, value: unattributed.value, raw: unattributed.raw, text: unattributed.text,
      shareText: unattributed.shareText, color: unattributed.color, muted: true } });
  return ordered;
}

export function drillPartShare(value: number | null, total: number | null, signed = false): string {
  if (value == null || total == null || total === 0) return '—';
  const share = Math.round((signed ? value / total : Math.abs(value) / Math.abs(total)) * 1000) / 10;
  return `${share}%`;
}

/** An inferred total is unavailable when any of its parts is unavailable. */
export function drillPartsTotal(parts: readonly DrillPart[]): number | null {
  return parts.some(part => part.value == null) ? null : parts.reduce((sum, part) => sum + part.value!, 0);
}
