import { ellipsise } from '../../charts/labels.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import {
  drawArtSlot,
  drawBottomTimeAxis,
  drawChevron,
  drawChildRowTint,
  drawFlagSlot,
  drawGapBand,
  drawMissingMark,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  drawScrollFade,
  drawSparkline,
  drawSplitBar,
  drawSplitWarBar,
  drawStatusMark,
  drawThresholdMeter,
  drawTimelineSpanBar,
  drawTrackBar,
  FLAG_LOSER_ALPHA,
  FLAG_SLOT_HEIGHT,
  FLAG_SLOT_WIDTH,
  resolveKitTokens,
  TIMELINE_AXIS_HEIGHT,
  type ArtSlotState,
  type FlagSlotState,
} from '../../draw/kit.ts';
import { drawHatchRect } from '../valueCanvas.ts';
import { toArtSlotState, toFlagSlotState } from '../art/adapters.ts';
import type { ArtHandle, BoundedReadView, FamilyTheme } from '../contract.ts';
import {
  BOTTOM_METERS_HEADER_HEIGHT,
  HEADER_CARD_HEIGHT,
  METER_SECTION_HEADER_HEIGHT,
  type TimelineLayout,
} from './layout.ts';
import {
  isLaneExpanded,
  type TimelineCountryChip,
  type TimelineHeaderCard,
  type TimelineHitTarget,
  type TimelineLaneModel,
  type TimelineMeterRow,
  type TimelineModel,
  type TimelineSeriesModel,
  type TimelineSpanModel,
} from './model.ts';

export interface DrawFamilyTimelineInput {
  readonly model: TimelineModel;
  readonly layout: TimelineLayout;
  readonly readView: BoundedReadView;
  readonly theme: FamilyTheme;
  readonly scrollTop: number;
  readonly expanded: Readonly<Record<string, boolean>>;
  readonly inspectedKey?: string | null;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
}

function artSlotFromReadView(
  readView: BoundedReadView,
  handle: ArtHandle | null | undefined,
  missing: boolean | undefined,
  resolvedName?: string,
): ArtSlotState {
  if (!handle) return missing ? { kind: 'missing', resolvedName } : { kind: 'none' };
  return toArtSlotState(readView.art(handle), resolvedName);
}

function flagSlotFromReadView(
  readView: BoundedReadView,
  handle: ArtHandle | null | undefined,
  tag?: string,
): FlagSlotState {
  if (!handle) return { kind: 'missing', tag };
  return toFlagSlotState(readView.art(handle), tag);
}

function drawHeaderCard(
  ctx: Canvas2D,
  card: TimelineHeaderCard,
  width: number,
  readView: BoundedReadView,
  theme: FamilyTheme,
): void {
  const padX = 12;
  const midY = HEADER_CARD_HEIGHT / 2;
  let left = padX;
  const portraitState = artSlotFromReadView(readView, card.portrait, card.portraitMissing, card.name);
  if (portraitState.kind !== 'none') {
    drawArtSlot(ctx, left, Math.round(midY - 17), 34, 34, portraitState, theme, { radius: 4 });
    left += 42;
  }

  let statsWidth = 0;
  const statGap = 14;
  const statWidths: number[] = [];
  for (const stat of card.stats) {
    ctx.font = `600 12.5px ${theme.font}`;
    const vw = textWidth(ctx, stat.value);
    ctx.font = `400 10.5px ${theme.font}`;
    const lw = textWidth(ctx, stat.label);
    const w = Math.max(vw, lw);
    statWidths.push(w);
    statsWidth += w + statGap;
  }
  const showStats = width - left - statsWidth > 120;
  const nameMaxW = Math.max(60, showStats ? width - left - statsWidth - 12 : width - left - padX);

  ctx.save();
  ctx.font = `600 13px ${theme.font}`;
  ctx.fillStyle = theme.text;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(ellipsise(card.name, nameMaxW, s => textWidth(ctx, s)), left, midY - 7);

  let subLeft = left;
  if (card.flag || card.flagTag) {
    drawFlagSlot(ctx, subLeft, Math.round(midY + 2), flagSlotFromReadView(readView, card.flag, card.flagTag), theme, { width: 16, height: 11 });
    subLeft += 21;
  }
  if (card.subtitle) {
    ctx.font = `400 11px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(ellipsise(card.subtitle, Math.max(20, nameMaxW - (subLeft - left)), s => textWidth(ctx, s)), subLeft, midY + 8);
  }

  if (showStats) {
    let rx = width - padX;
    for (let i = card.stats.length - 1; i >= 0; i--) {
      const stat = card.stats[i]!;
      const w = statWidths[i]!;
      ctx.textAlign = 'right';
      ctx.font = `600 12.5px ${theme.font}`;
      ctx.fillStyle = stat.tone === 'positive'
        ? theme.green
        : stat.tone === 'negative'
          ? theme.red
          : stat.tone === 'muted'
            ? theme.muted
            : theme.text;
      ctx.fillText(stat.value, rx, midY - 6);
      ctx.font = `400 10px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.fillText(stat.label, rx, midY + 8);
      rx -= w + statGap;
    }
  }
  ctx.restore();
  drawRowHairline(ctx, 0, width, HEADER_CARD_HEIGHT - 1, theme, 'strong');
}

