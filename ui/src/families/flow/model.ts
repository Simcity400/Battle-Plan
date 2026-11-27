import { auditCell, auditRow, type FamilyAuditRow } from '../auditRow.ts';
import { barsLabelAudit } from '../bars/model.ts';
import type { Value } from '../value.ts';
import type { ArtResult, BoundedReadView, MirrorModel, MirrorRow } from '../contract.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import type { FamilyTooltipPayload } from '../host/tooltips.ts';
import type { FlowLayout } from './painter.ts';

export type FlowMode = 'centred' | 'bipartite';
export type FlowSide = 'left' | 'centre' | 'right';

export interface FlowPartKey {
  readonly key: string;
  readonly label: Value<string>;
  readonly ordinal: number;
}

export interface FlowNode {
  readonly key: string;
  readonly side: FlowSide;
  readonly label: Value<string>;
  readonly tag?: string;
  readonly flag?: ArtResult;
  readonly numeric: number | null;
  readonly figure: string;
  readonly others?: number;
  readonly unknown?: string;
  readonly spoken: string;
  readonly audit: string;
  readonly tooltip?: FamilyTooltipPayload;
  readonly subject?: string;
}

export interface FlowBand {
  readonly part: number;
  readonly numeric: number;
}

export interface FlowRibbon {
  readonly key: string;
  readonly from: string;
  readonly to: string;
  readonly bands: readonly FlowBand[];
  readonly missing?: boolean;
  readonly others?: boolean;
  readonly numeric: number | null;
  readonly audit: string;
  readonly tooltip?: FamilyTooltipPayload;
}

export interface FlowModel {
  readonly hour: number;
  readonly mode: FlowMode;
  readonly left: readonly FlowNode[];
  readonly right: readonly FlowNode[];
  readonly centre?: FlowNode;
  readonly ribbons: readonly FlowRibbon[];
  readonly parts: readonly FlowPartKey[];
  readonly leftHeader?: string;
  readonly rightHeader?: string;
  readonly unitNote?: string;
  readonly notRecordedKey?: boolean;
  readonly summaryText?: string | null;
  readonly notes?: readonly string[];
  readonly warnings?: readonly string[];
  readonly empty?: string | null;
  readonly emptyDetail?: string | null;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly refused?: string;
}

export interface FlowProjection<TOptions = unknown> {
  readonly family: 'flow';
  readonly kind: string;
  project(readView: BoundedReadView, options: TOptions): FlowModel;
  nextChange?(readView: BoundedReadView, options: TOptions): number;
}

export function flowNodes(model: FlowModel): readonly FlowNode[] {
  return model.centre ? [...model.left, model.centre, ...model.right] : [...model.left, ...model.right];
}

export function flowRibbonMarked(ribbon: FlowRibbon): boolean {
  return ribbon.missing === true || ribbon.bands.some(band => band.numeric > 0);
}

export function deriveFlowNav(model: FlowModel, layout?: FlowLayout): readonly FamilyNavTuple[] {
  const nodes = flowNodes(model);
  const nodeBox = new Map(layout?.nodes.map(box => [box.node.key, box]));
  const ribbonBox = new Map(layout?.ribbons.map(box => [box.ribbon.key, box]));
  const stop = (key: string, name: string, index: number, meta: unknown, place: readonly [number, number] | null): FamilyNavTuple =>
    place ? [key, name, index, -1, '', meta, place[0], place[1]] : [key, name, index, -1, '', meta];
  return [
    ...nodes.map((node, index) => {
      const box = nodeBox.get(node.key);
      return stop(`node:${node.key}`, node.spoken, index, node.subject ?? null, box ? [box.slotTop, box.slotHeight] : null);
    }),
    ...model.ribbons.filter(flowRibbonMarked).map((ribbon, index) => {
      const box = ribbonBox.get(ribbon.key);
      const top = box ? Math.min(box.a0, box.b0) : 0;
      return stop(`ribbon:${ribbon.key}`, `${ribbon.tooltip?.title ?? ribbon.key}: ${ribbon.audit}`, nodes.length + index, null, box ? [top, Math.max(box.a1, box.b1) - top] : null);
    }),
  ];
}

export function deriveFlowMirror(model: FlowModel): MirrorModel {
  const rows: MirrorRow[] = [];
  for (const node of flowNodes(model)) rows.push({ key: `node:${node.key}`, name: node.spoken, rowIndex: rows.length, cells: [] });
  for (const ribbon of model.ribbons.filter(flowRibbonMarked)) rows.push({ key: `ribbon:${ribbon.key}`, name: ribbon.tooltip?.title ?? ribbon.key, rowIndex: rows.length, cells: [{ column: 'Amount', spoken: ribbon.audit }] });
  return { first: 0, total: rows.length, rows };
}

export function deriveFlowAudit(model: FlowModel): readonly FamilyAuditRow[] {
  const rows: FamilyAuditRow[] = [];
  if (model.refused !== undefined) rows.push(auditRow('__refused__', model.refused));
  if (model.empty) rows.push(auditRow('__empty__', `${model.emptyKind ?? 'empty'}: ${model.empty}${model.emptyDetail ? `: ${model.emptyDetail}` : ''}`));
  for (const node of flowNodes(model)) rows.push(auditRow(`node:${node.key}`, node.audit), auditCell(`node:${node.key}`, 'label', barsLabelAudit(node.label)));
  for (const part of model.parts) rows.push(auditCell(`part:${part.key}`, 'label', barsLabelAudit(part.label)));
  for (const ribbon of model.ribbons) rows.push(auditRow(`ribbon:${ribbon.key}`, ribbon.audit));
  return rows;
}

export function findFlowTooltip(model: FlowModel, token: string): FamilyTooltipPayload | null {
  if (token.startsWith('node:')) {
    const key = token.slice(5);
    return flowNodes(model).find(node => node.key === key)?.tooltip ?? null;
  }
  if (token.startsWith('ribbon:')) {
    const key = token.slice(7);
    return model.ribbons.find(ribbon => ribbon.key === key)?.tooltip ?? null;
  }
  return null;
}
