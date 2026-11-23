import { liveKey } from '../../charts/live.ts';
import { panelViews, recordedPanelRows } from '../../draw/panels/views.ts';
import { repliedCells, sectionOf } from './liveCells.ts';

export function exactLive(raw: unknown): Map<string, number | string> {
  const out = new Map<string, number | string>(), section = sectionOf(raw);
  if (!section) return out;
  const values = repliedCells(raw).read();
  for (let at = 0; at < section.cells.length; at += 3 + section.cells[at + 2]! * 3) {
    const key = liveKey(section.rows[section.cells[at]!]!, section.kinds[section.cells[at + 1]!]!.id);
    out.set(key, values.get(key) ?? values.missing(key));
  }
  return out;
}

export function panelAuditRows(product: string, raw: unknown, live: ReadonlyMap<string, unknown> | null | undefined) {
  return [...recordedPanelRows(product, raw).map(row => ({ row, key: row, value: row })),
    ...[...live ?? []].map(([key, value]) => ({ row: `live:${key}`, key: `live:${key}`, value: String(value) }))];
}

export function panelTexts(product: string, raw: unknown, live: ReadonlyMap<string, number | string>, hour: number): ReadonlyMap<string, string> | null {
  const view = panelViews[product];
  if (!view) return null;
  const values = { get: (key: string) => { const value = live.get(key); return typeof value === 'number' ? value : undefined; },
    missing: (key: string) => { const value = live.get(key); return typeof value === 'string' ? value : 'Not recorded'; } };
  return view.values(raw, values, hour, view.structure?.(raw, hour) ?? '') as ReadonlyMap<string, string> | null;
}