function drawMatchupSideInline(
  ctx: Canvas2D,
  x: number,
  midY: number,
  maxRight: number,
  side: { readonly lead: TimelineCountryChip; readonly allies: readonly TimelineCountryChip[]; readonly dimmed?: boolean },
  readView: BoundedReadView,
  theme: FamilyTheme,
  fontSize: number,
  weight: number,
): number {
  let cursor = x;
  const dimmed = side.dimmed === true || side.lead.dimmed === true;
  const flagW = fontSize >= 12 ? 18 : 16;
  const flagH = fontSize >= 12 ? 12 : 10;
  if (cursor + flagW <= maxRight) {
    drawFlagSlot(ctx, cursor, Math.round(midY - flagH / 2), flagSlotFromReadView(readView, side.lead.flag, side.lead.tag), theme, {
      width: flagW,
      height: flagH,
      dimmed,
    });
    cursor += flagW + 5;
  }
  ctx.save();
  ctx.font = `${weight} ${fontSize}px ${theme.font}`;
  ctx.fillStyle = dimmed ? theme.muted : theme.text;
  if (dimmed) ctx.globalAlpha = 0.65;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  const allyReserve = side.allies.length > 0 ? Math.min(side.allies.length, 4) * 16 + 4 : 0;
  const availName = Math.max(0, maxRight - cursor - allyReserve);
  if (availName >= 14) {
    const fitted = ellipsise(side.lead.name, availName, s => textWidth(ctx, s));
    ctx.fillText(fitted, cursor, midY + 0.5);
    cursor += textWidth(ctx, fitted) + 4;
  }
  ctx.restore();

  const smallW = 14;
  const smallH = 9;
  for (const ally of side.allies) {
    if (cursor + smallW > maxRight) break;
    drawFlagSlot(ctx, cursor, Math.round(midY - smallH / 2), flagSlotFromReadView(readView, ally.flag, ally.tag), theme, {
      width: smallW,
      height: smallH,
      dimmed: dimmed || ally.dimmed === true,
    });
    cursor += smallW + 2;
  }
  return cursor;
}

