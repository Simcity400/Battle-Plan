import type { FamilyNavTuple } from '../host/keyboard.ts';
import type { LaneBinding, LaneFamily } from '../laneRuntime.ts';
import { parseRegionHitKey, REGION_ADD_KEY, regionHitKey, type RegionRowsField } from '../regions/regionRows.ts';
import { hitHeatLanes, LANE_PAD, lanesWindow, layoutHeatLanes, type HeatLanesLayout } from './lanesLayout.ts';
import {
  deriveLanesAudit,
  deriveLanesMirror,
  deriveLanesTooltip,
  laneIdentities,
  laneInspectKey,
  parseLaneInspectKey,
  type HeatLanesModel,
} from './lanesModel.ts';
import { collectVisibleLaneArt, drawHeatLanes } from './lanesPainter.ts';

export interface FamilyHeatLanesBinding<TOptions = unknown> extends LaneBinding<TOptions> {
  readonly family: 'heat-lanes';
  readonly width: number;
  readonly height: number;
  readonly pauseFlush?: number;
}

const inspectedLane = (inspect: string | null) => parseLaneInspectKey(inspect?.startsWith('lane:') ? inspect.slice(5) : null);

function hoveredOf(model: HeatLanesModel, inspect: string | null): { readonly rowIndex: number | null; readonly hour: number | null; readonly add: boolean } {
  const lane = inspectedLane(inspect);
  if (lane) return { rowIndex: lane.rowIndex, hour: lane.hour, add: false };
  const region = parseRegionHitKey(inspect);
  if (!region) return { rowIndex: null, hour: null, add: false };
  if (region.part === 'add') return { rowIndex: null, hour: null, add: true };
  const rowIndex = model.rows.findIndex(row => row.id === region.key);
  return { rowIndex: rowIndex >= 0 ? rowIndex : null, hour: null, add: false };
}

function regionRows(model: HeatLanesModel, layout: HeatLanesLayout): RegionRowsField | null {
  if (!model.regions) return null;
  return {
    rows: model.rows.flatMap((row, index) => (row.region === undefined ? [] : [{
      key: row.id,
      id: row.region,
      label: row.label,
      picked: row.pinned === true,
      top: layout.plotTop + layout.rowTops[index]!,
      height: layout.rowHeight,
      ...(row.readout?.kind === 'value'
        ? { activity: { ...(typeof row.readout.swatch === 'string' && row.readout.swatch !== 'hollow' ? { color: row.readout.swatch } : {}), text: [row.readout.primary, row.readout.secondary].filter(Boolean).join(' ') } }
        : {}),
    }])),
    add: layout.addTop === null ? null : { top: layout.plotTop + layout.addTop, height: layout.rowHeight },
    nameLeft: LANE_PAD,
  };
}

const windows = new WeakMap<HeatLanesLayout, { readonly scrollTop: number; readonly layout: HeatLanesLayout }>();

function shown(frame: { readonly model: HeatLanesModel; readonly layout: HeatLanesLayout; readonly binding: { readonly scrollTop: number } }): HeatLanesLayout {
  const held = windows.get(frame.layout);
  if (held && held.scrollTop === frame.binding.scrollTop) return held.layout;
  const layout = { ...frame.layout, ...lanesWindow(frame.layout, frame.model.rows.length, frame.binding.scrollTop) };
  windows.set(frame.layout, { scrollTop: frame.binding.scrollTop, layout });
  return layout;
}

export const heatLanesLane: LaneFamily<FamilyHeatLanesBinding, HeatLanesModel, HeatLanesLayout> = {
  family: 'heat-lanes',
  boundary: () => Infinity,
  redraw: () => Infinity,
  status: model => (model.refused !== undefined ? { refused: model.refused } : 'scene'),
  view: [],
  layout: (model, _binding, surface) => layoutHeatLanes({
    width: surface.width,
    height: surface.height,
    scrollTop: 0,
    firstHour: model.firstHour,
    lastHour: model.lastHour,
    displayedHour: model.hour,
    rows: model.rows,
    regions: model.regions,
    legend: model.legend,
    requestedReadoutWidth: model.readoutWidth,
    requestedRowHeight: model.rowHeight,
  }),
  paint(ctx, frame) {
    const hovered = hoveredOf(frame.model, frame.inspect);
    drawHeatLanes(ctx, {
      model: frame.model,
      layout: shown(frame),
      theme: frame.theme,
      scrollTop: frame.binding.scrollTop,
      hoveredRow: hovered.rowIndex,
      hoveredAdd: hovered.add,
      hoveredHour: hovered.hour,
      selectedRowId: frame.binding.selectedKey ?? null,
      focusedRowId: frame.binding.focusedKey ?? null,
      artCache: frame.art,
    });
    return false;
  },
  hit(frame, point) {
    const { model } = frame;
    const focused = frame.binding.focusedKey ? model.rows.findIndex(row => row.id === frame.binding.focusedKey) : -1;
    const toolsRow = hoveredOf(model, frame.inspect).rowIndex ?? (focused >= 0 ? focused : null);
    const hit = hitHeatLanes(point[0], point[1], frame.binding.scrollTop, frame.layout, model.rows, toolsRow);
    if (!hit) return null;
    if (hit.kind === 'add') return regionHitKey('add', REGION_ADD_KEY);
    if (hit.kind === 'remove' || hit.kind === 'grip') return regionHitKey(hit.kind, model.rows[hit.rowIndex]!.id);
    return `lane:${laneInspectKey(hit.rowIndex, hit.kind === 'lane' ? hit.hour : null)}`;
  },
  tooltip(frame, key, focus) {
    const { model } = frame;
    if (focus) {
      const row = model.rows.findIndex(item => item.id === key);
      const payload = row >= 0 ? deriveLanesTooltip(model, row, model.hour) : null;
      return payload ? { hour: model.hour, rowKey: key, payload } : null;
    }
    const hovered = inspectedLane(key);
    if (!hovered) return null;
    const payload = deriveLanesTooltip(model, hovered.rowIndex, hovered.hour);
    return payload ? { hour: hovered.hour ?? model.hour, rowKey: laneInspectKey(hovered.rowIndex, hovered.hour), payload } : null;
  },
  structure({ model, layout }) {
    const regions = regionRows(model, layout);
    const nav = model.rows.map((row, index): FamilyNavTuple => [row.id, row.label, index, -1, '', row.meta ?? null, layout.rowTops[index]!, layout.rowHeight]);
    if (model.regions && layout.addTop !== null) nav.push([REGION_ADD_KEY, model.regions.addLabel, model.rows.length, -1, '', null, layout.addTop, layout.rowHeight]);
    return {
      ...(regions ? { regions } : {}),
      meta: {
        empty: model.empty ?? null,
        emptyDetail: model.emptyDetail ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        notes: model.notes ?? [],
        warnings: model.warnings ?? [],
      },
      extent: { totalHeight: layout.totalHeight, rowStride: layout.rowStride, plotTop: layout.plotTop, plotHeight: layout.plotHeight, rows: laneIdentities(model.rows) },
      nav,
    };
  },
  mirror: frame => {
    const window = shown(frame);
    return deriveLanesMirror(frame.model, window.firstRow, window.endRow);
  },
  audit: frame => {
    const window = shown(frame);
    return deriveLanesAudit(frame.model, { firstRow: window.firstRow, endRow: window.endRow });
  },
  visibleArt: frame => collectVisibleLaneArt(frame.model, shown(frame)),
};
