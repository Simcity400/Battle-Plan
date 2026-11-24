import { auditRow, type FamilyAuditRow } from '../auditRow.ts';
import type { LegendStripEntry } from '../../draw/kit.ts';
import type { HourSpan } from '../../generated/dashboard';
import type { FamilyTooltipPayload } from '../host/tooltips.ts';
import type { ArtHandle, BoundedReadView, DisplayedHour, FamilyProjection } from '../contract.ts';

export interface TimelineSpan {
  readonly id: string;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly color: string;
  readonly label?: string;
  readonly icon?: ArtHandle | null;
  readonly iconMissing?: boolean;
  readonly heldBefore?: boolean;
  readonly startBy?: boolean;
  readonly endUnseen?: boolean;
  readonly open: boolean;
  readonly halves?: readonly [string, string];
  readonly marks?: readonly TimelineMark[];
  readonly meta?: unknown;
}

export interface TimelineMark {
  readonly hour: number;
  readonly art: ArtHandle | null;
  readonly label: string;
}

export interface TimelineSideLabel {
  readonly name: string;
  readonly flag: ArtHandle | null;
  readonly flagTag: string;
  readonly strip: readonly { readonly flag: ArtHandle | null; readonly tag: string }[];
  readonly dim?: boolean;
}

export interface TimelineOutcome {
  readonly mark: 'ongoing' | 'ended' | 'flag';
  readonly text: string;
  readonly detail?: string;
  readonly flag?: ArtHandle | null;
}

export type TimelineLaneKind = 'item' | 'group' | 'heading';

export interface TimelineLane {
  readonly key: string;
  readonly kind: TimelineLaneKind;
  readonly depth?: number;
  readonly label: string;
  readonly subtitle?: string;
  readonly aside?: string;
  readonly icon?: ArtHandle | null;
  readonly iconMissing?: boolean;
  readonly flag?: ArtHandle | null;
  readonly flagTag?: string;
  readonly swatch?: string;
  readonly expandable?: boolean;
  readonly defaultOpen?: boolean;
  readonly parentKey?: string;
  readonly spans: readonly TimelineSpan[];
  readonly openedMarker?: { readonly hour: number; readonly label: string };
  readonly ruleColor?: string;
  readonly endText?: string;
  readonly sides?: readonly [TimelineSideLabel, TimelineSideLabel];
  readonly outcome?: TimelineOutcome;
  readonly barHeight?: number;
  readonly height?: number;
  readonly auditValue: string;
  readonly meta?: unknown;
}

export interface TimelineModel {
  readonly hour: DisplayedHour;
  readonly firstHour: number;
  readonly lanes: readonly TimelineLane[];
  readonly unobserved: readonly HourSpan[];
  readonly legend?: readonly LegendStripEntry[];
  readonly iconsOnBars: boolean;
  readonly labelWidth: number;
  readonly outcomeWidth?: number;
  readonly summaryText?: string | null;
  readonly empty?: string | null;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly warnings: readonly string[];
  readonly notes?: readonly string[];
  readonly nextChangeAt: number;
}

export interface TimelineHit {
  readonly kind: 'lane' | 'span' | 'toggle';
  readonly laneKey: string;
  readonly spanId?: string;
  readonly hour?: number;
}

export interface TimelineProjection<TOptions = unknown> extends FamilyProjection<TimelineModel, TOptions> {
  readonly family: 'timeline';
  tooltip?(model: TimelineModel, hit: TimelineHit, readView: BoundedReadView, options: TOptions): FamilyTooltipPayload | null;
}

export function laneOpen(lane: TimelineLane, expanded: Readonly<Record<string, boolean>>): boolean {
  if (!lane.expandable) return false;
  return expanded[lane.key] ?? lane.defaultOpen === true;
}

export function visibleLanes(
  lanes: readonly TimelineLane[],
  expanded: Readonly<Record<string, boolean>>,
): TimelineLane[] {
  const closed = new Set<string>();
  for (const lane of lanes) {
    if (lane.expandable && !laneOpen(lane, expanded)) closed.add(lane.key);
  }
  if (!closed.size) return lanes.slice();
  return lanes.filter(lane => !lane.parentKey || !closed.has(lane.parentKey));
}

export function deriveTimelineAudit(model: TimelineModel): FamilyAuditRow[] {
  const rows: FamilyAuditRow[] = [];
  if (model.summaryText) rows.push(auditRow('__summary__', model.summaryText));
  for (const lane of model.lanes) {
    const spans = lane.spans.map(spanAudit).join(',');
    rows.push(auditRow(lane.key, `${lane.auditValue}|${lane.flagTag ?? ''}|${spans}`));
  }
  return rows;
}

function spanAudit(span: TimelineSpan): string {
  const start = `${span.heldBefore ? '<' : span.startBy ? '~' : ''}${span.firstHour}`;
  const end = span.open ? 'open' : `${span.lastHour}${span.endUnseen ? '~' : ''}`;
  return `${span.id}:${start}-${end}:${span.label ?? ''}`;
}

export function spanAt(lane: TimelineLane, hit: TimelineHit): TimelineSpan | undefined {
  return hit.spanId ? lane.spans.find(s => s.id === hit.spanId) : undefined;
}

export function laneByKey(model: TimelineModel, key: string): TimelineLane | undefined {
  return model.lanes.find(l => l.key === key);
}

const SEP = '\u001f';

export function encodeTimelineHit(hit: TimelineHit | null): string | null {
  if (!hit) return null;
  return `${hit.kind}:${hit.laneKey}${SEP}${hit.spanId ?? ''}${SEP}${hit.hour ?? ''}`;
}

export function decodeTimelineHit(token: string | null | undefined): TimelineHit | null {
  if (!token) return null;
  const colon = token.indexOf(':');
  if (colon < 0) return null;
  const kind = token.slice(0, colon);
  if (kind !== 'lane' && kind !== 'span' && kind !== 'toggle') return null;
  const [laneKey, spanId, hourText] = token.slice(colon + 1).split(SEP);
  if (!laneKey) return null;
  const hour = hourText ? Number(hourText) : NaN;
  return {
    kind,
    laneKey,
    ...(spanId ? { spanId } : {}),
    ...(Number.isFinite(hour) ? { hour } : {}),
  };
}
