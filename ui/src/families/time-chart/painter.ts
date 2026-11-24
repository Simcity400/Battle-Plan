import type { Canvas2D } from '../../charts/textWidth.ts';
import {
  drawArtSlot,
  drawCrosshairLine,
  drawDateTab,
  drawFlagSlot,
  drawGapBand,
  drawLegendEntry,
  drawPlayheadLine,
  drawSeriesMissingStrip,
  resolveKitTokens,
  type ArtSlotState,
  type FlagSlotState,
} from '../../draw/kit.ts';
import { parseColor } from '../../shell/appearance/color.ts';
import type { FamilyArtBitmap } from '../artLoader.ts';
import type { FamilyTheme } from '../contract.ts';
import { formatDateOnly } from '../valueFormat.ts';
import { parseChartInspect, sampleSeriesValueAt } from './hitTest.ts';
import { formatChartAxisValue, type TimeChartLayout } from './layout.ts';
import {
  CHART_MEASURE_COLOURS,
  type ChartArtRef,
  type TimeChartModel,
  type TimeChartSeriesModel,
} from './model.ts';

const DEFAULT_THEME: FamilyTheme = {
  surface: '#111111',
  raised: '#262626',
  dark: true,
  text: '#f5f5f5',
  muted: '#929292',
  line: 'rgba(245, 245, 245, 0.18)',
  lineSoft: 'rgba(245, 245, 245, 0.055)',
  accent: '#4f86f7',
  blue: '#4f86f7',
  red: CHART_MEASURE_COLOURS.short,
  shortage: CHART_MEASURE_COLOURS.shortInk,
  amber: CHART_MEASURE_COLOURS.politicalPower,
  green: CHART_MEASURE_COLOURS.supplyOk,
  grey: '#929292',
  hatch: 'rgba(245, 245, 245, 0.17)',
  hatchEdge: 'rgba(245, 245, 245, 0.12)',
  font: "'Segoe UI', system-ui, sans-serif",
  fontSize: { small: 11, body: 12 },
  lineWidth: 1,
  hatchSpacing: 5,
};

function artSlotState(art: ChartArtRef | undefined, bitmaps: ReadonlyMap<string, FamilyArtBitmap | null>): ArtSlotState {
  if (!art || art.state === 'none') return { kind: 'none' };
  if (art.state === 'missing') return { kind: 'missing', resolvedName: art.label ?? art.tag };
  if (art.state === 'refused') return { kind: 'refused', reason: art.reason };
  if (art.state === 'failed') return { kind: 'failed', reason: art.reason };
  if (art.url) {
    const bmp = bitmaps.get(art.url);
    if (bmp) return { kind: 'ready', bitmap: bmp as unknown as CanvasImageSource, width: bmp.width, height: bmp.height };
  }
  return { kind: 'pending' };
}

function flagSlotState(art: ChartArtRef | undefined, bitmaps: ReadonlyMap<string, FamilyArtBitmap | null>): FlagSlotState {
  if (!art || art.state === 'pending' || art.state === 'none') return { kind: 'pending' };
  if (art.state === 'missing') return { kind: 'missing', tag: art.tag ?? art.label };
  if (art.state === 'failed' || art.state === 'refused') return { kind: 'failed' };
  if (art.url) {
    const bmp = bitmaps.get(art.url);
    if (bmp) return { kind: 'ready', bitmap: bmp as unknown as CanvasImageSource, width: bmp.width, height: bmp.height };
  }
  return { kind: 'pending' };
}

