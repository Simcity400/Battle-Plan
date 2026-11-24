import { visibleLanes, type TimelineLane, type TimelineModel } from './model.ts';

export const LANE_HEIGHT = 38;
export const LANE_HEIGHT_COMPACT = 26;
export const GROUP_LANE_HEIGHT = 38;
export const HEADING_LANE_HEIGHT = 22;
export const AXIS_HEIGHT = 26;
export const BAR_HEIGHT = 14;
export const BAR_HEIGHT_ICONS = 18;
export const BAR_ICON_MIN_WIDTH = 22;
export const LANE_ICON = 22;
export const GROUP_RULE_HEIGHT = 4;
export const TOP_PAD = 4;
export const PLOT_GAP = 12;
export const RIGHT_PAD = 12;
export const MIN_LABEL_WIDTH = 96;
export const MIN_SUBTITLE_WIDTH = 360;

export interface PlacedLane {
  readonly lane: TimelineLane;
  readonly ordinal: number;
  readonly y: number;
  readonly height: number;
}

export interface TimelineLayout {
  readonly width: number;
  readonly height: number;
  readonly compact: boolean;
  readonly labelWidth: number;
  readonly plotLeft: number;
  readonly plotRight: number;
  readonly bodyHeight: number;
  readonly contentHeight: number;
  readonly axisTop: number;
  readonly lanes: readonly PlacedLane[];
  readonly startHour: number;
  readonly endHour: number;
}

export function laneHeight(lane: TimelineLane, compact: boolean): number {
  if (lane.kind === 'heading') return HEADING_LANE_HEIGHT;
  if (lane.kind === 'group') return compact || !lane.subtitle ? LANE_HEIGHT_COMPACT : GROUP_LANE_HEIGHT;
  return lane.subtitle && !compact ? LANE_HEIGHT : LANE_HEIGHT_COMPACT;
}

export function layoutTimeline(
  model: TimelineModel,
  width: number,
  height: number,
  expanded: Readonly<Record<string, boolean>>,
): TimelineLayout {
  const w = Math.max(160, width);
  const h = Math.max(AXIS_HEIGHT + LANE_HEIGHT_COMPACT, height);
  const compact = w < MIN_SUBTITLE_WIDTH;
  const labelWidth = compact
    ? MIN_LABEL_WIDTH
    : Math.max(MIN_LABEL_WIDTH, Math.min(model.labelWidth, Math.floor(w * 0.38)));
  const plotLeft = labelWidth + PLOT_GAP;
  const plotRight = Math.max(plotLeft + 24, w - RIGHT_PAD);
  const lanes: PlacedLane[] = [];
  let y = TOP_PAD;
  const shown = visibleLanes(model.lanes, expanded);
  for (let i = 0; i < shown.length; i++) {
    const lane = shown[i]!;
    const lh = laneHeight(lane, compact);
    lanes.push({ lane, ordinal: i, y, height: lh });
    y += lh;
  }
  const bodyHeight = h - AXIS_HEIGHT;
  return {
    width: w,
    height: h,
    compact,
    labelWidth,
    plotLeft,
    plotRight,
    bodyHeight,
    contentHeight: y + TOP_PAD,
    axisTop: bodyHeight,
    lanes,
    startHour: model.extent[0],
    endHour: Math.max(model.extent[0] + 1, model.extent[1]),
  };
}

export function hourToX(layout: TimelineLayout, hour: number): number {
  const span = layout.endHour - layout.startHour;
  return layout.plotLeft + ((hour - layout.startHour) / span) * (layout.plotRight - layout.plotLeft);
}

export function xToHour(layout: TimelineLayout, x: number): number {
  const clamped = Math.max(layout.plotLeft, Math.min(layout.plotRight, x));
  const span = layout.endHour - layout.startHour;
  return layout.startHour + ((clamped - layout.plotLeft) / Math.max(1, layout.plotRight - layout.plotLeft)) * span;
}

export function placedLaneAt(layout: TimelineLayout, contentY: number): PlacedLane | null {
  let lo = 0;
  let hi = layout.lanes.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >>> 1;
    const placed = layout.lanes[mid]!;
    if (contentY < placed.y) hi = mid - 1;
    else if (contentY >= placed.y + placed.height) lo = mid + 1;
    else return placed;
  }
  return null;
}
