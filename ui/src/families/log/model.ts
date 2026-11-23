import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { AuditRow } from '../../draw/notifications';
import type { MissingSpan } from '../../generated/dashboard';
import { canonicalAuditData } from '../audit.ts';
import type { ListboxMirrorItem } from '../host/mirror.ts';
import type { Value } from '../value.ts';
import { formatValueState, spokenValue, type ValueFormat } from '../valueFormat.ts';
import type { LogCountryLook } from './declaration.ts';

export type LogDensity = 'comfortable' | 'compact';
export type LogLayoutKind = 'row' | 'card';
export type LogTone = 'default' | 'muted' | 'accent' | 'positive' | 'negative' | 'warning';

export interface LogTextPart {
  readonly text: Value<string>;
  readonly flag?: LogCountryLook | null;
  readonly weight?: 'regular' | 'bold';
  readonly tone?: LogTone;
}

export type LogBand =
  | { readonly kind: 'text'; readonly text: Value<string>; readonly weight?: 'regular' | 'bold'; readonly tone?: LogTone; readonly maxLines?: number }
  | { readonly kind: 'rich'; readonly parts: readonly LogTextPart[]; readonly maxLines?: number }
  | { readonly kind: 'value'; readonly value: Value; readonly format?: ValueFormat; readonly icon?: string; readonly align?: 'start' | 'end'; readonly tone?: LogTone }
  | { readonly kind: 'pill'; readonly label: Value<string>; readonly tone?: LogTone; readonly liveDot?: boolean }
  | { readonly kind: 'flags'; readonly looks: readonly (LogCountryLook | null)[]; readonly max?: number; readonly unknownReason?: string }
  | { readonly kind: 'meter'; readonly fraction: Value<number>; readonly label: Value<string> }
  | { readonly kind: 'bar-pair'; readonly left: Value<number>; readonly right: Value<number>; readonly leftTone: string; readonly rightTone: string; readonly format?: ValueFormat }
  | { readonly kind: 'chips'; readonly items: readonly { readonly label: Value<string>; readonly flag?: LogCountryLook | null; readonly tone?: LogTone }[] };

export interface LogColumnTrack {
  readonly id: string;
  readonly width?: number;
  readonly minWidth?: number;
  readonly flex?: number;
  readonly align?: 'start' | 'end';
  readonly priority?: number;
}

export interface LogArtSpec {
  readonly icon?: string;
  readonly glyph?: string;
  readonly color?: string;
  readonly missingReason?: string;
}

export interface LogTooltipField {
  readonly label: string;
  readonly value: Value;
  readonly format?: ValueFormat;
  readonly tone?: 'positive' | 'negative' | 'neutral' | 'muted';
  readonly strong?: boolean;
  readonly divider?: boolean;
  readonly indent?: boolean;
}

export interface LogTooltipSpec {
  readonly title: string;
  readonly icon?: string;
  readonly subtitle?: string;
  readonly rows: readonly LogTooltipField[];
  readonly note?: string;
  readonly description?: string;
  readonly hint?: string;
}

export interface LogRowSpec {
  readonly key: string;
  readonly id?: string;
  readonly hour: number;
  readonly by?: boolean;
  readonly ordinal?: number;
  readonly art?: LogArtSpec;
  readonly slots: readonly LogBand[];
  readonly subline?: LogBand;
  readonly cardBands?: readonly LogBand[];
  readonly rawMagnitudes?: Readonly<Record<string, Value>>;
  readonly tooltip?: LogTooltipSpec | (() => LogTooltipSpec);
  readonly spoken: string;
  readonly meta?: unknown;
}

export interface LogFacetPill {
  readonly key: string;
  readonly label: string;
  readonly count: Value<number>;
  readonly tone?: LogTone;
}

export interface LogEmptyState {
  readonly kind: 'empty' | 'unavailable';
  readonly message: string;
  readonly detail?: string;
}

export interface LogModel {
  readonly hour: number;
  readonly fromHour?: number;
  readonly total: number;
  readonly baseOrdinal: number;
  readonly firstOrdinal: number;
  readonly offset: number;
  readonly layout: LogLayoutKind;
  readonly twoLine?: boolean;
  readonly columns: readonly LogColumnTrack[];
  readonly rows: readonly LogRowSpec[];
  readonly gaps: readonly MissingSpan[];
  readonly facetPills?: readonly LogFacetPill[];
  readonly empty: LogEmptyState;
  readonly warnings?: readonly string[];
  readonly notes?: readonly string[];
}

