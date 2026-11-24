import type { AuditRow } from '../../draw/notifications';
import type { HourSpan } from '../../generated/dashboard';
import type { FamilyTooltipEntry, FamilyTooltipPayload } from '../host/tooltips.ts';
import type { ArtHandle, BoundedReadView, DisplayedHour } from '../plan.ts';

export type TimelineMode = 'lanes' | 'meters' | 'series' | 'sections';

export interface TimelineCountryChip {
  readonly tag: string;
  readonly name: string;
  readonly flag?: ArtHandle | null;
  readonly flagMissing?: boolean;
  readonly color?: string | null;
  readonly dimmed?: boolean;
}

export interface TimelineSpanMarker {
  readonly id: string;
  readonly hour: number;
  readonly kind: 'surrender' | 'diamond' | 'text';
  readonly label?: string;
  readonly color?: string;
  readonly icon?: ArtHandle | null;
}

export interface TimelineSpanModel {
  readonly id: string;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly color: string;
  readonly label?: string;
  readonly icon?: ArtHandle | null;
  readonly iconMissing?: boolean;
  readonly heldBefore?: boolean;
  readonly fadedStart?: boolean;
  readonly fadedEnd?: boolean;
  readonly ended?: boolean;
  readonly openAtPlayhead?: boolean;
  readonly hatched?: boolean;
  readonly bottomColor?: string;
  readonly markers?: readonly TimelineSpanMarker[];
  readonly endText?: string;
  readonly meta?: unknown;
}

export interface TimelineDivergingBin {
  readonly firstHour: number;
  readonly lastHour: number;
  readonly won: number;
  readonly lost: number;
}

export interface TimelineOutcomeModel {
  readonly status: 'running' | 'waiting' | 'succeeded' | 'finished' | 'ended';
  readonly label: string;
  readonly detail?: string;
}

export interface TimelineMatchupSide {
  readonly lead: TimelineCountryChip;
  readonly allies: readonly TimelineCountryChip[];
  readonly dimmed?: boolean;
}

export type TimelineLaneKind =
  | 'standard'
  | 'group-header'
  | 'section-header'
  | 'matchup'
  | 'participant'
  | 'diverging-bars'
  | 'markers';

export interface TimelineLaneModel {
  readonly key: string;
  readonly kind: TimelineLaneKind;
  readonly depth?: number;
  readonly label: string;
  readonly subtitle?: string;
  readonly subtitleTone?: 'muted' | 'accent' | 'color';
  readonly subtitleColor?: string;
  readonly icon?: ArtHandle | null;
  readonly iconMissing?: boolean;
  readonly iconSize?: number;
  readonly flag?: ArtHandle | null;
  readonly flagTag?: string;
  readonly flagDimmed?: boolean;
  readonly colorSwatch?: string;
  readonly expandable?: boolean;
  readonly defaultOpen?: boolean;
  readonly parentKey?: string;
  readonly dimmed?: boolean;
  readonly height?: number;
  readonly spans?: readonly TimelineSpanModel[];
  readonly ruleColor?: string;
  readonly ruleSpan?: readonly [number, number];
  readonly matchup?: {
    readonly title?: string | null;
    readonly attackers: TimelineMatchupSide;
    readonly defenders: TimelineMatchupSide;
    readonly outcome: TimelineOutcomeModel;
  };
  readonly diverging?: {
    readonly bins: readonly TimelineDivergingBin[];
    readonly maxCount: number;
  };
  readonly pointMarkers?: readonly TimelineSpanMarker[];
  readonly auditValue?: string;
  readonly meta?: unknown;
}

export interface TimelineHeaderStat {
  readonly label: string;
  readonly value: string;
  readonly tone?: 'default' | 'positive' | 'negative' | 'muted';
}

export interface TimelineHeaderCard {
  readonly portrait?: ArtHandle | null;
  readonly portraitMissing?: boolean;
  readonly name: string;
  readonly subtitle?: string;
  readonly flag?: ArtHandle | null;
  readonly flagTag?: string;
  readonly stats: readonly TimelineHeaderStat[];
}

export interface TimelineMeterRow {
  readonly key: string;
  readonly label: string;
  readonly subtitle?: string;
  readonly subtitleColor?: string;
  readonly flag?: ArtHandle | null;
  readonly flagTag?: string;
  readonly icon?: ArtHandle | null;
  readonly iconMissing?: boolean;
  readonly fraction: number | null;
  readonly fill: string;
  readonly splitSegments?: readonly { readonly fraction: number; readonly fill: string }[];
  readonly valueText: string;
  readonly missing?: boolean;
  readonly thresholdTicks?: boolean;
  readonly sparkline?: readonly (number | null)[];
  readonly sectionHeader?: string;
  readonly plainRows?: readonly { readonly key: string; readonly label: string; readonly detail?: string; readonly icon?: ArtHandle | null; readonly iconMissing?: boolean }[];
  readonly auditValue?: string;
  readonly meta?: unknown;
}