function drawLaneLabel(
  ctx: Canvas2D,
  lane: TimelineLaneModel,
  y: number,
  h: number,
  labelWidth: number,
  expanded: boolean,
  readView: BoundedReadView,
  theme: FamilyTheme,
): void {
  const padX = 10;
  const indent = (lane.depth ?? 0) * 16;
  let left = padX + indent;
  const right = labelWidth - 6;
  const midY = y + h / 2;

  if (lane.kind === 'section-header') {
    ctx.save();
    ctx.font = `600 10.5px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(ellipsise(lane.label, Math.max(10, right - left), s => textWidth(ctx, s)), left, midY);
    ctx.restore();
    return;
  }

  if (lane.expandable) {
    drawChevron(ctx, left, midY, expanded, theme.muted, 10);
    left += 14;
  }

  if (lane.kind === 'matchup' && lane.matchup) {
    const m = lane.matchup;
    const hasTitle = Boolean(m.title && m.title.trim().length > 0);
    if (hasTitle) {
      ctx.save();
      ctx.font = `600 12px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(ellipsise(m.title!, Math.max(10, right - left), s => textWidth(ctx, s)), left, y + 11);
      ctx.restore();
      const row2Y = y + 25;
      const half = Math.floor((right - left - 14) / 2);
      const afterAtt = drawMatchupSideInline(ctx, left, row2Y, left + half, m.attackers, readView, theme, 11, 500);
      ctx.save();
      ctx.font = `400 11px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText('\u2013', afterAtt + 1, row2Y + 0.5);
      ctx.restore();
      drawMatchupSideInline(ctx, afterAtt + 11, row2Y, right, m.defenders, readView, theme, 11, 500);
    } else {
      const row1Y = lane.subtitle ? y + 12 : midY;
      const half = Math.floor((right - left - 14) / 2);
      const afterAtt = drawMatchupSideInline(ctx, left, row1Y, left + half, m.attackers, readView, theme, 12, 500);
      ctx.save();
      ctx.font = `400 11.5px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText('\u2013', afterAtt + 1, row1Y + 0.5);
      ctx.restore();
      drawMatchupSideInline(ctx, afterAtt + 11, row1Y, right, m.defenders, readView, theme, 12, 500);
      if (lane.subtitle) {
        ctx.save();
        ctx.font = `400 10.5px ${theme.font}`;
        ctx.fillStyle = theme.muted;
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(ellipsise(lane.subtitle, Math.max(10, right - left), s => textWidth(ctx, s)), left, y + 26);
        ctx.restore();
      }
    }
    return;
  }

  if (lane.kind === 'participant') {
    const dimmed = lane.dimmed === true || lane.flagDimmed === true;
    const fw = 16;
    const fh = 10;
    drawFlagSlot(ctx, left, Math.round(midY - fh / 2), flagSlotFromReadView(readView, lane.flag, lane.flagTag), theme, {
      width: fw,
      height: fh,
      dimmed,
    });
    left += fw + 6;
    ctx.save();
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.font = `500 11.5px ${theme.font}`;
    const subReserve = lane.subtitle ? Math.min(82, Math.floor((right - left) * 0.42)) : 0;
    const nameAvail = Math.max(10, right - left - subReserve);
    const fittedName = ellipsise(lane.label, nameAvail, s => textWidth(ctx, s));
    ctx.fillStyle = dimmed ? theme.muted : theme.text;
    if (dimmed) ctx.globalAlpha = 0.65;
    ctx.fillText(fittedName, left, midY + 0.5);
    if (lane.subtitle) {
      const nameW = textWidth(ctx, fittedName);
      const subX = left + nameW + 6;
      const subAvail = right - subX;
      if (subAvail >= 24) {
        ctx.font = `400 10.5px ${theme.font}`;
        ctx.fillStyle = theme.muted;
        ctx.fillText(ellipsise(lane.subtitle, subAvail, s => textWidth(ctx, s)), subX, midY + 0.5);
      }
    }
    ctx.restore();
    return;
  }

  if (lane.flag || lane.flagTag) {
    const fw = FLAG_SLOT_WIDTH;
    const fh = FLAG_SLOT_HEIGHT;
    drawFlagSlot(ctx, left, Math.round(midY - fh / 2), flagSlotFromReadView(readView, lane.flag, lane.flagTag), theme, {
      width: fw,
      height: fh,
      dimmed: lane.flagDimmed,
    });
    left += fw + 7;
  } else if (lane.icon || lane.iconMissing) {
    const sz = lane.iconSize ?? (h >= 34 ? 22 : 16);
    const st = artSlotFromReadView(readView, lane.icon, lane.iconMissing, lane.label);
    if (st.kind !== 'none') {
      drawArtSlot(ctx, left, Math.round(midY - sz / 2), sz, sz, st, theme, { radius: 3 });
      left += sz + 7;
    }
  } else if (lane.colorSwatch) {
    const sz = 10;
    ctx.save();
    ctx.fillStyle = lane.colorSwatch;
    ctx.beginPath();
    ctx.roundRect(left, Math.round(midY - sz / 2), sz, sz, 2.5);
    ctx.fill();
    ctx.restore();
    left += sz + 7;
  }

  const avail = Math.max(10, right - left);
  ctx.save();
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  const titleWeight = lane.kind === 'group-header' ? 600 : 500;
  const titleSize = lane.kind === 'group-header' ? 12 : 12;
  ctx.font = `${titleWeight} ${titleSize}px ${theme.font}`;
  ctx.fillStyle = lane.dimmed ? theme.muted : theme.text;
  if (lane.subtitle && h >= 32) {
    ctx.fillText(ellipsise(lane.label, avail, s => textWidth(ctx, s)), left, y + 12);
    ctx.font = `400 11px ${theme.font}`;
    ctx.fillStyle = lane.subtitleColor ?? theme.muted;
    ctx.fillText(ellipsise(lane.subtitle, avail, s => textWidth(ctx, s)), left, y + 26);
  } else if (lane.subtitle && lane.kind === 'group-header') {
    const nameFit = ellipsise(lane.label, avail * 0.68, s => textWidth(ctx, s));
    ctx.fillText(nameFit, left, midY);
    const nw = textWidth(ctx, nameFit);
    const subAvail = avail - nw - 8;
    if (subAvail >= 24) {
      ctx.font = `400 11px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.fillText(ellipsise(lane.subtitle, subAvail, s => textWidth(ctx, s)), left + nw + 8, midY + 0.5);
    }
  } else {
    ctx.fillText(ellipsise(lane.label, avail, s => textWidth(ctx, s)), left, midY);
  }
  ctx.restore();
}

function drawSpanMarkers(
  ctx: Canvas2D,
  span: TimelineSpanModel,
  barY: number,
  barH: number,
  layout: TimelineLayout,
  readView: BoundedReadView,
  theme: FamilyTheme,
): void {
  if (!span.markers?.length) return;
  for (const m of span.markers) {
    const mx = layout.toX(m.hour);
    if (mx < layout.plotLeft - 8 || mx > layout.plotRight + 8) continue;
    if (m.kind === 'surrender') {
      const iconState = artSlotFromReadView(readView, m.icon, false, m.label);
      if (iconState.kind === 'ready') {
        drawArtSlot(ctx, Math.round(mx - 7), Math.round(barY + barH / 2 - 7), 14, 14, iconState, theme, { radius: 0 });
      } else {
        ctx.save();
        ctx.strokeStyle = theme.text;
        ctx.fillStyle = theme.text;
        ctx.lineWidth = 1.2;
        const fy = Math.round(barY + barH / 2 - 5);
        ctx.beginPath();
        ctx.moveTo(mx, fy + 10);
        ctx.lineTo(mx, fy);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(mx, fy);
        ctx.lineTo(mx + 7, fy + 2.5);
        ctx.lineTo(mx, fy + 5);
        ctx.closePath();
        ctx.fill();
        ctx.restore();
      }
    } else if (m.kind === 'diamond') {
      const cy = barY + barH / 2;
      const r = 4.5;
      ctx.save();
      ctx.fillStyle = m.color ?? theme.accent;
      ctx.beginPath();
      ctx.moveTo(mx, cy - r);
      ctx.lineTo(mx + r, cy);
      ctx.lineTo(mx, cy + r);
      ctx.lineTo(mx - r, cy);
      ctx.closePath();
      ctx.fill();
      ctx.restore();
    }
  }
}

function drawLanePlot(
  ctx: Canvas2D,
  lane: TimelineLaneModel,
  y: number,
  h: number,
  expanded: boolean,
  layout: TimelineLayout,
  model: TimelineModel,
  readView: BoundedReadView,
  theme: FamilyTheme,
): void {
  const { plotLeft, plotRight, toX } = layout;
  const midY = y + h / 2;

  if (lane.kind === 'group-header' && lane.ruleSpan && lane.ruleColor) {
    const x0 = Math.max(plotLeft, toX(lane.ruleSpan[0]));
    const x1 = Math.min(plotRight, toX(lane.ruleSpan[1]));
    if (x1 > x0) {
      ctx.save();
      ctx.fillStyle = lane.ruleColor;
      ctx.beginPath();
      ctx.roundRect(x0, Math.round(midY - 2), Math.max(2, x1 - x0), 4, 2);
      ctx.fill();
      ctx.restore();
    }
    return;
  }

  if (lane.kind === 'diverging-bars' && lane.diverging) {
    const { bins, maxCount } = lane.diverging;
    const baseline = Math.round(midY) + 0.5;
    const maxBarH = Math.max(4, Math.floor(h / 2) - 4);
    drawRowHairline(ctx, plotLeft, plotRight, baseline, theme, 'normal');
    if (maxCount > 0) {
      for (const bin of bins) {
        const x0 = Math.max(plotLeft, toX(bin.firstHour));
        const x1 = Math.min(plotRight, toX(bin.lastHour));
        const bw = Math.max(2, x1 - x0 - 1);
        if (x1 < plotLeft || x0 > plotRight) continue;
        if (bin.won > 0) {
          const bh = Math.max(2, Math.round((bin.won / maxCount) * maxBarH));
          ctx.fillStyle = theme.green;
          ctx.fillRect(x0, baseline - bh, bw, bh);
        }
        if (bin.lost > 0) {
          const bh = Math.max(2, Math.round((bin.lost / maxCount) * maxBarH));
          ctx.fillStyle = theme.red;
          ctx.fillRect(x0, baseline + 1, bw, bh);
        }
      }
    }
    return;
  }

  if (lane.kind === 'markers' && lane.pointMarkers) {
    drawRowHairline(ctx, plotLeft, plotRight, midY, theme, 'normal');
    let lastLabelRight = -Infinity;
    for (const m of lane.pointMarkers) {
      const mx = toX(m.hour);
      if (mx < plotLeft || mx > plotRight) continue;
      const r = 4.5;
      ctx.save();
      ctx.fillStyle = m.color ?? theme.accent;
      ctx.beginPath();
      ctx.moveTo(mx, midY - r);
      ctx.lineTo(mx + r, midY);
      ctx.lineTo(mx, midY + r);
      ctx.lineTo(mx - r, midY);
      ctx.closePath();
      ctx.fill();
      if (m.label && mx + r + 4 > lastLabelRight) {
        ctx.font = `400 10.5px ${theme.font}`;
        ctx.fillStyle = theme.text;
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        const avail = plotRight - (mx + r + 5);
        if (avail >= 24) {
          const fitted = ellipsise(m.label, avail, s => textWidth(ctx, s));
          ctx.fillText(fitted, mx + r + 5, midY + 0.5);
          lastLabelRight = mx + r + 5 + textWidth(ctx, fitted) + 6;
        }
      }
      ctx.restore();
    }
    return;
  }

  const spans = lane.spans ?? [];
  const iconsOn = model.iconsOnBars !== false;
  for (const span of spans) {
    const rawX0 = toX(span.firstHour);
    const rawX1 = toX(span.lastHour);
    const x0 = Math.max(plotLeft, rawX0);
    const x1 = Math.min(plotRight, rawX1);
    if (x1 < plotLeft || x0 > plotRight) continue;
    const w = Math.max(1.5, x1 - x0);

    if (lane.kind === 'matchup') {
      drawSplitWarBar(ctx, {
        x: x0,
        y: y + Math.floor((h - 13) / 2),
        width: w,
        height: 13,
        topFill: span.color,
        bottomFill: span.bottomColor ?? span.color,
        theme,
        openAtPlayhead: span.openAtPlayhead,
        expanded,
        ended: span.ended,
      });
      if (!expanded) {
        drawSpanMarkers(ctx, span, y + Math.floor((h - 13) / 2), 13, layout, readView, theme);
      }
      continue;
    }

    const barH = lane.kind === 'participant'
      ? 10
      : iconsOn && (span.icon || span.iconMissing)
        ? Math.min(18, h - 8)
        : Math.min(14, h - 8);
    const barY = Math.round(midY - barH / 2);
    const iconSlot = iconsOn && (span.icon || span.iconMissing)
      ? artSlotFromReadView(readView, span.icon, span.iconMissing, span.label)
      : undefined;

    drawTimelineSpanBar(ctx, {
      x: x0,
      y: barY,
      width: w,
      height: barH,
      fill: span.color,
      theme,
      openAtPlayhead: span.openAtPlayhead,
      heldBefore: span.heldBefore,
      fadedStart: span.fadedStart,
      fadedEnd: span.fadedEnd,
      ended: span.ended,
      radius: lane.kind === 'participant' ? 2 : 2.5,
      icon: iconSlot,
      iconSize: barH,
      label: span.label,
      hatched: span.hatched,
    });

    drawSpanMarkers(ctx, span, barY, barH, layout, readView, theme);

    if (span.endText && x1 + 6 < plotRight - 24) {
      ctx.save();
      ctx.font = `400 10.5px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(ellipsise(span.endText, plotRight - x1 - 8, s => textWidth(ctx, s)), x1 + 5, midY + 0.5);
      ctx.restore();
    }
  }
}