export type FlatLogItem =
  | { readonly kind: 'month'; readonly key: string; readonly label: string; readonly top: number; readonly height: number }
  | { readonly kind: 'gap'; readonly key: string; readonly span: MissingSpan; readonly label: string; readonly top: number; readonly height: number }
  | { readonly kind: 'row'; readonly key: string; readonly row: LogRowSpec; readonly index: number; readonly top: number; readonly height: number };

function lookAuditNode(look: LogCountryLook | null | undefined) {
  if (!look) return null;
  return {
    name: look.name,
    flag: look.flag ?? null,
    color: look.color ?? null,
    status: look.status ?? null,
  };
}

function bandAuditNode(band: LogBand): Record<string, unknown> {
  switch (band.kind) {
    case 'text':
      return {
        kind: 'text',
        text: band.text,
        weight: band.weight ?? 'regular',
        tone: band.tone ?? 'default',
        maxLines: band.maxLines ?? 1,
      };
    case 'rich':
      return {
        kind: 'rich',
        parts: band.parts.map(part => ({
          text: part.text,
          flag: lookAuditNode(part.flag),
          weight: part.weight ?? 'regular',
          tone: part.tone ?? 'default',
        })),
        maxLines: band.maxLines ?? 1,
      };
    case 'value':
      return {
        kind: 'value',
        value: band.value,
        format: band.format ?? null,
        icon: band.icon ?? null,
        align: band.align ?? 'start',
        tone: band.tone ?? 'default',
      };
    case 'pill':
      return {
        kind: 'pill',
        label: band.label,
        tone: band.tone ?? 'default',
        liveDot: band.liveDot === true,
      };
    case 'flags':
      return {
        kind: 'flags',
        looks: band.looks.map(lookAuditNode),
        max: band.max ?? null,
        unknownReason: band.unknownReason ?? null,
      };
    case 'meter':
      return {
        kind: 'meter',
        fraction: band.fraction,
        label: band.label,
      };
    case 'bar-pair':
      return {
        kind: 'bar-pair',
        left: band.left,
        right: band.right,
        leftTone: band.leftTone,
        rightTone: band.rightTone,
        format: band.format ?? null,
      };
    case 'chips':
      return {
        kind: 'chips',
        items: band.items.map(item => ({
          label: item.label,
          flag: lookAuditNode(item.flag),
          tone: item.tone ?? 'default',
        })),
      };
  }
}

const auditRowCache = new WeakMap<LogRowSpec, string>();

export function canonicalLogRowAudit(row: LogRowSpec): string {
  const cached = auditRowCache.get(row);
  if (cached !== undefined) return cached;
  const encoded = canonicalAuditData({
    kind: 'row',
    hour: row.hour,
    by: row.by === true,
    ordinal: row.ordinal ?? null,
    art: row.art ? {
      icon: row.art.icon ?? null,
      glyph: row.art.glyph ?? null,
      color: row.art.color ?? null,
      missingReason: row.art.missingReason ?? null,
    } : null,
    slots: row.slots.map(bandAuditNode),
    subline: row.subline ? bandAuditNode(row.subline) : null,
    cardBands: row.cardBands ? row.cardBands.map(bandAuditNode) : null,
    rawMagnitudes: row.rawMagnitudes ?? null,
  });
  auditRowCache.set(row, encoded);
  return encoded;
}

export function canonicalLogItemAudit(item: FlatLogItem): string {
  if (item.kind === 'month') {
    return canonicalAuditData({ kind: 'month', label: item.label });
  }
  if (item.kind === 'gap') {
    return canonicalAuditData({
      kind: 'gap',
      firstHour: item.span.firstHour,
      lastHour: item.span.lastHour,
      label: item.label,
      reason: item.span.reason,
    });
  }
  return canonicalLogRowAudit(item.row);
}

export function canonicalLogSummaryAudit(model: LogModel, newRowCount = 0): string {
  return canonicalAuditData({
    kind: 'summary',
    total: model.total,
    baseOrdinal: model.baseOrdinal,
    firstOrdinal: model.firstOrdinal,
    offset: model.offset,
    newRowCount,
    empty: model.rows.length === 0 ? model.empty : null,
    facetPills: (model.facetPills ?? []).map(pill => ({
      key: pill.key,
      label: pill.label,
      count: pill.count,
      tone: pill.tone ?? null,
    })),
  });
}

export function deriveLogAuditRows(
  model: LogModel,
  items: readonly FlatLogItem[],
  start: number,
  end: number,
  scrollTop: number,
  stickyH: number,
  height: number,
  newRowCount = 0,
): AuditRow[] {
  const out: AuditRow[] = [
    { row: '__summary__', key: '__summary__', value: canonicalLogSummaryAudit(model, newRowCount) },
  ];
  if (model.rows.length === 0 && model.gaps.length === 0) return out;
  for (let i = start; i < end; i++) {
    const item = items[i]!;
    const y = Math.round(item.top - scrollTop);
    if (y + item.height <= stickyH || y >= height) continue;
    out.push({ row: item.key, key: item.key, value: canonicalLogItemAudit(item) });
  }
  return out;
}

