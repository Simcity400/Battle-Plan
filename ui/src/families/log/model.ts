import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { MissingSpan } from '../../generated/dashboard';
import { canonicalAuditValue } from '../audit.ts';
import type { MirrorModel, MirrorRow } from '../contract.ts';
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
  readonly dateLine?: string;
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
  readonly tooltip?: LogTooltipSpec | (() => LogTooltipSpec);
  readonly auditCells: Readonly<Record<string, Value>>;
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

export function canonicalLogRowAudit(row: LogRowSpec): string {
  const keys = Object.keys(row.auditCells).sort();
  const pairs: [string, string][] = new Array(keys.length);
  for (let i = 0; i < keys.length; i++) {
    const key = keys[i]!;
    pairs[i] = [key, canonicalAuditValue(row.auditCells[key]!)];
  }
  return JSON.stringify([row.hour, row.by === true ? 1 : 0, pairs]);
}

export function deriveLogAudit(model: LogModel, maxRows?: number): { readonly ordered: true; readonly values: Map<string, string> } {
  const values = new Map<string, string>();
  const limit = maxRows === undefined ? model.rows.length : Math.min(model.rows.length, maxRows);
  for (let i = 0; i < limit; i++) {
    const row = model.rows[i]!;
    values.set(row.key, canonicalLogRowAudit(row));
  }
  return { ordered: true, values };
}

export function deriveLogMirror(items: readonly FlatLogItem[], first: number, count: number, totalRows: number): MirrorModel {
  const rows: MirrorRow[] = [];
  let rowOrdinal = 0;
  for (let i = 0; i < items.length; i++) {
    const item = items[i]!;
    if (item.kind === 'month') continue;
    if (rowOrdinal >= first && rows.length < count) {
      if (item.kind === 'gap') {
        rows.push({
          key: item.key,
          rowIndex: rowOrdinal,
          name: item.label,
          cells: [{ column: 'reason', spoken: item.span.reason }],
        });
      } else {
        rows.push({
          key: item.row.key,
          rowIndex: rowOrdinal,
          name: item.row.spoken,
          cells: Object.entries(item.row.auditCells).map(([column, value]) => ({
            column,
            spoken: spokenValue(value),
          })),
        });
      }
    }
    rowOrdinal++;
    if (rows.length >= count) break;
  }
  return { first, total: Math.max(totalRows, rowOrdinal), rows };
}

export function deriveLogTooltip(row: LogRowSpec): TooltipContent | null {
  const raw = typeof row.tooltip === 'function' ? row.tooltip() : row.tooltip;
  if (!raw) return null;
  const datePrefix = row.by ? 'By ' : '';
  const subtitle = raw.dateLine ?? `${datePrefix}${formatDateTime(row.hour)}`;
  const rows: TooltipRow[] = raw.rows.map(field => {
    const formatted = formatValueState(field.value, field.format);
    const text = formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable' ? formatted.label : formatted.text;
    const detail = field.value.kind === 'notRecorded' || field.value.kind === 'partial' || field.value.kind === 'notApplicable'
      ? field.value.reason
      : undefined;
    return {
      label: field.label,
      value: detail && formatted.kind === 'notRecorded' ? `${text} — ${detail}` : text,
      tone: field.tone ?? (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable' ? 'muted' : undefined),
      strong: field.strong,
      divider: field.divider,
      indent: field.indent,
    };
  });
  return {
    title: raw.title,
    icon: raw.icon,
    subtitle,
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
