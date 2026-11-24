import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import {
  DEFAULT_FAMILY_THEME,
  drawArtSlot,
  drawCrosshairLine,
  drawDateTab,
  drawFlagSlot,
  drawGapBand,
  drawLegendStrip,
  drawPlayheadLine,
  drawSeriesMissingStrip,
  drawTimeAxis,
  resolveKitTokens,
} from '../../draw/kit.ts';
import { rgbaColor } from '../../shell/appearance/color.ts';
import { toArtSlotState, toFlagSlotState } from '../art/adapters.ts';
import { collectArtUrls, type ArtUrlInput } from '../art/provider.ts';
import type { FamilyTheme } from '../contract.ts';
import { formatDateOnly } from '../valueFormat.ts';
import { parseChartInspect, sampleSeriesValueAt } from './hitTest.ts';
import { formatChartAxisValue, type TimeChartLayout } from './layout.ts';
import type { TimeChartModel, TimeChartSeriesModel } from './model.ts';

export function collectVisibleChartArt(model: TimeChartModel, layout: TimeChartLayout): string[] {
  const items: ArtUrlInput[] = [];
  for (const label of layout.endLabels) {
    items.push(label.flag, label.icon);
  }
  for (const m of layout.markers) {
    if (m.kind === 'flag') items.push(m.marker.flag);
    if (m.kind === 'icon') items.push(m.marker.icon);
  }
  if (model.legend) {
    for (const item of model.legend) items.push(item.flag);
  }
  if (model.overlays) {
    for (const ov of model.overlays) items.push(ov.icon);
  }
  return collectArtUrls(items);
}