export interface TimelineSeriesLine {
  readonly key: string;
  readonly label: string;
  readonly color: string;
  readonly points: readonly (readonly [number, number | null])[];
  readonly endLabel?: string;
  readonly flag?: ArtHandle | null;
  readonly flagTag?: string;
  readonly dashed?: boolean;
  readonly auditValue?: string;
  readonly meta?: unknown;
}

export interface TimelineSeriesModel {
  readonly lines: readonly TimelineSeriesLine[];
  readonly yMin: number;
  readonly yMax: number;
  readonly yFormat: 'percent' | 'integer';
  readonly thresholdLines?: readonly number[];
  readonly markers?: readonly TimelineSpanMarker[];
  readonly legend?: boolean;
}

export interface TimelineSectionItem {
  readonly key: string;
  readonly label: string;
  readonly missingName?: boolean;
  readonly rightText: string;
  readonly muted?: boolean;
  readonly meta?: unknown;
}

export interface TimelineSectionGroup {
  readonly key: string;
  readonly title: string;
  readonly countText?: string;
  readonly column: 0 | 1 | 'full';
  readonly items: readonly TimelineSectionItem[];
}

export interface TimelineModel {
  readonly mode: TimelineMode;
  readonly hour: DisplayedHour;
  readonly extent: readonly [number, number];
  readonly playhead: number | null;
  readonly headerSummary?: string | null;
  readonly headerCard?: TimelineHeaderCard | null;
  readonly lanes?: readonly TimelineLaneModel[];
  readonly meters?: readonly TimelineMeterRow[];
  readonly bottomMetersTitle?: string;
  readonly bottomMeters?: readonly TimelineMeterRow[];
  readonly series?: TimelineSeriesModel | null;
  readonly sections?: readonly TimelineSectionGroup[];
  readonly unobserved?: readonly HourSpan[];
  readonly iconsOnBars?: boolean;
  readonly labelWidth?: number;
  readonly outcomeWidth?: number;
  readonly empty?: string | null;
  readonly emptyDetail?: string | null;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly warnings?: readonly string[];
  readonly notes?: readonly string[];
  readonly nextChangeAt?: number;
}

export interface TimelineHitTarget {
  readonly kind: 'lane' | 'span' | 'meter' | 'series' | 'section-item' | 'marker' | 'toggle';
  readonly key: string;
  readonly laneKey?: string;
  readonly spanId?: string;
  readonly hour?: number;
  readonly meta?: unknown;
}

export interface TimelineProjection<TOptions = Record<string, never>> {
  readonly family: 'timeline';
  readonly kind: string;
  project(readView: BoundedReadView, options: TOptions, expanded?: Readonly<Record<string, boolean>>): TimelineModel;
  nextChange?(readView: BoundedReadView, options: TOptions): number;
  tooltip?(
    model: TimelineModel,
    target: TimelineHitTarget,
    hour: number,
    readView: BoundedReadView,
    options: TOptions,
  ): FamilyTooltipEntry | FamilyTooltipPayload | null;
}

export function visibleLanesOf(
  lanes: readonly TimelineLaneModel[],
  expanded: Readonly<Record<string, boolean>>,
): TimelineLaneModel[] {
  const openParents = new Map<string, boolean>();
  for (const lane of lanes) {
    if (lane.expandable) {
      const explicit = expanded[lane.key];
      openParents.set(lane.key, explicit !== undefined ? explicit : lane.defaultOpen === true);
    }
  }
  const out: TimelineLaneModel[] = [];
  for (const lane of lanes) {
    if (lane.parentKey && !openParents.get(lane.parentKey)) continue;
    out.push(lane);
  }
  return out;
}

export function isLaneExpanded(
  lane: TimelineLaneModel,
  expanded: Readonly<Record<string, boolean>>,
): boolean {
  if (!lane.expandable) return false;
  const explicit = expanded[lane.key];
  return explicit !== undefined ? explicit : lane.defaultOpen === true;
}