function drawMatchupOutcome(
  ctx: Canvas2D,
  lane: TimelineLaneModel,
  y: number,
  h: number,
  layout: TimelineLayout,
  theme: FamilyTheme,
): void {
  if (!lane.matchup || layout.outcomeWidth <= 20) return;
  const x = layout.plotRight + 10;
  const maxW = Math.max(10, layout.width - x - 10);
  const outcome = lane.matchup.outcome;
  if (outcome.detail && h >= 32) {
    drawStatusMark(ctx, x, y + 12, outcome.status, theme, { label: outcome.label, maxWidth: maxW });
    ctx.save();
    ctx.font = `400 10.5px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(ellipsise(outcome.detail, maxW, s => textWidth(ctx, s)), x + 12, y + 26);
    ctx.restore();
  } else {
    drawStatusMark(ctx, x, y + h / 2, outcome.status, theme, { label: outcome.label, maxWidth: maxW });
  }
}

function drawMeterRow(
  ctx: Canvas2D,
  row: TimelineMeterRow,
  y: number,
  h: number,
  width: number,
  readView: BoundedReadView,
  theme: FamilyTheme,
  inspected: boolean,
  selected: boolean,
  focused: boolean,
): void {
  let rowTop = y;
  let rowH = h;
  if (row.sectionHeader) {
    ctx.save();
    ctx.font = `600 11px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(row.sectionHeader, 12, rowTop + METER_SECTION_HEADER_HEIGHT / 2 + 2);
    ctx.restore();
    rowTop += METER_SECTION_HEADER_HEIGHT;
    rowH -= METER_SECTION_HEADER_HEIGHT;
  }

  if (selected) drawRowSelected(ctx, 0, rowTop, width, rowH, theme);
  else if (inspected) drawRowHover(ctx, 0, rowTop, width, rowH, theme);
  if (focused) drawRowFocusRing(ctx, 0, rowTop, width, rowH, theme);

  if (row.plainRows) {
    let py = rowTop + 4;
    for (const item of row.plainRows) {
      let lx = 14;
      const my = py + 11;
      if (item.icon || item.iconMissing) {
        drawArtSlot(ctx, lx, my - 8, 16, 16, artSlotFromReadView(readView, item.icon, item.iconMissing, item.label), theme, { radius: 2 });
        lx += 22;
      }
      ctx.save();
      ctx.font = `500 12px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      const detailW = item.detail ? Math.min(130, width * 0.36) : 0;
      ctx.fillText(ellipsise(item.label, Math.max(20, width - lx - detailW - 18), s => textWidth(ctx, s)), lx, my);
      if (item.detail) {
        ctx.font = `400 11px ${theme.font}`;
        ctx.fillStyle = theme.muted;
        ctx.textAlign = 'right';
        ctx.fillText(ellipsise(item.detail, detailW, s => textWidth(ctx, s)), width - 14, my);
      }
      ctx.restore();
      py += 22;
    }
    return;
  }

  const midY = rowTop + rowH / 2;
  let left = 12;
  if (row.flag || row.flagTag) {
    drawFlagSlot(ctx, left, Math.round(midY - FLAG_SLOT_HEIGHT / 2), flagSlotFromReadView(readView, row.flag, row.flagTag), theme);
    left += FLAG_SLOT_WIDTH + 8;
  } else if (row.icon || row.iconMissing) {
    drawArtSlot(ctx, left, Math.round(midY - 9), 18, 18, artSlotFromReadView(readView, row.icon, row.iconMissing, row.label), theme, { radius: 3 });
    left += 24;
  }

  const showSpark = Boolean(row.sparkline && row.sparkline.length >= 2 && width >= 460);
  const sparkW = showSpark ? 56 : 0;
  const valueW = row.missing ? 86 : 64;
  const rightEdge = width - 12;
  const sparkX = rightEdge - sparkW;
  const valueRight = showSpark ? sparkX - 10 : rightEdge;
  const labelW = Math.min(176, Math.max(84, Math.floor((valueRight - left) * 0.38)));
  const barX = left + labelW + 10;
  const barW = Math.max(28, valueRight - valueW - barX - 8);

  ctx.save();
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.font = `500 12px ${theme.font}`;
  ctx.fillStyle = theme.text;
  if (row.subtitle && rowH >= 32) {
    ctx.fillText(ellipsise(row.label, labelW, s => textWidth(ctx, s)), left, rowTop + 12);
    ctx.font = `400 11px ${theme.font}`;
    ctx.fillStyle = row.subtitleColor ?? theme.muted;
    ctx.fillText(ellipsise(row.subtitle, labelW, s => textWidth(ctx, s)), left, rowTop + 26);
  } else {
    ctx.fillText(ellipsise(row.label, labelW, s => textWidth(ctx, s)), left, midY);
  }
  ctx.restore();

  if (row.splitSegments && row.splitSegments.length > 0) {
    drawSplitBar(ctx, barX, Math.round(midY - 4), barW, 8, row.splitSegments, theme);
  } else if (row.thresholdTicks) {
    drawThresholdMeter(ctx, barX, midY, barW, 7, row.fraction, row.fill, theme);
  } else if (row.fraction === null) {
    drawHatchRect(ctx, barX, Math.round(midY - 3), barW, 6, 2, theme);
  } else {
    drawTrackBar(ctx, barX, midY, barW, 6, row.fraction, row.fill, theme);
  }

  if (row.missing) {
    drawMissingMark(ctx, valueRight - valueW, midY, valueW, theme, { align: 'right' });
  } else {
    ctx.save();
    ctx.font = `600 12.5px ${theme.font}`;
    ctx.fillStyle = theme.text;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillText(row.valueText, valueRight, midY);
    ctx.restore();
  }

  if (showSpark && row.sparkline) {
    drawSparkline(ctx, sparkX, Math.round(midY - 9), sparkW, 18, row.sparkline, row.fill, theme);
  }

  drawRowHairline(ctx, 12, width - 12, rowTop + rowH - 1, theme, 'normal');
}

function drawSeriesMode(
  ctx: Canvas2D,
  series: TimelineSeriesModel,
  model: TimelineModel,
  layout: TimelineLayout,
  theme: FamilyTheme,
): void {
  const { plotLeft, plotRight, contentTop, axisTop, toX } = layout;
  const tokens = resolveKitTokens(theme);
  const top = contentTop + 10;
  const bottom = axisTop - 8;
  const plotH = Math.max(20, bottom - top);
  const ySpan = Math.max(1e-6, series.yMax - series.yMin);
  const toY = (val: number) => bottom - ((Math.max(series.yMin, Math.min(series.yMax, val)) - series.yMin) / ySpan) * plotH;

  if (model.unobserved) {
    for (const gap of model.unobserved) {
      const gx0 = Math.max(plotLeft, toX(gap.firstHour));
      const gx1 = Math.min(plotRight, toX(gap.lastHour));
      if (gx1 > gx0) drawGapBand(ctx, gx0, top, gx1 - gx0, plotH, theme);
    }
  }

  const yTicks = series.yFormat === 'percent' ? [0, 0.25, 0.5, 0.75, 1] : [series.yMin, (series.yMin + series.yMax) / 2, series.yMax];
  ctx.save();
  ctx.font = `400 10.5px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.textAlign = 'right';
  ctx.textBaseline = 'middle';
  for (const v of yTicks) {
    const py = Math.round(toY(v)) + 0.5;
    ctx.strokeStyle = tokens.gridLine;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(plotLeft, py);
    ctx.lineTo(plotRight, py);
    ctx.stroke();
    const label = series.yFormat === 'percent' ? `${Math.round(v * 100)}%` : String(Math.round(v));
    ctx.fillText(label, plotLeft - 6, py);
  }

  if (series.thresholdLines) {
    ctx.strokeStyle = tokens.zeroLine;
    ctx.setLineDash([4, 3]);
    for (const v of series.thresholdLines) {
      const py = Math.round(toY(v)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(plotLeft, py);
      ctx.lineTo(plotRight, py);
      ctx.stroke();
    }
    ctx.setLineDash([]);
  }

  const endLabels: { y: number; text: string; color: string }[] = [];
  for (const line of series.lines) {
    ctx.strokeStyle = line.color;
    ctx.lineWidth = 1.8;
    ctx.lineJoin = 'round';
    ctx.lineCap = 'round';
    if (line.dashed) ctx.setLineDash([4, 3]);
    else ctx.setLineDash([]);
    ctx.beginPath();
    let drawing = false;
    let lastX = plotLeft;
    let lastY = bottom;
    for (const [h, v] of line.points) {
      if (v === null || !Number.isFinite(v) || h > layout.endHour) {
        drawing = false;
        continue;
      }
      const px = Math.max(plotLeft, Math.min(plotRight, toX(h)));
      const py = toY(v);
      if (!drawing) {
        ctx.moveTo(px, py);
        drawing = true;
      } else {
        ctx.lineTo(px, py);
      }
      lastX = px;
      lastY = py;
    }
    ctx.stroke();
    if (drawing && line.endLabel && layout.outcomeWidth >= 60) {
      endLabels.push({ y: lastY, text: line.endLabel, color: line.color });
      ctx.fillStyle = line.color;
      ctx.beginPath();
      ctx.arc(lastX, lastY, 2.8, 0, Math.PI * 2);
      ctx.fill();
    }
  }
  ctx.setLineDash([]);

  if (endLabels.length > 0 && layout.outcomeWidth >= 60) {
    endLabels.sort((a, b) => a.y - b.y);
    let prevY = top - 14;
    ctx.font = `500 11px ${theme.font}`;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const maxW = Math.max(20, layout.width - plotRight - 14);
    for (const lbl of endLabels) {
      const y = Math.max(prevY + 14, Math.min(bottom, lbl.y));
      ctx.fillStyle = lbl.color;
      ctx.fillText(ellipsise(lbl.text, maxW, s => textWidth(ctx, s)), plotRight + 8, y);
      prevY = y;
    }
  }

  if (series.markers) {
    for (const m of series.markers) {
      const mx = toX(m.hour);
      if (mx < plotLeft || mx > plotRight) continue;
      const r = 4;
      ctx.fillStyle = m.color ?? theme.accent;
      ctx.beginPath();
      ctx.moveTo(mx, bottom - r);
      ctx.lineTo(mx + r, bottom);
      ctx.lineTo(mx, bottom + r);
      ctx.lineTo(mx - r, bottom);
      ctx.closePath();
      ctx.fill();
    }
  }
  ctx.restore();

  drawBottomTimeAxis(ctx, {
    plotLeft,
    plotRight,
    plotTop: top,
    axisTop,
    startHour: layout.startHour,
    endHour: layout.endHour,
    playheadHour: model.playhead,
    theme,
    drawGridlines: false,
  });
}

export function drawFamilyTimeline(ctx: Canvas2D, input: DrawFamilyTimelineInput): void {
  const { model, layout, readView, theme, scrollTop, expanded, inspectedKey, selectedKey, focusedKey } = input;
  const { width, height } = layout;

  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, width, height);

  if (model.headerCard) {
    drawHeaderCard(ctx, model.headerCard, width, readView, theme);
  }

  if (model.empty) {
    ctx.save();
    ctx.font = `400 12px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(ellipsise(model.empty, Math.max(40, width - 32), s => textWidth(ctx, s)), width / 2, layout.headerCardBottom + (height - layout.headerCardBottom) / 2);
    ctx.restore();
    return;
  }

  if (model.mode === 'series' && model.series) {
    drawSeriesMode(ctx, model.series, model, layout, theme);
    return;
  }

  if (model.mode === 'meters') {
    const meters = model.meters ?? [];
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, layout.headerCardBottom, width, Math.max(0, height - layout.headerCardBottom));
    ctx.clip();
    for (const placed of layout.meters) {
      const y = placed.y - scrollTop;
      if (y + placed.height < layout.headerCardBottom || y > height) continue;
      const row = meters[placed.index]!;
      drawMeterRow(
        ctx,
        row,
        y,
        placed.height,
        width,
        readView,
        theme,
        inspectedKey === row.key,
        selectedKey === row.key,
        focusedKey === row.key,
      );
    }
    if (scrollTop + (height - layout.headerCardBottom) < layout.contentHeight - 2) {
      drawScrollFade(ctx, 0, height - 24, width, 24, theme, 'bottom');
    }
    ctx.restore();
    return;
  }

  if (model.mode === 'sections') {
    const sections = model.sections ?? [];
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, layout.headerCardBottom, width, Math.max(0, height - layout.headerCardBottom));
    ctx.clip();
    for (const sh of layout.sectionHeaders) {
      const y = sh.y - scrollTop;
      if (y + sh.height < layout.headerCardBottom || y > height) continue;
      ctx.save();
      ctx.font = `600 11.5px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(sh.section.title, sh.x, y + sh.height / 2);
      if (sh.section.countText) {
        const tw = textWidth(ctx, sh.section.title);
        ctx.font = `400 11px ${theme.font}`;
        ctx.fillStyle = theme.muted;
        ctx.fillText(` \u00b7 ${sh.section.countText}`, sh.x + tw, y + sh.height / 2);
      }
      ctx.restore();
      drawRowHairline(ctx, sh.x, sh.x + sh.width, y + sh.height - 1, theme, 'strong');
    }
    for (const si of layout.sectionItems) {
      const y = si.y - scrollTop;
      if (y + si.height < layout.headerCardBottom || y > height) continue;
      const item = sections[si.sectionIndex]?.items[si.itemIndex];
      if (!item) continue;
      if (selectedKey === item.key) drawRowSelected(ctx, si.x - 4, y, si.width + 8, si.height, theme, 0);
      else if (inspectedKey === item.key) drawRowHover(ctx, si.x - 4, y, si.width + 8, si.height, theme, 0);
      if (focusedKey === item.key) drawRowFocusRing(ctx, si.x - 4, y, si.width + 8, si.height, theme, 0);

      const midY = y + si.height / 2;
      ctx.save();
      ctx.font = `400 11px ${theme.font}`;
      const rightW = textWidth(ctx, item.rightText);
      const nameMax = Math.max(20, si.width - rightW - 12);
      if (item.missingName) {
        drawMissingMark(ctx, si.x, midY, nameMax, theme, { figure: 'Name' });
      } else {
        ctx.font = `500 12px ${theme.font}`;
        ctx.fillStyle = item.muted ? theme.muted : theme.text;
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(ellipsise(item.label, nameMax, s => textWidth(ctx, s)), si.x, midY);
      }
      ctx.font = `400 11px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillText(item.rightText, si.x + si.width, midY);
      ctx.restore();
      drawRowHairline(ctx, si.x, si.x + si.width, y + si.height - 1, theme, 'normal');
    }
    if (scrollTop + (height - layout.headerCardBottom) < layout.contentHeight - 2) {
      drawScrollFade(ctx, 0, height - 24, width, 24, theme, 'bottom');
    }
    ctx.restore();
    return;
  }

  const plotAreaH = Math.max(0, layout.axisTop - layout.headerCardBottom);
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, layout.headerCardBottom, width, plotAreaH);
  ctx.clip();

  if (model.unobserved) {
    for (const gap of model.unobserved) {
      const gx0 = Math.max(layout.plotLeft, layout.toX(gap.firstHour));
      const gx1 = Math.min(layout.plotRight, layout.toX(gap.lastHour));
      if (gx1 > gx0) drawGapBand(ctx, gx0, layout.headerCardBottom, gx1 - gx0, plotAreaH, theme);
    }
  }

  for (const placed of layout.lanes) {
    const y = placed.y - scrollTop;
    if (y + placed.height < layout.headerCardBottom || y > layout.axisTop) continue;
    const lane = placed.lane;
    const laneOpen = isLaneExpanded(lane, expanded);
    if (lane.kind === 'participant' || (lane.depth ?? 0) > 0) {
      drawChildRowTint(ctx, 0, y, width, placed.height, theme);
    }
    if (selectedKey === lane.key) drawRowSelected(ctx, 0, y, width, placed.height, theme);
    else if (inspectedKey === lane.key) drawRowHover(ctx, 0, y, width, placed.height, theme);
    if (focusedKey === lane.key) drawRowFocusRing(ctx, 0, y, width, placed.height, theme);

    drawLaneLabel(ctx, lane, y, placed.height, layout.labelWidth, laneOpen, readView, theme);
    drawLanePlot(ctx, lane, y, placed.height, laneOpen, layout, model, readView, theme);
    drawMatchupOutcome(ctx, lane, y, placed.height, layout, theme);
    if (lane.kind !== 'section-header') {
      drawRowHairline(ctx, 8, width - 8, y + placed.height - 1, theme, lane.kind === 'group-header' ? 'strong' : 'normal');
    }
  }

  if (model.bottomMeters && model.bottomMeters.length > 0) {
    const bmTop = layout.bottomMetersTop - scrollTop;
    if (bmTop < layout.axisTop) {
      ctx.save();
      ctx.font = `600 11px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(model.bottomMetersTitle ?? 'Traits in training', 12, bmTop + BOTTOM_METERS_HEADER_HEIGHT / 2);
      ctx.restore();
      for (const pm of layout.bottomMeters) {
        const my = pm.y - scrollTop;
        if (my + pm.height < layout.headerCardBottom || my > layout.axisTop) continue;
        const row = model.bottomMeters[pm.index]!;
        drawMeterRow(ctx, row, my, pm.height, width, readView, theme, inspectedKey === row.key, selectedKey === row.key, focusedKey === row.key);
      }
    }
  }

  if (scrollTop + plotAreaH < layout.contentHeight - 2) {
    drawScrollFade(ctx, 0, layout.axisTop - 24, width, 24, theme, 'bottom');
  }
  ctx.restore();

  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, layout.axisTop, width, TIMELINE_AXIS_HEIGHT + 4);
  drawBottomTimeAxis(ctx, {
    plotLeft: layout.plotLeft,
    plotRight: layout.plotRight,
    plotTop: layout.headerCardBottom,
    axisTop: layout.axisTop,
    startHour: layout.startHour,
    endHour: layout.endHour,
    playheadHour: model.playhead,
    theme,
    drawGridlines: true,
  });
}

