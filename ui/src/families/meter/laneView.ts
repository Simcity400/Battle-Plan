import type { LaneBinding, LaneFamily, LaneFrame } from '../laneRuntime.ts';
import { easeBarFraction } from '../bars/painter.ts';
import { glideDurationForSpeed } from '../table/painter.ts';
import {
  deriveMeterAudit,
  deriveMeterMirror,
  deriveMeterNav,
  findMeterTooltip,
  nextMeterChange,
  type MeterModel,
} from './model.ts';
import { drawMeterModel, hitMeterModel, layoutMeterModel, type MeterLayout } from './painter.ts';

export interface FamilyMeterBinding<TOptions = unknown> extends LaneBinding<TOptions> {
  readonly family: 'meters';
  readonly width: number;
  readonly height: number;
  readonly pauseFlush?: number;
}

const inspectedRow = (inspect: string | null): string | null => (inspect?.startsWith('row:') ? inspect.slice(4) : null);

function collectMeterFractions(model: MeterModel): Map<string, number> {
  const out = new Map<string, number>();
  for (const section of model.sections ?? []) {
    for (const row of section.rows ?? []) {
      if (row.bar.kind !== 'none' && row.bar.fraction !== null && Number.isFinite(row.bar.fraction)) {
        out.set(row.key, row.bar.fraction);
      }
    }
  }
  return out;
}

function easedMetersOf(frame: LaneFrame<FamilyMeterBinding, MeterModel, MeterLayout>): {
  readonly map?: ReadonlyMap<string, number>;
  readonly moving: boolean;
} {
  const motion = frame.binding.motion;
  if (!motion?.easeValues) return { moving: false };
  const duration = glideDurationForSpeed(frame.speed, false, motion.glideDurationMs);
  if (duration <= 0 || !frame.before || frame.before.hour === frame.hour) return { moving: false };
  const progress = (frame.now - frame.changedAt) / duration;
  if (progress >= 1) return { moving: false };
  const prior = collectMeterFractions(frame.before.model);
  const current = collectMeterFractions(frame.model);
  const eased = new Map<string, number>();
  let moving = false;
  for (const [key, cur] of current) {
    const prev = prior.get(key);
    if (prev === undefined) continue;
    if (Math.abs(prev - cur) > 1e-4) moving = true;
    const blended = easeBarFraction(prev, cur, progress);
    if (blended !== null) eased.set(key, blended);
  }
  return moving ? { map: eased, moving: true } : { moving: false };
}

export const meterLane: LaneFamily<FamilyMeterBinding, MeterModel, MeterLayout> = {
  family: 'meters',
  boundary: nextMeterChange,
  redraw: () => Infinity,
  status: model => (model.refused !== undefined ? { refused: model.refused } : 'scene'),
  view: [],
  layout: (model, _binding, surface, theme, measure) => layoutMeterModel(measure, model, surface.width, theme),
  paint(ctx, frame) {
    const eased = easedMetersOf(frame);
    drawMeterModel(ctx, {
      layout: frame.layout,
      width: frame.surface.width,
      height: frame.surface.height,
      scrollLeft: 0,
      scrollTop: frame.binding.scrollTop,
      theme: frame.theme,
      inspectedKey: inspectedRow(frame.inspect),
      selectedKey: frame.binding.selectedKey ?? null,
      focusedKey: frame.binding.focusedKey ?? null,
      artCache: frame.art,
      ...(eased.map ? { easedFractions: eased.map } : {}),
    });
    return eased.moving;
  },
  hit: (frame, point) => hitMeterModel(frame.layout, point[0], point[1], 0, frame.binding.scrollTop),
  tooltip(frame, key, focus) {
    const rowKey = focus ? key : inspectedRow(key);
    const payload = rowKey ? findMeterTooltip(frame.model, rowKey) : null;
    return payload && rowKey ? { rowKey, payload } : null;
  },
  structure({ model, layout }) {
    return {
      meta: {
        empty: model.empty ?? null,
        emptyDetail: model.emptyDetail ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        summaryText: model.summaryText ?? null,
        notes: model.notes ?? [],
        warnings: model.warnings ?? [],
        layout: model.layout,
      },
      nav: deriveMeterNav(model, layout.itemBands),
      extent: { width: layout.contentWidth, height: layout.contentHeight },
    };
  },
  mirror: frame => deriveMeterMirror(frame.model),
  audit: frame => deriveMeterAudit(frame.model),
  visibleArt: () => [],
};