export function deriveLogAudit(model: LogModel, maxRows?: number): { readonly ordered: true; readonly values: Map<string, string> } {
  const values = new Map<string, string>();
  values.set('__summary__', canonicalLogSummaryAudit(model, 0));
  const limit = maxRows === undefined ? model.rows.length : Math.min(model.rows.length, maxRows);
  for (let i = 0; i < limit; i++) {
    const row = model.rows[i]!;
    values.set(row.key, canonicalLogRowAudit(row));
  }
  return { ordered: true, values };
}

function bandSpokenReasons(band: LogBand, out: string[]): void {
  const push = (value: Value) => {
    if (value.kind === 'notRecorded' || value.kind === 'partial' || value.kind === 'notApplicable') {
      const text = spokenValue(value);
      if (text && !out.includes(text)) out.push(text);
    }
  };
  switch (band.kind) {
    case 'text':
      push(band.text);
      return;
    case 'rich':
      for (const part of band.parts) push(part.text);
      return;
    case 'value':
      push(band.value);
      return;
    case 'pill':
      push(band.label);
      return;
    case 'meter':
      push(band.fraction);
      push(band.label);
      return;
    case 'bar-pair':
      push(band.left);
      push(band.right);
      return;
    case 'chips':
      for (const item of band.items) push(item.label);
      return;
    case 'flags':
      if (band.unknownReason && !out.includes(band.unknownReason)) out.push(band.unknownReason);
      return;
  }
}

export function spokenLogRow(row: LogRowSpec): string {
  const reasons: string[] = [];
  for (const band of row.slots) bandSpokenReasons(band, reasons);
  if (row.subline) bandSpokenReasons(row.subline, reasons);
  if (row.cardBands) {
    for (const band of row.cardBands) bandSpokenReasons(band, reasons);
  }
  const missing = reasons.filter(reason => !row.spoken.includes(reason));
  return missing.length > 0 ? `${row.spoken} (${missing.join('; ')})` : row.spoken;
}

export function deriveLogMirrorItems(
  items: readonly FlatLogItem[],
  first: number,
  count: number,
  selectedKey: string | null = null,
): ListboxMirrorItem[] {
  const out: ListboxMirrorItem[] = [];
  let rowOrdinal = 0;
  for (let i = 0; i < items.length; i++) {
    const item = items[i]!;
    if (item.kind === 'month') continue;
    if (rowOrdinal >= first && out.length < count) {
      if (item.kind === 'gap') {
        out.push({
          key: item.key,
          role: 'option',
          index: rowOrdinal,
          spoken: `${item.label} — ${item.span.reason}`,
        });
      } else {
        out.push({
          key: item.row.key,
          role: 'option',
          index: item.index,
          selected: selectedKey !== null && selectedKey === item.row.key,
          spoken: spokenLogRow(item.row),
        });
      }
    }
    rowOrdinal++;
    if (out.length >= count) break;
  }
  return out;
}

export function deriveLogTooltip(row: LogRowSpec): TooltipContent | null {
  const raw = typeof row.tooltip === 'function' ? row.tooltip() : row.tooltip;
  if (!raw) return null;
  const rows: TooltipRow[] = raw.rows.map(field => {
    const formatted = formatValueState(field.value, field.format);
    const text = formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable' ? formatted.label : formatted.text;
    const detail = field.value.kind === 'notRecorded' || field.value.kind === 'partial' || field.value.kind === 'notApplicable'
      ? field.value.reason
      : undefined;
    return {
      label: field.label,
      value: detail && (formatted.kind === 'notRecorded' || formatted.kind === 'partial') ? `${text} — ${detail}` : text,
      tone: field.tone ?? (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable' ? 'muted' : undefined),
      strong: field.strong,
      divider: field.divider,
      indent: field.indent,
    };
  });
  return {
    title: raw.title,
    icon: raw.icon,
    ...(raw.subtitle ? { subtitle: raw.subtitle } : {}),
    rows,
    note: raw.note ?? raw.description ?? raw.hint,
  };
}

export function deriveGapTooltip(span: MissingSpan): TooltipContent {
  return {
    title: 'Not recorded',
    subtitle: `${formatDateTime(span.firstHour)} – ${formatDateTime(span.lastHour)}`,
    rows: [{ label: 'Reason', value: span.reason, tone: 'muted' }],
  };
}