export function hitFamilyTimeline(
  model: TimelineModel,
  layout: TimelineLayout,
  point: { readonly x: number; readonly y: number },
  scrollTop: number,
): TimelineHitTarget | null {
  if (model.empty || point.y < layout.headerCardBottom) return null;

  if (model.mode === 'meters') {
    const meters = model.meters ?? [];
    const logicalY = point.y + scrollTop;
    for (const placed of layout.meters) {
      if (logicalY >= placed.y && logicalY < placed.y + placed.height) {
        const row = meters[placed.index]!;
        return { kind: 'meter', key: row.key, meta: row.meta };
      }
    }
    return null;
  }

  if (model.mode === 'sections') {
    const logicalY = point.y + scrollTop;
    for (const si of layout.sectionItems) {
      if (point.x >= si.x && point.x <= si.x + si.width && logicalY >= si.y && logicalY < si.y + si.height) {
        const item = model.sections?.[si.sectionIndex]?.items[si.itemIndex];
        return { kind: 'section-item', key: si.key, meta: item?.meta };
      }
    }
    return null;
  }

  if (model.mode === 'series' && model.series) {
    if (point.x < layout.plotLeft || point.x > layout.plotRight || point.y > layout.axisTop) return null;
    const hour = Math.floor(layout.hourAtX(point.x));
    const line = model.series.lines[0];
    return line ? { kind: 'series', key: line.key, hour, meta: line.meta } : null;
  }

  if (point.y >= layout.axisTop) return null;
  const logicalY = point.y + scrollTop;

  for (const placed of layout.lanes) {
    if (logicalY < placed.y || logicalY >= placed.y + placed.height) continue;
    const lane = placed.lane;
    if (lane.kind === 'section-header') return null;
    if (lane.expandable && point.x <= layout.labelWidth) {
      return { kind: 'toggle', key: lane.key, laneKey: lane.key, meta: lane.meta };
    }
    const hour = point.x >= layout.plotLeft && point.x <= layout.plotRight
      ? Math.floor(layout.hourAtX(point.x))
      : undefined;
    if (hour !== undefined && lane.spans) {
      for (const span of lane.spans) {
        const x0 = layout.toX(span.firstHour);
        const x1 = layout.toX(span.lastHour);
        if (point.x >= x0 - 3 && point.x <= x1 + 3) {
          return { kind: 'span', key: lane.key, laneKey: lane.key, spanId: span.id, hour, meta: span.meta ?? lane.meta };
        }
      }
    }
    return { kind: 'lane', key: lane.key, laneKey: lane.key, hour, meta: lane.meta };
  }

  if (model.bottomMeters) {
    for (const pm of layout.bottomMeters) {
      if (logicalY >= pm.y && logicalY < pm.y + pm.height) {
        const row = model.bottomMeters[pm.index]!;
        return { kind: 'meter', key: row.key, meta: row.meta };
      }
    }
  }

  return null;
}