export function deriveTimelineAudit(
  model: TimelineModel,
  expanded: Readonly<Record<string, boolean>>,
): AuditRow[] {
  const rows: AuditRow[] = [];
  const push = (key: string, value: string) => {
    rows.push({ row: key, key, value });
  };
  if (model.headerSummary) {
    push('__summary__', model.headerSummary);
  }
  if (model.headerCard) {
    push(
      '__header__',
      `${model.headerCard.name}|${model.headerCard.stats.map(s => `${s.label}:${s.value}`).join('|')}`,
    );
  }
  if (model.mode === 'lanes' && model.lanes) {
    for (const lane of visibleLanesOf(model.lanes, expanded)) {
      if (lane.kind === 'section-header') continue;
      const val = lane.auditValue
        ?? (lane.matchup ? `${lane.label}|${lane.matchup.outcome.label}` : lane.subtitle ? `${lane.label}|${lane.subtitle}` : lane.label);
      push(lane.key, val);
    }
  } else if (model.mode === 'meters' && model.meters) {
    for (const meter of model.meters) {
      push(meter.key, meter.auditValue ?? `${meter.label}|${meter.valueText}`);
    }
  } else if (model.mode === 'series' && model.series) {
    for (const line of model.series.lines) {
      push(line.key, line.auditValue ?? `${line.label}|${line.endLabel ?? ''}`);
    }
  } else if (model.mode === 'sections' && model.sections) {
    for (const section of model.sections) {
      for (const item of section.items) {
        push(item.key, `${section.title}|${item.label}|${item.rightText}`);
      }
    }
  }
  return rows;
}

export function defaultTimelineTooltip(
  model: TimelineModel,
  target: TimelineHitTarget,
  hour: number,
): FamilyTooltipPayload | null {
  if (model.mode === 'lanes' && model.lanes) {
    const lane = model.lanes.find(l => l.key === (target.laneKey ?? target.key));
    if (!lane) return null;
    const span = target.spanId ? lane.spans?.find(s => s.id === target.spanId) : undefined;
    if (span) {
      return {
        title: span.label ?? lane.label,
        subtitle: lane.subtitle,
        hour: target.hour ?? hour,
        items: [
          { label: 'Lane', value: lane.label },
          { label: 'Status', value: span.openAtPlayhead ? 'Ongoing' : 'Ended' },
        ],
      };
    }
    return {
      title: lane.label,
      subtitle: lane.subtitle,
      hour: target.hour ?? hour,
    };
  }
  if (model.mode === 'meters' && model.meters) {
    const meter = model.meters.find(m => m.key === target.key);
    if (!meter) return null;
    return {
      title: meter.label,
      subtitle: meter.subtitle,
      hour,
      items: [{ label: 'Value', value: meter.valueText }],
    };
  }
  if (model.mode === 'sections' && model.sections) {
    for (const section of model.sections) {
      const item = section.items.find(i => i.key === target.key);
      if (item) {
        return {
          title: item.missingName ? 'Name not recorded' : item.label,
          subtitle: section.title,
          hour,
          items: [{ label: 'Recorded', value: item.rightText }],
        };
      }
    }
  }
  return null;
}

export function collectTimelineArtHandles(
  model: TimelineModel,
  expanded: Readonly<Record<string, boolean>>,
): ArtHandle[] {
  const seen = new Set<string>();
  const handles: ArtHandle[] = [];
  const push = (handle: ArtHandle | null | undefined) => {
    if (!handle) return;
    const key = handle.kind === 'flag'
      ? `flag|${handle.choice}|${handle.size}`
      : handle.kind === 'sprite'
        ? `sprite|${handle.key}|${handle.frame ?? 0}`
        : handle.kind === 'portrait'
          ? `portrait|${handle.key}|${handle.owner ?? ''}`
          : handle.kind === 'path'
            ? `path|${handle.path}|${handle.frame ?? 0}`
            : `url|${handle.url}|${handle.frame ?? 0}`;
    if (seen.has(key)) return;
    seen.add(key);
    handles.push(handle);
  };
  if (model.headerCard) {
    push(model.headerCard.portrait);
    push(model.headerCard.flag);
  }
  if (model.lanes) {
    for (const lane of visibleLanesOf(model.lanes, expanded)) {
      push(lane.icon);
      push(lane.flag);
      if (lane.matchup) {
        push(lane.matchup.attackers.lead.flag);
        for (const ally of lane.matchup.attackers.allies) push(ally.flag);
        push(lane.matchup.defenders.lead.flag);
        for (const ally of lane.matchup.defenders.allies) push(ally.flag);
      }
      if (lane.spans) {
        for (const span of lane.spans) {
          if (model.iconsOnBars !== false) push(span.icon);
          if (span.markers) for (const m of span.markers) push(m.icon);
        }
      }
    }
  }
  if (model.meters) {
    for (const m of model.meters) {
      push(m.flag);
      push(m.icon);
      if (m.plainRows) for (const r of m.plainRows) push(r.icon);
    }
  }
  if (model.bottomMeters) {
    for (const m of model.bottomMeters) {
      push(m.flag);
      push(m.icon);
    }
  }
  if (model.series) {
    for (const line of model.series.lines) push(line.flag);
  }
  return handles;
}