export function collectVisibleChartArt(model: TimeChartModel, layout: TimeChartLayout): string[] {
  const urls = new Set<string>();
  for (const label of layout.endLabels) {
    if (label.flag?.url && label.flag.state !== 'missing') urls.add(label.flag.url);
    if (label.icon?.url && label.icon.state !== 'missing') urls.add(label.icon.url);
  }
  for (const m of layout.markers) {
    if (m.kind === 'flag' && m.marker.flag?.url && m.marker.flag.state !== 'missing') urls.add(m.marker.flag.url);
    if (m.kind === 'icon' && m.marker.icon?.url && m.marker.icon.state !== 'missing') urls.add(m.marker.icon.url);
  }
  if (model.legend) {
    for (const item of model.legend) {
      if (item.flag?.url && item.flag.state !== 'missing') urls.add(item.flag.url);
    }
  }
  if (model.overlays) {
    for (const ov of model.overlays) {
      if (ov.icon?.url && ov.icon.state !== 'missing') urls.add(ov.icon.url);
    }
  }
  return [...urls];
}

function buildLineSubpaths(
  series: TimeChartSeriesModel,
  layout: TimeChartLayout,
  until: number,
): { readonly subpaths: readonly (readonly [number, number][])[]; readonly end: readonly [number, number] | null } {
  const step = series.step === true;
  const subpaths: [number, number][][] = [];
  let end: readonly [number, number] | null = null;
  const bounded = series.bounded;

  if (bounded) {
    if (bounded.runs && bounded.runs.length > 0) {
      let cur: [number, number][] = [];
      for (const run of bounded.runs) {
        if (run.lastHour < layout.xDomain[0] || run.firstHour > layout.xDomain[1]) continue;
        if (run.breakBefore && cur.length) {
          subpaths.push(cur);
          cur = [];
        }
        const x0 = layout.toX(Math.max(run.firstHour, layout.xDomain[0]));
        const x1 = layout.toX(Math.min(run.lastHour, layout.xDomain[1]));
        const y = layout.toY(run.value);
        if (!cur.length) cur.push([x0, y]);
        else if (step) cur.push([x0, cur[cur.length - 1]![1]], [x0, y]);
        else cur.push([x0, y]);
        if (x1 > x0) cur.push([x1, y]);
        end = [x1, y];
      }
      if (cur.length) subpaths.push(cur);
    } else {
      let cur: [number, number][] = [];
      let prevLastHour = -Infinity;
      for (const b of bounded.buckets) {
        if (!b || b.lastHour < layout.xDomain[0] || b.firstHour > layout.xDomain[1]) continue;
        const disconnected = cur.length > 0 && (b.firstBreak || b.firstHour > prevLastHour + Math.max(1, bounded.binHours));
        if (disconnected) {
          subpaths.push(cur);
          cur = [];
        }
        const pushPoint = (h: number, v: number) => {
          const x = layout.toX(Math.max(layout.xDomain[0], Math.min(layout.xDomain[1], h)));
          const y = layout.toY(v);
          if (!cur.length) cur.push([x, y]);
          else if (step) cur.push([x, cur[cur.length - 1]![1]], [x, y]);
          else cur.push([x, y]);
          end = [x, y];
        };
        pushPoint(b.firstHour, b.firstValue);
        if (b.minValue !== b.firstValue || b.maxValue !== b.firstValue) {
          if (b.minHour <= b.maxHour) {
            if (b.minValue !== b.firstValue) pushPoint(b.minHour, b.minValue);
            if (b.maxValue !== b.minValue) pushPoint(b.maxHour, b.maxValue);
          } else {
            if (b.maxValue !== b.firstValue) pushPoint(b.maxHour, b.maxValue);
            if (b.minValue !== b.maxValue) pushPoint(b.minHour, b.minValue);
          }
        }
        if (b.lastHour !== b.firstHour || b.lastValue !== b.firstValue) {
          pushPoint(b.lastHour, b.lastValue);
        }
        prevLastHour = b.lastHour;
      }
      if (cur.length) subpaths.push(cur);
    }
    if (series.headValue != null && Number.isFinite(series.headValue)) {
      const hx = layout.toX(Math.min(series.headHour ?? until, layout.xDomain[1]));
      const hy = layout.toY(series.headValue);
      const last = subpaths[subpaths.length - 1];
      if (last && last.length > 0 && hx >= last[last.length - 1]![0]) {
        if (hx > last[last.length - 1]![0] || hy !== last[last.length - 1]![1]) {
          if (step) last.push([hx, last[last.length - 1]![1]]);
          last.push([hx, hy]);
        }
      } else if (!last) {
        subpaths.push([[hx, hy]]);
      }
      end = [hx, hy];
    }
    return { subpaths, end };
  }

  if (series.points && series.points.length > 0) {
    let cur: [number, number][] = [];
    for (const pt of series.points) {
      if (pt.hour > until) break;
      if (pt.value === null || !Number.isFinite(pt.value)) {
        if (cur.length) { subpaths.push(cur); cur = []; }
        continue;
      }
      const x = layout.toX(pt.hour);
      const y = layout.toY(pt.value);
      if (!cur.length) cur.push([x, y]);
      else if (step) cur.push([x, cur[cur.length - 1]![1]], [x, y]);
      else cur.push([x, y]);
      end = [x, y];
    }
    if (cur.length) subpaths.push(cur);
  }
  return { subpaths, end };
}