export function encodeTimelineHit(hit: TimelineHitTarget | null): string | null {
  if (!hit) return null;
  if (hit.kind === 'toggle') return `toggle:${hit.key}`;
  if (hit.kind === 'span' && hit.spanId) return `span:${hit.key}\u001f${hit.spanId}\u001f${hit.hour ?? ''}`;
  if (hit.hour !== undefined) return `row:${hit.key}\u001f\u001f${hit.hour}`;
  return `row:${hit.key}`;
}

export function decodeTimelineHit(encoded: string | null): TimelineHitTarget | null {
  if (!encoded) return null;
  if (encoded.startsWith('toggle:')) {
    const key = encoded.slice(7);
    return key ? { kind: 'toggle', key, laneKey: key } : null;
  }
  if (encoded.startsWith('span:')) {
    const [key, spanId, h] = encoded.slice(5).split('\u001f');
    const hour = h !== undefined && h !== '' ? Number(h) : undefined;
    return key ? { kind: 'span', key, laneKey: key, spanId, ...(Number.isFinite(hour) ? { hour } : {}) } : null;
  }
  if (encoded.startsWith('row:')) {
    const [key, , h] = encoded.slice(4).split('\u001f');
    const hour = h !== undefined && h !== '' ? Number(h) : undefined;
    return key ? { kind: 'lane', key, laneKey: key, ...(Number.isFinite(hour) ? { hour } : {}) } : null;
  }
  return null;
}
