import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import { collectArtUrls } from '../art/provider.ts';
import type { ArtHandle, BoundedReadView, FamilyTheme, MirrorModel } from '../contract.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import type { FamilyTooltipPayload } from '../host/tooltips.ts';
import type { IdentityIndex } from '../identity/index.ts';
import type { LaneBinding, LaneFamily } from '../laneRuntime.ts';
import { AXIS_HEIGHT, frameTimeline, placeTimeline, visibleRange, type TimelineLayout, type TimelinePlacement } from './layout.ts';
import { resolutionFor, type TimelineResolution } from './resolution.ts';
import { decodeTimelineHit, deriveTimelineAudit, encodeTimelineHit, laneByKey, spanAt, type TimelineHit, type TimelineLane, type TimelineModel, type TimelineProjection } from './model.ts';
import { drawTimeline, hitTimeline } from './painter.ts';

export interface FamilyTimelineBinding<TOptions = unknown> extends LaneBinding<TOptions> {
  readonly family: 'timeline';
  readonly width: number;
  readonly height: number;
  readonly theme: FamilyTheme;
  readonly selectedKey: string | null;
  readonly focusedKey: string | null;
  readonly expanded: Readonly<Record<string, boolean>>;
  readonly pauseFlush: number;
}

export interface TimelineExtentField {
  readonly height: number;
}

interface TimelineFrameLayout {
  readonly placement: TimelinePlacement;
  readonly frame: TimelineLayout;
  readonly resolution: TimelineResolution;
}

export function nextDayStart(hour: number): number {
  return hour + 24 - ((((Math.floor(hour) - HOUR_EPOCH) % 24) + 24) % 24);
}
function navOf(layout: TimelinePlacement): FamilyNavTuple[] {
  const nav: FamilyNavTuple[] = [];
  for (const placed of layout.lanes) {
    const lane = placed.lane;
    if (lane.kind === 'heading') continue;
    const tree = lane.expandable ? (layout.lanes[placed.ordinal + 1]?.lane.parentKey === lane.key ? 1 : 0) : -1;
    nav.push([lane.key, lane.label, nav.length, tree, lane.parentKey ?? '', lane.meta ?? null, placed.y, placed.height]);
  }
  return nav;
}

function mirrorOf(layout: TimelinePlacement): MirrorModel {
  const rows = layout.lanes.filter(placed => placed.lane.kind !== 'heading').map((placed, index) => ({
    key: placed.lane.key,
    name: placed.lane.label,
    rowIndex: index,
    level: placed.lane.depth ?? 0,
    ...(placed.lane.expandable ? { group: true, open: layout.lanes[placed.ordinal + 1]?.lane.parentKey === placed.lane.key } : {}),
    cells: placed.lane.subtitle ? [{ column: 'status', spoken: placed.lane.subtitle }] : [],
  }));
  return { first: 0, total: rows.length, rows };
}

export function defaultTimelineTooltip(model: TimelineModel, lane: TimelineLane, hit: TimelineHit): FamilyTooltipPayload {
  const span = spanAt(lane, hit);
  return {
    title: span?.label ?? lane.label,
    ...(span?.label && span.label !== lane.label ? { subtitle: lane.label } : lane.subtitle ? { subtitle: lane.subtitle } : {}),
    hour: hit.hour ?? model.hour,
  };
}

function tooltipFor(
  projection: TimelineProjection<unknown>,
  model: TimelineModel,
  hit: TimelineHit,
  readView: BoundedReadView | null,
  options: unknown,
): FamilyTooltipPayload | null {
  const lane = laneByKey(model, hit.laneKey);
  if (!lane) return null;
  if (projection.tooltip && readView) return projection.tooltip(model, hit, readView, options);
  return defaultTimelineTooltip(model, lane, hit);
}

function collectArt(model: TimelineModel, layout: TimelinePlacement, scrollTop: number, index: IdentityIndex | null): string[] {
  const handles: (ArtHandle | null | undefined)[] = [];
  const [first, end] = visibleRange(layout, scrollTop);
  for (let i = first; i < end; i++) {
    const lane = layout.lanes[i]!.lane;
    handles.push(lane.flag, lane.icon, lane.outcome?.flag);
    for (const side of lane.sides ?? []) {
      handles.push(side.flag);
      for (const member of side.strip) handles.push(member.flag);
    }
    for (const span of lane.spans) {
      if (model.iconsOnBars) handles.push(span.icon);
      for (const mark of span.marks ?? []) handles.push(mark.art);
    }
  }
  return collectArtUrls(handles, index);
}