function tracePolyline(ctx: Canvas2D, pts: readonly (readonly [number, number])[]): void {
  if (!pts.length) return;
  ctx.beginPath();
  ctx.moveTo(pts[0]![0], pts[0]![1]);
  for (let i = 1; i < pts.length; i++) {
    ctx.lineTo(pts[i]![0], pts[i]![1]);
  }
}

export function drawTimeChart(
  ctx: Canvas2D,
  model: TimeChartModel,
  layout: TimeChartLayout,
  themeInput: FamilyTheme | undefined,
  bitmaps: ReadonlyMap<string, FamilyArtBitmap | null>,
  inspectKey: string | null | undefined,
  dpr: number,
): void {
  const theme = themeInput ?? DEFAULT_THEME;
  const tokens = resolveKitTokens(theme);
  const parsedInspect = parseChartInspect(inspectKey);
  const hoverHour = parsedInspect?.kind === 'plot' || parsedInspect?.kind === 'line'
    ? parsedInspect.hour
    : parsedInspect?.kind === 'marker'
      ? layout.markers[parsedInspect.index]?.marker.hour ?? null
      : null;
  const hoveredSeriesId = parsedInspect?.kind === 'label' || parsedInspect?.kind === 'line'
    ? parsedInspect.seriesId
    : null;

  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, layout.width, layout.height);

  const smallFont = `${theme.fontSize.small}px ${theme.font}`;
  const labelNameFont = `500 ${theme.fontSize.body}px ${theme.font}`;
  const labelValueFont = `600 ${theme.fontSize.body}px ${theme.font}`;
  const overlayFont = `${theme.fontSize.body}px ${theme.font}`;
  const textSecondary = tokens.dark ? '#d4d4d4' : theme.muted;

  if (layout.legendTop !== null && model.legend && model.legend.length > 0) {
    let lx = layout.left;
    const ly = layout.legendTop + 7;
    for (const item of model.legend) {
      const drawn = drawLegendEntry(ctx, lx, ly, item.label, item.color ?? '', theme, {
        dash: item.dash,
        hatch: item.hatch,
        flag: item.flag ? flagSlotState(item.flag, bitmaps) : undefined,
      });
      lx += drawn.width + 14;
    }
  }

  for (const v of layout.yTicks) {
    const y = Math.round(layout.toY(v)) + 0.5;
    if (y < layout.top - 2 || y > layout.baseY + 2) continue;
    const isZeroCrossing = v === 0 && layout.yDomain[0] < 0;
    ctx.save();
    ctx.strokeStyle = isZeroCrossing ? tokens.zeroLine : tokens.gridLine;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(layout.left, y);
    ctx.lineTo(layout.left + layout.plotWidth, y);
    ctx.stroke();
    ctx.font = smallFont;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillText(formatChartAxisValue(v, model.yAxis.format), layout.left - 8, y);
    ctx.restore();
  }

  for (const g of layout.gaps) {
    drawGapBand(ctx, g.x0, layout.top, g.x1 - g.x0, layout.plotHeight, theme);
  }

  if (model.mode === 'columns') {
    for (const col of layout.columns) {
      if (col.kind === 'fold') {
        drawGapBand(ctx, col.barX, layout.top, col.barWidth, layout.plotHeight, theme);
        continue;
      }
      const m = col.month!;
      if (m.status !== 'recorded') {
        drawGapBand(ctx, col.barX, layout.top, col.barWidth, layout.plotHeight, theme);
        continue;
      }
      let yb = layout.toY(0);
      for (const seg of m.segments) {
        if (seg.value === 0) continue;
        const nextY = layout.toY(seg.value);
        const h = Math.abs(layout.toY(0) - nextY);
        ctx.save();
        ctx.globalAlpha = m.partial ? 0.55 : 1;
        ctx.fillStyle = seg.color;
        if (seg.value > 0) {
          ctx.fillRect(col.barX, yb - h, col.barWidth, h);
          yb -= h;
        } else {
          ctx.fillRect(col.barX, layout.toY(0), col.barWidth, h);
        }
        ctx.restore();
      }
    }
  } else if (model.mode === 'stack' && model.stack && model.stack.hours.length > 0) {
    const { hours, bands, total } = model.stack;
    let count = 0;
    while (count < hours.length && hours[count]! <= model.hour) count++;
    if (count > 0) {
      const acc = new Float64Array(count);
      ctx.save();
      ctx.beginPath();
      ctx.rect(layout.left, layout.top - 2, layout.plotWidth + 1, layout.plotHeight + 4);
      ctx.clip();
      for (const band of bands) {
        const topVals = new Float64Array(count);
        for (let i = 0; i < count; i++) {
          topVals[i] = acc[i]! + (band.values[i] ?? 0);
        }
        ctx.beginPath();
        for (let i = 0; i < count; i++) {
          const x = layout.toX(hours[i]!);
          const y = layout.toY(topVals[i]!);
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        for (let i = count - 1; i >= 0; i--) {
          ctx.lineTo(layout.toX(hours[i]!), layout.toY(acc[i]!));
        }
        ctx.closePath();
        ctx.fillStyle = band.color;
        ctx.globalAlpha = (band.opacity ?? 0.78) * (hoveredSeriesId && hoveredSeriesId !== band.id ? 0.35 : 1);
        ctx.fill();
        ctx.globalAlpha = 0.9;
        ctx.strokeStyle = theme.surface;
        ctx.lineWidth = 0.8;
        ctx.stroke();
        acc.set(topVals);
      }
      if (total) {
        ctx.beginPath();
        for (let i = 0; i < count; i++) {
          const tv = total.values[i];
          if (tv == null) continue;
          const x = layout.toX(hours[i]!);
          const y = layout.toY(tv);
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.globalAlpha = 1;
        ctx.strokeStyle = total.color ?? theme.text;
        ctx.lineWidth = 1.6;
        ctx.lineJoin = 'round';
        ctx.stroke();
      }
      ctx.restore();
    }
    drawPlayheadLine(ctx, layout.playheadX, layout.top - 2, layout.baseY + layout.markerLaneHeight, theme);
  } else {
    const zeroY = layout.toY(0);
    const clipZeroY = layout.yDomain[0] < 0 ? zeroY : layout.baseY + 4;
    const endDots: { readonly x: number; readonly y: number; readonly color: string; readonly opacity: number }[] = [];

    for (let idx = model.series.length - 1; idx >= 0; idx--) {
      const s = model.series[idx]!;
      const { subpaths, end } = buildLineSubpaths(s, layout, model.hour);
      const baseOpacity = s.opacity ?? (s.role === 'tail' ? 0.22 : s.role === 'ceiling' ? 0.55 : 1);
      const opacity = hoveredSeriesId && hoveredSeriesId !== s.id ? Math.min(baseOpacity, 0.33) : baseOpacity;
      const lineWidth = s.width ?? (s.role === 'tail' ? 1 : s.role === 'ceiling' ? 1.4 : model.series.length > 2 ? 1.6 : 1.75);

      ctx.save();
      ctx.beginPath();
      ctx.rect(layout.left, layout.top - 2, layout.plotWidth + 1, layout.plotHeight + 4);
      ctx.clip();

      if (s.area && subpaths.length > 0) {
        const baselineY = layout.toY(Math.max(layout.yDomain[0], 0));
        for (const pts of subpaths) {
          if (pts.length < 2) continue;
          ctx.save();
          ctx.beginPath();
          ctx.rect(layout.left, layout.top - 2, layout.plotWidth + 1, Math.max(0, clipZeroY - (layout.top - 2)));
          ctx.clip();
          tracePolyline(ctx, pts);
          ctx.lineTo(pts[pts.length - 1]![0], baselineY);
          ctx.lineTo(pts[0]![0], baselineY);
          ctx.closePath();
          const grad = ctx.createLinearGradient(0, layout.top, 0, baselineY);
          const rgb = parseColor(s.color);
          if (rgb) {
            grad.addColorStop(0, `rgba(${rgb.r}, ${rgb.g}, ${rgb.b}, ${(0.26 * opacity).toFixed(3)})`);
            grad.addColorStop(1, `rgba(${rgb.r}, ${rgb.g}, ${rgb.b}, ${(0.02 * opacity).toFixed(3)})`);
            ctx.globalAlpha = 1;
          } else {
            grad.addColorStop(0, s.color);
            grad.addColorStop(1, s.color);
            ctx.globalAlpha = 0.22 * opacity;
          }
          ctx.fillStyle = grad;
          ctx.fill();
          ctx.restore();

          if (s.negRed && layout.yDomain[0] < 0) {
            ctx.save();
            ctx.beginPath();
            ctx.rect(layout.left, clipZeroY, layout.plotWidth + 1, Math.max(0, layout.baseY + 4 - clipZeroY));
            ctx.clip();
            tracePolyline(ctx, pts);
            ctx.lineTo(pts[pts.length - 1]![0], zeroY);
            ctx.lineTo(pts[0]![0], zeroY);
            ctx.closePath();
            ctx.globalAlpha = 0.16 * opacity;
            ctx.fillStyle = theme.red;
            ctx.fill();
            ctx.restore();
          }
        }
      }

      for (const pts of subpaths) {
        if (pts.length < 1) continue;
        const strokeSegment = (strokeColor: string, clipTop: number, clipHeight: number) => {
          ctx.save();
          if (clipHeight > 0) {
            ctx.beginPath();
            ctx.rect(layout.left, clipTop, layout.plotWidth + 1, clipHeight);
            ctx.clip();
          }
          ctx.globalAlpha = opacity;
          ctx.strokeStyle = strokeColor;
          ctx.lineWidth = lineWidth;
          ctx.lineJoin = 'round';
          ctx.lineCap = 'round';
          if (s.dash?.length) ctx.setLineDash([...s.dash]);
          tracePolyline(ctx, pts);
          ctx.stroke();
          ctx.restore();
        };
        if (s.negRed && layout.yDomain[0] < 0) {
          strokeSegment(s.color, layout.top - 2, Math.max(0, clipZeroY - (layout.top - 2)));
          strokeSegment(tokens.shortage, clipZeroY, Math.max(0, layout.baseY + 4 - clipZeroY));
        } else {
          strokeSegment(s.color, 0, 0);
        }
      }

      if (s.missingSpans && s.missingSpans.length > 0) {
        for (const span of s.missingSpans) {
          const sa = Math.max(layout.xDomain[0], span.firstHour);
          const sb = Math.min(layout.xDomain[1], model.hour, span.lastHour);
          if (sb <= sa) continue;
          const sx0 = layout.toX(sa);
          const sw = Math.max(2, layout.toX(sb) - sx0);
          drawSeriesMissingStrip(ctx, sx0, layout.baseY - 6, sw, s.color, theme, 5);
        }
      }
      ctx.restore();

      if (end && s.endLabel) {
        const isNeg = s.negRed && s.headValue != null && s.headValue < 0;
        endDots.push({ x: end[0], y: end[1], color: isNeg ? tokens.shortage : s.color, opacity });
      }
    }

    drawPlayheadLine(ctx, layout.playheadX, layout.top - 2, layout.baseY + layout.markerLaneHeight, theme);

    for (const dot of endDots) {
      ctx.save();
      ctx.globalAlpha = dot.opacity;
      ctx.beginPath();
      ctx.arc(dot.x, dot.y, 3.6, 0, Math.PI * 2);
      ctx.fillStyle = dot.color;
      ctx.fill();
      ctx.strokeStyle = theme.surface;
      ctx.lineWidth = 2;
      ctx.stroke();
      ctx.restore();
    }
  }

  for (const label of layout.endLabels) {
    ctx.save();
    if (label.faded) ctx.globalAlpha = 0.35;
    if (label.hasLeader) {
      ctx.strokeStyle = tokens.playhead;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(layout.playheadX + 4, label.anchorY);
      ctx.lineTo(label.x - 3, label.y);
      ctx.stroke();
    }
    let cursorX = label.x;
    if (label.colorBar) {
      ctx.fillStyle = label.colorBar;
      ctx.beginPath();
      ctx.roundRect(cursorX, label.y - 6, 3, 12, 1);
      ctx.fill();
      cursorX += 9;
    }
    if (label.flag && label.flag.state !== 'none') {
      drawFlagSlot(ctx, cursorX, label.y - 6, flagSlotState(label.flag, bitmaps), theme, { width: 18, height: 12 });
      cursorX += 24;
    } else if (label.icon && label.icon.state !== 'none') {
      drawArtSlot(ctx, cursorX, label.y - 7.5, 16, 15, artSlotState(label.icon, bitmaps), theme, { radius: 0 });
      cursorX += 22;
    }
    if (label.name) {
      ctx.font = labelNameFont;
      ctx.fillStyle = textSecondary;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(label.name, cursorX, label.y + 0.5);
      cursorX += ctx.measureText(label.name).width + 6;
    }
    ctx.font = labelValueFont;
    ctx.fillStyle = label.valueColor ?? theme.text;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(label.value, cursorX, label.y + 0.5);
    ctx.restore();
  }

  if (model.overlays && !layout.small) {
    for (const ov of model.overlays) {
      ctx.save();
      ctx.font = overlayFont;
      const tw = ctx.measureText(ov.text).width;
      const ox = layout.left + layout.plotWidth - 8 - tw;
      const oy = ov.position === 'top-right' ? layout.top + 12 : layout.baseY - 12;
      if (ov.arrow) {
        ctx.fillStyle = ov.color ?? textSecondary;
        ctx.beginPath();
        const ax = ox - 12;
        if (ov.arrow === 'right') {
          ctx.moveTo(ax, oy - 3.5);
          ctx.lineTo(ax + 6, oy);
          ctx.lineTo(ax, oy + 3.5);
        } else {
          ctx.moveTo(ax + 6, oy - 3.5);
          ctx.lineTo(ax, oy);
          ctx.lineTo(ax + 6, oy + 3.5);
        }
        ctx.closePath();
        ctx.fill();
      }
      ctx.fillStyle = textSecondary;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(ov.text, ox, oy);
      ctx.restore();
    }
  }

  for (const m of layout.markers) {
    if (m.kind === 'flag') {
      drawFlagSlot(ctx, m.x - 8, m.y - 5, flagSlotState(m.marker.flag, bitmaps), theme, { width: 16, height: 10 });
    } else if (m.kind === 'icon') {
      drawArtSlot(ctx, m.x - 9, m.y - 9, 18, 18, artSlotState(m.marker.icon, bitmaps), theme, { radius: 0 });
    } else {
      ctx.save();
      ctx.strokeStyle = m.marker.tickColor ?? (tokens.dark ? 'rgba(245, 245, 245, 0.42)' : 'rgba(15, 23, 42, 0.42)');
      ctx.lineWidth = 1.6;
      ctx.lineCap = 'round';
      ctx.beginPath();
      ctx.moveTo(m.x, m.y - m.size / 2);
      ctx.lineTo(m.x, m.y + m.size / 2);
      ctx.stroke();
      ctx.restore();
    }
  }

  ctx.save();
  ctx.strokeStyle = tokens.zeroLine;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(layout.left, layout.axisY + 0.5);
  ctx.lineTo(layout.left + layout.plotWidth, layout.axisY + 0.5);
  ctx.stroke();
  ctx.restore();

  for (const g of layout.gaps) {
    drawSeriesMissingStrip(ctx, g.x0, layout.axisY - 2, g.x1 - g.x0, theme.hatch, theme, 5);
  }

  if (model.mode === 'columns') {
    for (const col of layout.columns) {
      const cx = col.x + col.width / 2;
      if (col.kind === 'fold') {
        ctx.save();
        ctx.fillStyle = theme.surface;
        ctx.fillRect(cx - 5, layout.axisY - 4, 10, 9);
        ctx.strokeStyle = tokens.zeroLine;
        ctx.lineWidth = 1.3;
        ctx.beginPath();
        ctx.moveTo(cx - 5, layout.axisY + 4);
        ctx.lineTo(cx - 1, layout.axisY - 4);
        ctx.moveTo(cx + 1, layout.axisY + 4);
        ctx.lineTo(cx + 5, layout.axisY - 4);
        ctx.stroke();
        ctx.restore();
      }
      if (col.label && Math.abs(cx - layout.dateTabX) > layout.dateTabWidth / 2 + 10) {
        ctx.save();
        ctx.font = smallFont;
        ctx.fillStyle = col.majorLabel ? textSecondary : theme.muted;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'top';
        ctx.fillText(col.label, cx, layout.axisY + 6);
        ctx.restore();
      }
    }
  } else {
    for (const tick of layout.axisTicks) {
      ctx.save();
      ctx.strokeStyle = tokens.zeroLine;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(tick.x + 0.5, layout.axisY);
      ctx.lineTo(tick.x + 0.5, layout.axisY + (tick.major ? 5 : 3));
      ctx.stroke();
      if (tick.label) {
        ctx.font = smallFont;
        ctx.fillStyle = tick.major ? textSecondary : theme.muted;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'top';
        ctx.fillText(tick.label, tick.x, layout.axisY + 6);
      }
      ctx.restore();
    }
  }

  drawDateTab(ctx, layout.dateTabText, layout.dateTabX, layout.axisY + 3, theme, 'playhead', {
    left: layout.left,
    right: layout.width - 6,
  });

  if (hoverHour != null && model.mode !== 'columns' && hoverHour >= layout.xDomain[0] && hoverHour <= model.hour) {
    const xh = layout.toX(hoverHour);
    drawCrosshairLine(ctx, xh, layout.top - 2, layout.axisY, theme);
    for (const s of model.series) {
      const sampled = sampleSeriesValueAt(s, hoverHour, model.hour);
      if (!sampled) continue;
      const dotColor = s.negRed && sampled.value < 0 ? tokens.shortage : s.color;
      ctx.save();
      ctx.beginPath();
      ctx.arc(xh, layout.toY(sampled.value), 3.4, 0, Math.PI * 2);
      ctx.fillStyle = dotColor;
      ctx.fill();
      ctx.strokeStyle = theme.surface;
      ctx.lineWidth = 1.8;
      ctx.stroke();
      ctx.restore();
    }
    drawDateTab(ctx, formatDateOnly(hoverHour), xh, layout.axisY + 3, theme, 'hover', {
      left: layout.left,
      right: layout.width - 6,
    });
  }
}
