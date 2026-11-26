import { visibleLanes, type TimelineLane, type TimelineModel } from './model.ts';

export const LANE_HEIGHT = 38;
export const LANE_HEIGHT_COMPACT = 26;
export const GROUP_LANE_HEIGHT = 38;
export const HEADING_LANE_HEIGHT = 20;
export const HEADER_HEIGHT = 54;
export const METERS_MIN_HEIGHT = 240;
export const METERS_ROW_HEIGHT = 30;
export const METERS_HEADER_HEIGHT = 22;
export const METERS_PER_ROW = 3;
export const LEGEND_HEIGHT = 20;
export const AXIS_HEIGHT = 26;
export const TOP_AXIS_HEIGHT = 22;
export const BAR_HEIGHT = 14;
export const BAR_HEIGHT_ICONS = 18;
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

export interface TimelinePlacement {
  readonly width: number;
  readonly height: number;
  readonly compact: boolean;
  readonly labelWidth: number;
  readonly plotLeft: number;
  readonly plotRight: number;
  readonly headerHeight: number;
  readonly axisPosition: 'bottom' | 'top';
  readonly bodyTop: number;
  readonly bodyHeight: number;
  readonly contentHeight: number;
  readonly axisTop: number;
  readonly metersTop: number;
  readonly metersHeight: number;
  readonly lanes: readonly PlacedLane[];
  readonly startHour: number;
}

export interface TimelineLayout extends TimelinePlacement {
  readonly endHour: number;
}

export function laneHeight(lane: TimelineLane, compact: boolean): number {
  if (lane.height !== undefined) return lane.height;
  if (lane.kind === 'heading') return HEADING_LANE_HEIGHT;
  if (lane.kind === 'group') return compact || !lane.subtitle ? LANE_HEIGHT_COMPACT : GROUP_LANE_HEIGHT;
  return lane.subtitle && !compact ? LANE_HEIGHT : LANE_HEIGHT_COMPACT;
}

export function placeTimeline(
  model: TimelineModel,
  width: number,
  height: number,
  expanded: Readonly<Record<string, boolean>>,
): TimelinePlacement {
  const w = Math.max(160, width);
  const headerHeight = model.header ? HEADER_HEIGHT : 0;
  const legendHeight = model.legend?.length ? LEGEND_HEIGHT : 0;
  const axisPosition = model.axisPosition ?? 'bottom';
  const axisHeight = axisPosition === 'top' ? TOP_AXIS_HEIGHT : AXIS_HEIGHT;
  const bodyTop = headerHeight + legendHeight + (axisPosition === 'top' ? axisHeight : 0);
  const h = Math.max(bodyTop + (axisPosition === 'bottom' ? axisHeight : 0) + LANE_HEIGHT_COMPACT, height);
  const compact = w < MIN_SUBTITLE_WIDTH;
  const labelWidth = model.labelWidth < MIN_LABEL_WIDTH
    ? Math.max(48, Math.min(model.labelWidth, Math.floor(w * 0.28)))
    : compact
      ? MIN_LABEL_WIDTH
      : Math.max(MIN_LABEL_WIDTH, Math.min(model.labelWidth, Math.floor(w * 0.38)));
  const plotLeft = labelWidth + PLOT_GAP;
  const outcome = compact || w < 540 ? 0 : Math.min(model.outcomeWidth ?? 0, Math.floor(w * 0.25));
  const plotRight = Math.max(plotLeft + 24, w - RIGHT_PAD - outcome);
  const lanes: PlacedLane[] = [];
  let y = TOP_PAD;
  const shown = visibleLanes(model.lanes, expanded);
  for (let i = 0; i < shown.length; i++) {
    const lane = shown[i]!;
    const lh = laneHeight(lane, compact);
    lanes.push({ lane, ordinal: i, y, height: lh });
    y += lh;
  }
  const contentHeight = y + TOP_PAD;
  const wantedRows = Math.max(1, Math.ceil((model.meters?.items.length ?? 0) / METERS_PER_ROW));
  const roomRows = Math.floor((h - bodyTop - (axisPosition === 'bottom' ? axisHeight : 0) - LANE_HEIGHT_COMPACT - METERS_HEADER_HEIGHT - 8) / METERS_ROW_HEIGHT);
  const meterRows = Math.min(wantedRows, roomRows);
  const showMeters = !!model.meters && !compact && h >= METERS_MIN_HEIGHT && w >= 420 && meterRows >= 1;
  const metersHeight = showMeters ? METERS_HEADER_HEIGHT + meterRows * METERS_ROW_HEIGHT + 8 : 0;
  const bodyHeight = Math.max(
    LANE_HEIGHT_COMPACT,
    h - bodyTop - (axisPosition === 'bottom' ? axisHeight : 0) - metersHeight,
  );
  const axisTop = axisPosition === 'top' ? headerHeight + legendHeight : bodyTop + bodyHeight;
  const metersTop = axisPosition === 'top'
    ? Math.min(bodyTop + contentHeight + 6, h - metersHeight)
    : bodyTop + bodyHeight + axisHeight;
  return {
    width: w,
    height: h,
    compact,
    labelWidth,
    plotLeft,
    plotRight,
    headerHeight,
    axisPosition,
    bodyTop,
    bodyHeight,
    contentHeight,
    axisTop,
    metersTop,
    metersHeight,
    lanes,
    startHour: model.firstHour,
  };
}

export function frameTimeline(placement: TimelinePlacement, hour: number): TimelineLayout {
  return { ...placement, endHour: Math.max(placement.startHour + 1, hour) };
}

export function visibleRange(layout: TimelinePlacement, scrollTop: number): readonly [first: number, end: number] {
  const lanes = layout.lanes;
  let lo = 0;
  let hi = lanes.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (lanes[mid]!.y + lanes[mid]!.height <= scrollTop) lo = mid + 1;
    else hi = mid;
  }
  let end = lo;
  const bottom = scrollTop + layout.bodyHeight;
  while (end < lanes.length && lanes[end]!.y < bottom) end++;
  return [lo, end];
}

export function hoursPerPixel(layout: TimelineLayout): number {
  return (layout.endHour - layout.startHour) / Math.max(1, layout.plotRight - layout.plotLeft);
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

export function placedLaneAt(layout: TimelinePlacement, contentY: number): PlacedLane | null {
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