const lastEvents = new WeakMap<TimelineModel, number>();

function lastEvent(model: TimelineModel): number {
  let last = lastEvents.get(model);
  if (last !== undefined) return last;
  last = -Infinity;
  for (const lane of model.lanes) {
    for (const span of lane.spans) {
      if (span.firstHour > last) last = span.firstHour;
      if (!span.open && span.lastHour > last) last = span.lastHour;
    }
  }
  lastEvents.set(model, last);
  return last;
}

const placements = new WeakMap<TimelineModel, { readonly key: readonly unknown[]; readonly placement: TimelinePlacement }>();
const resolutions = new WeakMap<TimelinePlacement, TimelineResolution>();

function placementOf(model: TimelineModel, width: number, height: number, expanded: Readonly<Record<string, boolean>>): TimelinePlacement {
  const held = placements.get(model);
  if (held && held.key[0] === width && held.key[1] === height && held.key[2] === expanded) return held.placement;
  const placement = placeTimeline(model, width, height, expanded);
  placements.set(model, { key: [width, height, expanded], placement });
  return placement;
}

function focusHit(model: TimelineModel, key: string, hour: number): TimelineHit {
  const current = laneByKey(model, key)?.spans.find(span => span.open);
  return current ? { kind: 'span', laneKey: key, spanId: current.id, hour } : { kind: 'lane', laneKey: key, hour };
}

export const timelineLane: LaneFamily<FamilyTimelineBinding, TimelineModel, TimelineFrameLayout> = {
  family: 'timeline',
  boundary: model => model.nextChangeAt,
  redraw: (_model, hour) => nextDayStart(hour),
  status: () => 'scene',
  view: ['expanded'],
  layout(model, binding, surface, _theme, _measure, hour) {
    const placement = placementOf(model, surface.width, surface.height, binding.expanded);
    const frame = frameTimeline(placement, Math.max(nextDayStart(hour) - 24, lastEvent(model)));
    const resolution = resolutionFor(resolutions.get(placement) ?? null, model, frame);
    resolutions.set(placement, resolution);
    return { placement, frame, resolution };
  },
  paint(ctx, frame) {
    drawTimeline(ctx, {
      model: frame.model,
      layout: frame.layout.frame,
      art: frame.artProvider,
      theme: frame.theme,
      scrollTop: frame.binding.scrollTop,
      expanded: frame.binding.expanded,
      inspectedKey: decodeTimelineHit(frame.inspect)?.laneKey ?? null,
      selectedKey: frame.binding.selectedKey,
      focusedKey: frame.binding.focusedKey,
      resolution: frame.layout.resolution,
    });
    return false;
  },
  hit: (frame, point) => encodeTimelineHit(hitTimeline(frame.layout.frame, frame.layout.resolution, point[0], point[1], frame.binding.scrollTop)),
  tooltip(frame, key, focus) {
    const hit = focus ? focusHit(frame.model, key, frame.hour) : decodeTimelineHit(key);
    if (!hit) return null;
    const payload = tooltipFor(frame.projection as TimelineProjection<unknown>, frame.model, hit, frame.readNow(), frame.binding.options);
    return payload ? { hour: hit.hour ?? frame.hour, rowKey: hit.laneKey, payload } : null;
  },
  structure({ model, layout }) {
    return {
      meta: {
        empty: model.empty ?? null,
        emptyDetail: null,
        emptyKind: model.emptyKind ?? 'empty',
        summaryText: model.summaryText ?? null,
        summaryPartial: model.summaryPartial ?? null,
        notes: model.notes ?? [],
        warnings: model.warnings,
      },
      extent: { height: layout.frame.bodyTop + layout.frame.contentHeight + AXIS_HEIGHT } satisfies TimelineExtentField,
      nav: navOf(layout.frame),
    };
  },
  mirror: frame => mirrorOf(frame.layout.frame),
  audit: frame => deriveTimelineAudit(frame.model),
  visibleArt: frame => collectArt(frame.model, frame.layout.frame, frame.binding.scrollTop, frame.index),
};