export function buildLineSubpaths(
  series: TimeChartSeriesModel,
  layout: TimeChartLayout,
  until: number,
): { readonly subpaths: readonly (readonly [number, number][])[]; readonly end: readonly [number, number] | null } {
  const step = series.step === true;
  const scale = series.valueScale ?? 1;
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
        const y = layout.toY(run.value * scale);
        if (!cur.length) {
          cur.push([x0, y]);
          if (step && x1 > x0) cur.push([x1, y]);
        } else if (step) {
          cur.push([x0, cur[cur.length - 1]![1]], [x0, y]);
          if (x1 > x0) cur.push([x1, y]);
        } else {
          cur.push([x0, y]);
        }
        end = [step && x1 > x0 ? x1 : x0, y];
      }
      if (cur.length) subpaths.push(cur);
    } else {
      let cur: [number, number][] = [];
      let prevLastHour = -Infinity;
      const pushPoint = (h: number, v: number) => {
        const x = layout.toX(Math.max(layout.xDomain[0], Math.min(layout.xDomain[1], h)));
        const y = layout.toY(v * scale);
        if (!cur.length) {
          cur.push([x, y]);
        } else {
          const prev = cur[cur.length - 1]!;
          if (prev[0] === x && prev[1] === y) return;
          if (step) cur.push([x, prev[1]], [x, y]);
          else cur.push([x, y]);
        }
        end = [x, y];
      };
      for (const b of bounded.buckets) {
        if (!b || b.lastHour < layout.xDomain[0] || b.firstHour > layout.xDomain[1]) continue;
        const disconnected = cur.length > 0 && (b.firstBreak || b.firstHour > prevLastHour + Math.max(1, bounded.binHours));
        if (disconnected) {
          subpaths.push(cur);
          cur = [];
        }
        if (step) {
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
        } else {
          pushPoint(b.firstHour, b.firstValue);
          const loEnd = Math.min(b.firstValue, b.lastValue);
          const hiEnd = Math.max(b.firstValue, b.lastValue);
          const hasLower = b.minValue < loEnd;
          const hasHigher = b.maxValue > hiEnd;
          if (hasLower && hasHigher) {
            if (b.minHour <= b.maxHour) {
              pushPoint(b.minHour, b.minValue);
              pushPoint(b.maxHour, b.maxValue);
            } else {
              pushPoint(b.maxHour, b.maxValue);
              pushPoint(b.minHour, b.minValue);
            }
          } else if (hasLower) {
            pushPoint(b.minHour, b.minValue);
          } else if (hasHigher) {
            pushPoint(b.maxHour, b.maxValue);
          }
          const endHour = b.lastValueHour >= b.firstHour ? b.lastValueHour : b.lastHour;
          if (endHour > b.firstHour || b.lastValue !== b.firstValue) {
            pushPoint(endHour, b.lastValue);
          }
        }
        prevLastHour = b.lastHour;
      }
      if (cur.length) subpaths.push(cur);
    }
    const headHour = series.headHour ?? until;
    if (series.headValue != null && Number.isFinite(series.headValue) && headHour >= layout.xDomain[0] && headHour <= layout.xDomain[1]) {
      const hx = layout.toX(headHour);
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
  inspectKey: string | null | undefined,
  dpr: number,
): void {
  const theme = themeInput ?? DEFAULT_FAMILY_THEME;
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
  const textSecondary = tokens.textSecondary;

  if (layout.legendTop !== null && model.legend && model.legend.length > 0) {
    const legend = model.legend;
    const isLineLegend = model.mode === 'lines';
    let ordinaryLen = legend.length;
    while (ordinaryLen > 0 && legend[ordinaryLen - 1]!.hatch) ordinaryLen--;
    const trailing = ordinaryLen < legend.length
      ? legend.slice(ordinaryLen).map(item => ({
          label: item.label,
          color: item.color,
          hatch: true,
          muted: false,
        }))
      : undefined;
    drawLegendStrip(ctx, layout.left, layout.legendTop, layout.width - layout.left, 14, {
      entries: {
        length: ordinaryLen,
        at: i => {
          const item = legend[i]!;
          return {
            label: item.label,
            color: item.color,
            ...(isLineLegend && !item.hatch ? { line: true } : {}),
            dash: item.dash,
            hatch: item.hatch,
            flag: item.flag ? toFlagSlotState(item.flag.art, item.flag.tag ?? item.flag.label) : undefined,
            muted: false,
          };
        },
        ...(trailing ? { trailing } : {}),
      },
      entryGap: 14,
    }, theme);
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
    ctx.fillText(formatChartAxisValue(v, model.yAxis.format, layout.yTicks), layout.left - 8, y);
    ctx.restore();
  }

  for (const g of layout.gaps) {
    drawGapBand(ctx, g.x0, layout.top, g.x1 - g.x0, layout.plotHeight, theme);
  }

  if (model.mode === 'columns') {
    const hoveredColIdx = parsedInspect?.kind === 'month' || parsedInspect?.kind === 'fold' ? parsedInspect.index : -1;
    for (const col of layout.columns) {
      if (col.index === hoveredColIdx) {
        ctx.save();
        ctx.fillStyle = theme.text;
        ctx.globalAlpha = 0.055;
        ctx.fillRect(col.x, layout.top, col.width, layout.plotHeight);
        ctx.restore();
      }
      if (col.kind === 'fold') {
        drawGapBand(ctx, col.barX, layout.top, col.barWidth, layout.plotHeight, theme);
        continue;
      }
      const m = col.month!;
      if (m.status !== 'recorded') {
        drawGapBand(ctx, col.barX, layout.top, col.barWidth, layout.plotHeight, theme);
        continue;
      }
      let yPos = layout.toY(0);
      let yNeg = layout.toY(0);
      for (const seg of m.segments) {
        if (seg.value === 0) continue;
        const h = Math.abs(layout.toY(0) - layout.toY(seg.value));
        ctx.save();
        ctx.globalAlpha = m.partial ? 0.55 : 1;
        ctx.fillStyle = seg.color;
        if (seg.value > 0) {
          ctx.fillRect(col.barX, yPos - h, col.barWidth, h);
          yPos -= h;
        } else {
          ctx.fillRect(col.barX, yNeg, col.barWidth, h);
          yNeg += h;
        }
        ctx.restore();
      }
    }
  } else if (model.mode === 'stack' && model.stack && model.stack.hours.length > 0) {
    const { hours, bands, total, step } = model.stack;
    let count = 0;
    while (count < hours.length && hours[count]! <= model.hour) count++;
    if (count > 0 && step) {
      const gapBetween = (a: number, b: number) =>
        model.gaps?.some(g => g.firstHour > a && g.firstHour < b) ?? false;
      const isNullAt = (i: number) => (total?.values[i] == null) && bands.every(b => b.values[i] == null);
      const runs: { readonly start: number; readonly end: number; readonly endHour: number }[] = [];
      let runStart = -1;
      for (let i = 0; i < count; i++) {
        if (isNullAt(i)) {
          if (runStart >= 0) {
            runs.push({ start: runStart, end: i - 1, endHour: hours[i]! });
            runStart = -1;
          }
          continue;
        }
        if (runStart >= 0 && gapBetween(hours[i - 1]!, hours[i]!)) {
          const hitGap = model.gaps!.find(g => g.firstHour > hours[i - 1]! && g.firstHour < hours[i]!);
          runs.push({ start: runStart, end: i - 1, endHour: hitGap ? hitGap.firstHour : hours[i - 1]! });
          runStart = i;
        } else if (runStart < 0) {
          runStart = i;
        }
      }
      if (runStart >= 0) {
        const trailingGap = model.gaps?.find(g => g.firstHour > hours[count - 1]! && g.firstHour <= model.hour);
        runs.push({ start: runStart, end: count - 1, endHour: trailingGap ? trailingGap.firstHour : Math.min(layout.xDomain[1], model.hour) });
      }
      if (runs.length > 0) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(layout.left, layout.top - 2, layout.plotWidth + 1, layout.plotHeight + 4);
        ctx.clip();
        for (const run of runs) {
          const len = run.end - run.start + 1;
          const posAcc = new Float64Array(len);
          const negAcc = new Float64Array(len);
          const endX = layout.toX(Math.min(run.endHour, layout.xDomain[1], model.hour));
          const xEndAt = (k: number) => (k + 1 < len ? layout.toX(hours[run.start + k + 1]!) : endX);
          for (const band of bands) {
            const botVals = new Float64Array(len);
            const topVals = new Float64Array(len);
            const sign = new Int8Array(len);
            for (let k = 0; k < len; k++) {
              const v = band.values[run.start + k];
              if (v == null || !Number.isFinite(v) || v === 0) continue;
              if (v > 0) {
                botVals[k] = posAcc[k]!;
                topVals[k] = posAcc[k]! + v;
                posAcc[k] = topVals[k]!;
                sign[k] = 1;
              } else {
                botVals[k] = negAcc[k]!;
                topVals[k] = negAcc[k]! + v;
                negAcc[k] = topVals[k]!;
                sign[k] = -1;
              }
            }
            let k = 0;
            while (k < len) {
              const s = sign[k]!;
              if (s === 0) { k++; continue; }
              const subStart = k;
              while (k < len && sign[k] === s) k++;
              const subEnd = k - 1;
              const subEndX = xEndAt(subEnd);
              ctx.beginPath();
              for (let j = subStart; j <= subEnd; j++) {
                const x = layout.toX(hours[run.start + j]!);
                const y = layout.toY(topVals[j]!);
                if (j === subStart) ctx.moveTo(x, y);
                else {
                  ctx.lineTo(x, layout.toY(topVals[j - 1]!));
                  ctx.lineTo(x, y);
                }
              }
              if (subEndX > layout.toX(hours[run.start + subEnd]!)) {
                ctx.lineTo(subEndX, layout.toY(topVals[subEnd]!));
                ctx.lineTo(subEndX, layout.toY(botVals[subEnd]!));
              }
              for (let j = subEnd; j >= subStart; j--) {
                const x = layout.toX(hours[run.start + j]!);
                const y = layout.toY(botVals[j]!);
                if (j < subEnd) {
                  ctx.lineTo(layout.toX(hours[run.start + j + 1]!), y);
                }
                ctx.lineTo(x, y);
              }
              ctx.closePath();
              ctx.fillStyle = band.color;
              ctx.globalAlpha = (band.opacity ?? 0.78) * (hoveredSeriesId && hoveredSeriesId !== band.id ? 0.35 : 1);
              ctx.fill();
              ctx.globalAlpha = 0.9;
              ctx.strokeStyle = theme.surface;
              ctx.lineWidth = 0.8;
              ctx.stroke();
            }
          }
          if (total) {
            ctx.globalAlpha = 1;
            ctx.strokeStyle = total.color ?? theme.text;
            ctx.lineWidth = 1.6;
            ctx.lineJoin = 'round';
            let started = false;
            let prevY = 0;
            let lastValidK = -1;
            for (let k = 0; k < len; k++) {
              const tv = total.values[run.start + k];
              if (tv == null || !Number.isFinite(tv)) {
                if (started) {
                  const breakX = layout.toX(hours[run.start + k]!);
                  if (breakX > layout.toX(hours[run.start + lastValidK]!)) ctx.lineTo(breakX, prevY);
                  ctx.stroke();
                  started = false;
                }
                continue;
              }
              const x = layout.toX(hours[run.start + k]!);
              const y = layout.toY(tv);
              if (!started) { ctx.beginPath(); ctx.moveTo(x, y); started = true; }
              else { ctx.lineTo(x, prevY); ctx.lineTo(x, y); }
              prevY = y;
              lastValidK = k;
            }
            if (started) {
              if (endX > layout.toX(hours[run.start + lastValidK]!)) {
                ctx.lineTo(endX, prevY);
              }
              ctx.stroke();
            }
          }
        }
        if (layout.yDomain[0] < 0) {
          const zeroY = Math.round(layout.toY(0)) + 0.5;
          ctx.globalAlpha = 1;
          ctx.strokeStyle = tokens.hairlineStrong;
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(layout.left, zeroY);
          ctx.lineTo(layout.left + layout.plotWidth, zeroY);
          ctx.stroke();
        }
        ctx.restore();
      }
    } else if (count > 0 && bands.length > 0) {
      const segments: [number, number][] = [];
      let segStart = -1;
      for (let i = 0; i < count; i++) {
        const valid = bands.every(band => {
          const v = band.values[i];
          return v !== null && v !== undefined && Number.isFinite(v);
        });
        if (valid) {
          if (segStart < 0) segStart = i;
        } else if (segStart >= 0) {
          segments.push([segStart, i - 1]);
          segStart = -1;
        }
      }
      if (segStart >= 0) segments.push([segStart, count - 1]);

      if (segments.length > 0) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(layout.left, layout.top - 2, layout.plotWidth + 1, layout.plotHeight + 4);
        ctx.clip();
        for (const [start, end] of segments) {
          const len = end - start + 1;
          const acc = new Float64Array(len);
          for (let bandIdx = 0; bandIdx < bands.length; bandIdx++) {
            const band = bands[bandIdx]!;
            const topVals = new Float64Array(len);
            for (let k = 0; k < len; k++) {
              topVals[k] = acc[k]! + (band.values[start + k] ?? 0);
            }
            ctx.beginPath();
            if (len === 1) {
              const x0 = layout.toX(hours[start]!);
              const x1 = Math.min(layout.playheadX, x0 + 1);
              const yTop = layout.toY(topVals[0]!);
              const yBot = layout.toY(acc[0]!);
              ctx.moveTo(x0, yTop);
              ctx.lineTo(x1, yTop);
              ctx.lineTo(x1, yBot);
              ctx.lineTo(x0, yBot);
            } else {
              for (let k = 0; k < len; k++) {
                const x = layout.toX(hours[start + k]!);
                const y = layout.toY(topVals[k]!);
                if (k === 0) ctx.moveTo(x, y);
                else ctx.lineTo(x, y);
              }
              for (let k = len - 1; k >= 0; k--) {
                ctx.lineTo(layout.toX(hours[start + k]!), layout.toY(acc[k]!));
              }
            }
            ctx.closePath();
            ctx.fillStyle = band.color;
            ctx.globalAlpha = (band.opacity ?? 0.78) * (hoveredSeriesId && hoveredSeriesId !== band.id ? 0.35 : 1);
            ctx.fill();
            if (bandIdx < bands.length - 1 && len > 1) {
              ctx.beginPath();
              for (let k = 0; k < len; k++) {
                const x = layout.toX(hours[start + k]!);
                const y = layout.toY(topVals[k]!);
                if (k === 0) ctx.moveTo(x, y);
                else ctx.lineTo(x, y);
              }
              ctx.globalAlpha = 0.9;
              ctx.strokeStyle = theme.surface;
              ctx.lineWidth = 0.8;
              ctx.stroke();
            }
            acc.set(topVals);
          }
        }
        if (total) {
          ctx.globalAlpha = 1;
          ctx.strokeStyle = total.color ?? theme.text;
          ctx.lineWidth = 1.6;
          ctx.lineJoin = 'round';
          let open = false;
          for (let i = 0; i < count; i++) {
            const tv = total.values[i];
            if (tv == null || !Number.isFinite(tv)) {
              if (open) { ctx.stroke(); open = false; }
              continue;
            }
            const x = layout.toX(hours[i]!);
            const y = layout.toY(tv);
            if (!open) { ctx.beginPath(); ctx.moveTo(x, y); open = true; }
            else ctx.lineTo(x, y);
          }
          if (open) ctx.stroke();
        }
        ctx.restore();
      }
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
          const topStop = rgbaColor(s.color, 0.26 * opacity, '');
          if (topStop) {
            grad.addColorStop(0, topStop);
            grad.addColorStop(1, rgbaColor(s.color, 0.02 * opacity, s.color));
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
          const dash = s.dash?.length ? s.dash : s.role === 'ceiling' ? [5, 4] : null;
          if (dash) ctx.setLineDash([...dash]);
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
    if (label.flag && label.flag.art.status !== 'noArtExpected') {
      drawFlagSlot(ctx, cursorX, label.y - 6, toFlagSlotState(label.flag.art, label.flag.tag ?? label.flag.label), theme, { width: 18, height: 12 });
      cursorX += 24;
    } else if (label.icon && label.icon.art.status !== 'noArtExpected') {
      drawArtSlot(ctx, cursorX, label.y - 7.5, 16, 15, toArtSlotState(label.icon.art, label.icon.label), theme, { radius: 0 });
      cursorX += 22;
    }
    if (label.name) {
      ctx.font = labelNameFont;
      ctx.fillStyle = label.valueMuted ? theme.muted : textSecondary;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(label.name, cursorX, label.y + 0.5);
      cursorX += textWidth(ctx, label.name) + 6;
    }
    ctx.font = labelValueFont;
    ctx.fillStyle = label.valueMuted ? theme.muted : (label.valueColor ?? theme.text);
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(label.value, cursorX, label.y + 0.5);
    ctx.restore();
  }

  if (model.overlays && layout.plotHeight >= 72 && layout.plotWidth >= 240) {
    for (const ov of model.overlays) {
      ctx.save();
      ctx.font = overlayFont;
      const tw = textWidth(ctx, ov.text);
      const ox = layout.left + layout.plotWidth - 8 - tw;
      const oy = ov.position === 'top-right' ? layout.top + 12 : layout.baseY - 12;
      const arrowW = ov.arrow ? 12 : 0;
      const padX = 5;
      const plateLeft = Math.max(layout.left + 2, ox - arrowW - padX);
      const plateRight = Math.min(layout.left + layout.plotWidth - 2, ox + tw + padX);
      if (plateRight > plateLeft) {
        ctx.fillStyle = theme.surface;
        ctx.beginPath();
        ctx.roundRect(plateLeft, oy - 8, plateRight - plateLeft, 16, 3);
        ctx.fill();
      }
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
    if (m.kind === 'flag' && m.marker.flag) {
      drawFlagSlot(ctx, m.x - 8, m.y - 5, toFlagSlotState(m.marker.flag.art, m.marker.flag.tag ?? m.marker.flag.label), theme, { width: 16, height: 10 });
    } else if (m.kind === 'icon' && m.marker.icon) {
      drawArtSlot(ctx, m.x - 9, m.y - 9, 18, 18, toArtSlotState(m.marker.icon.art, m.marker.icon.label), theme, { radius: 0 });
    } else {
      ctx.save();
      ctx.strokeStyle = m.marker.tickColor ?? tokens.markerTick;
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

  const hoverInPlot = hoverHour != null && model.mode !== 'columns' && hoverHour >= layout.xDomain[0] && hoverHour <= model.hour;

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
      } else if (col.month?.status === 'recorded' && col.month.missingParts && col.month.missingParts.length > 0) {
        drawSeriesMissingStrip(ctx, col.barX, layout.axisY + 1, col.barWidth, theme.hatch, theme, 4);
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
    drawDateTab(ctx, layout.dateTabText, layout.dateTabX, layout.axisY + 3, theme, 'playhead', {
      left: layout.left,
      right: layout.width - 6,
    });
  } else {
    if (hoverInPlot) {
      const xh = layout.toX(hoverHour);
      drawCrosshairLine(ctx, xh, layout.top - 2, layout.axisY, theme);
      if (model.mode === 'stack' && model.stack?.total && model.stack.hours.length > 0) {
        let at = -1;
        for (let i = 0; i < model.stack.hours.length; i++) {
          if (model.stack.hours[i]! <= hoverHour) at = i;
          else break;
        }
        const tv = at >= 0 ? model.stack.total.values[at] : null;
        if (tv != null && Number.isFinite(tv)) {
          ctx.save();
          ctx.beginPath();
          ctx.arc(xh, layout.toY(tv), 3.4, 0, Math.PI * 2);
          ctx.fillStyle = model.stack.total.color ?? theme.text;
          ctx.fill();
          ctx.strokeStyle = theme.surface;
          ctx.lineWidth = 1.8;
          ctx.stroke();
          ctx.restore();
        }
      }
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
    }
    drawTimeAxis(ctx, layout.rawAxisTicks, {
      plotLeft: layout.left,
      plotRight: layout.left + layout.plotWidth,
      startHour: layout.xDomain[0],
      endHour: layout.xDomain[1],
      toX: layout.toX,
      axisTop: layout.axisY,
      playhead: model.hour,
      playheadLabel: layout.dateTabText,
      hover: hoverInPlot ? hoverHour : null,
      hoverLabel: hoverInPlot ? formatDateOnly(hoverHour) : undefined,
      playheadLineTop: null,
      hairline: 'none',
      tickColor: tokens.zeroLine,
      firstLabelOnly: layout.small,
      tabBounds: { left: layout.left, right: layout.width - 6 },
      tabY: layout.axisY + 3,
      labelY: layout.axisY + 12,
      theme,
    });
  }
}
