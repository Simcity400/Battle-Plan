import type { AuditRow } from '../../draw/notifications';
import type { FamilyTheme, MirrorModel, MirrorRow } from '../contract.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import { layoutBreakdown, type BreakdownLayout, type DrawnItem } from './layout.ts';
import { breakdownLevel, COMPARISON_KEY, type BreakdownModel, type BreakdownTotal } from './model.ts';

function spoken(item: DrawnItem, total: BreakdownTotal): string {
  const { part } = item;
  if (part.notRecorded || part.value === null) return `not recorded${part.missingReason ? `: ${part.missingReason}` : ''}`;
  const value = `${part.exact ?? part.text}${total.unit ? ` ${total.unit}` : ''}`;
  return `${value}${item.shareText ? `, ${item.shareText}` : ''}${part.partial ? ', partial' : ''}`;
}

export function deriveBreakdownNavAndMirror(
  layout: BreakdownLayout,
  total: BreakdownTotal,
): { readonly nav: readonly FamilyNavTuple[]; readonly mirror: MirrorModel } {
  const names = new Map<string, string>();
  const nav: FamilyNavTuple[] = [];
  const rows: MirrorRow[] = [];
  layout.items.forEach((item, index) => {
    const parentName = item.parentKey === null ? null : names.get(item.parentKey) ?? null;
    const name = parentName ? `${parentName}, ${item.part.name}` : item.part.name;
    names.set(item.key, name);
    nav.push([item.key, item.part.name, index, item.open, item.parentKey ?? '', null]);
    rows.push({
      key: item.key,
      name,
      rowIndex: index,
      level: item.depth,
      ...(item.open >= 0 ? { group: true, open: item.open === 1 } : {}),
      cells: [{ column: 'value', spoken: spoken(item, total) }],
    });
  });
  return { nav, mirror: { first: 0, total: rows.length, rows } };
}

function stated(total: BreakdownTotal): string {
  if (total.value === null) return `notRecorded:${total.reason ?? 'Not recorded'}`;
  const text = total.exact ?? total.text;
  return total.atLeast ? `atLeast:${text}` : total.partial ? `partial:${text}` : text;
}

function drawn(item: DrawnItem): string {
  const { part } = item;
  if (part.notRecorded || part.value === null) return `notRecorded:${part.missingReason ?? 'Not recorded'}`;
  const text = part.exact ?? part.text;
  return `${part.atLeast ? 'atLeast:' : part.partial ? 'partial:' : ''}${text}${item.shareText ? `|${item.shareText}` : ''}`;
}

export function deriveBreakdownAudit(model: BreakdownModel, layout: BreakdownLayout | null): readonly AuditRow[] {
  const rows: AuditRow[] = [{ row: '__total__', key: '__total__', value: stated(model.total) }];
  if (model.periodLabel) rows.push({ row: '__period__', key: '__period__', value: model.periodLabel });
  if (model.comparison) {
    const { label, value, note } = model.comparison;
    rows.push({ row: COMPARISON_KEY, key: COMPARISON_KEY, value: `${label}:${value}${note ? `:${note}` : ''}` });
  }
  if (model.empty) rows.push({ row: '__empty__', key: '__empty__', value: `${model.empty}${model.emptyDetail ? `:${model.emptyDetail}` : ''}` });
  for (const item of layout?.items ?? []) rows.push({ row: item.key, key: item.key, value: drawn(item) });
  return rows;
}

const STEADY_MEASURE = (text: string) => text.length * 7;

export function auditLayout(
  model: BreakdownModel,
  width: number,
  height: number,
  expanded: Readonly<Record<string, boolean>>,
  theme: FamilyTheme | undefined,
): BreakdownLayout | null {
  if (!theme || width <= 0 || height <= 0) return null;
  return layoutBreakdown({
    view: model.view ?? 'parts',
    level: breakdownLevel(model),
    ...(model.periodLabel ? { periodLabel: model.periodLabel } : {}),
    ...(model.comparison ? { comparison: model.comparison } : {}),
    width,
    height,
    theme,
    namesInRing: false,
    userExpanded: expanded,
    measure: STEADY_MEASURE,
  });
}
