import { TIMELINE_AXIS_HEIGHT } from '../../draw/kit.ts';
import { visibleLanesOf, type TimelineLaneModel, type TimelineModel, type TimelineSectionGroup } from './model.ts';

export const HEADER_CARD_HEIGHT = 54;
export const DEFAULT_LANE_HEIGHT = 38;
export const GROUP_HEADER_HEIGHT = 30;
export const SECTION_HEADER_HEIGHT = 24;
export const MATCHUP_LANE_HEIGHT = 36;
export const PARTICIPANT_LANE_HEIGHT = 22;
export const COMPACT_LANE_HEIGHT = 26;
export const METER_ROW_HEIGHT = 38;
export const METER_SECTION_HEADER_HEIGHT = 24;
export const BOTTOM_METERS_HEADER_HEIGHT = 24;
export const SECTION_ITEM_HEIGHT = 26;
export const SECTION_TITLE_HEIGHT = 26;
export const PLOT_RIGHT_PAD = 10;

export function laneHeightOf(lane: TimelineLaneModel): number {
  if (lane.height !== undefined) return lane.height;
  switch (lane.kind) {
    case 'group-header': return GROUP_HEADER_HEIGHT;
    case 'section-header': return lane.label === 'Against' ? 18 : SECTION_HEADER_HEIGHT;
    case 'matchup': return MATCHUP_LANE_HEIGHT;
    case 'participant': return PARTICIPANT_LANE_HEIGHT;
    case 'diverging-bars': return 40;
    case 'markers': return 30;
    default: return lane.subtitle ? DEFAULT_LANE_HEIGHT : COMPACT_LANE_HEIGHT;
  }
}

export interface PlacedLane {
  readonly lane: TimelineLaneModel;
  readonly index: number;
  readonly y: number;
  readonly height: number;
}

export interface PlacedMeter {
  readonly index: number;
  readonly y: number;
  readonly height: number;
}

export interface PlacedSectionItem {
  readonly sectionIndex: number;
  readonly itemIndex: number;
  readonly key: string;
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export interface PlacedSectionHeader {
  readonly section: TimelineSectionGroup;
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export interface TimelineLayout {
  readonly width: number;
  readonly height: number;
  readonly headerCardBottom: number;
  readonly labelWidth: number;
  readonly outcomeWidth: number;
  readonly plotLeft: number;
  readonly plotRight: number;
  readonly contentTop: number;
  readonly axisTop: number;
  readonly contentHeight: number;
  readonly totalHeight: number;
  readonly lanes: readonly PlacedLane[];
  readonly meters: readonly PlacedMeter[];
  readonly bottomMetersTop: number;
  readonly bottomMeters: readonly PlacedMeter[];
  readonly sectionHeaders: readonly PlacedSectionHeader[];
  readonly sectionItems: readonly PlacedSectionItem[];
  readonly startHour: number;
  readonly endHour: number;
  readonly toX: (hour: number) => number;
  readonly hourAtX: (x: number) => number;
}

export function layoutTimeline(
  model: TimelineModel,
  width: number,
  height: number,
  expanded: Readonly<Record<string, boolean>>,
): TimelineLayout {
  const safeW = Math.max(120, width);
  const safeH = Math.max(80, height);
  const headerCardBottom = model.headerCard ? HEADER_CARD_HEIGHT : 0;
  const startHour = model.extent[0];
  const endHour = Math.max(startHour + 1, model.extent[1]);
  const span = Math.max(1, endHour - startHour);

  if (model.mode === 'lanes') {
    const visible = visibleLanesOf(model.lanes ?? [], expanded);
    const hasMatchup = visible.some(l => l.kind === 'matchup');
    const desiredOutcome = model.outcomeWidth ?? (hasMatchup ? (safeW >= 620 ? 196 : safeW >= 460 ? 150 : 0) : 0);
    const outcomeWidth = Math.min(desiredOutcome, Math.max(0, Math.floor(safeW * 0.28)));
    const desiredLabel = model.labelWidth ?? (hasMatchup ? 280 : 184);
    const maxLabel = Math.max(100, Math.floor((safeW - outcomeWidth) * (hasMatchup ? 0.46 : 0.42)));
    const labelWidth = Math.min(desiredLabel, maxLabel);
    const plotLeft = labelWidth + 8;
    const plotRight = Math.max(plotLeft + 40, safeW - outcomeWidth - PLOT_RIGHT_PAD);
    const plotW = Math.max(1, plotRight - plotLeft);

    const placedLanes: PlacedLane[] = [];
    let cursor = headerCardBottom + 4;
    for (let i = 0; i < visible.length; i++) {
      const lane = visible[i]!;
      const h = laneHeightOf(lane);
      placedLanes.push({ lane, index: i, y: cursor, height: h });
      cursor += h;
    }

    const bottomMeters: PlacedMeter[] = [];
    let bottomMetersTop = cursor;
    if (model.bottomMeters && model.bottomMeters.length > 0) {
      cursor += 8;
      bottomMetersTop = cursor;
      cursor += BOTTOM_METERS_HEADER_HEIGHT;
      for (let i = 0; i < model.bottomMeters.length; i++) {
        bottomMeters.push({ index: i, y: cursor, height: 24 });
        cursor += 24;
      }
    }

    const contentHeight = cursor - headerCardBottom;
    const axisTop = Math.max(headerCardBottom + 24, safeH - TIMELINE_AXIS_HEIGHT);
    const totalHeight = Math.max(safeH, cursor + TIMELINE_AXIS_HEIGHT + 4);

    return {
      width: safeW,
      height: safeH,
      headerCardBottom,
      labelWidth,
      outcomeWidth,
      plotLeft,
      plotRight,
      contentTop: headerCardBottom,
      axisTop,
      contentHeight,
      totalHeight,
      lanes: placedLanes,
      meters: [],
      bottomMetersTop,
      bottomMeters,
      sectionHeaders: [],
      sectionItems: [],
      startHour,
      endHour,
      toX: hour => plotLeft + ((hour - startHour) / span) * plotW,
      hourAtX: x => startHour + ((Math.max(plotLeft, Math.min(plotRight, x)) - plotLeft) / plotW) * span,
    };
  }

  if (model.mode === 'meters') {
    const meters: PlacedMeter[] = [];
    let cursor = headerCardBottom + 6;
    const rows = model.meters ?? [];
    for (let i = 0; i < rows.length; i++) {
      const row = rows[i]!;
      let h = row.plainRows ? 24 + row.plainRows.length * 22 : METER_ROW_HEIGHT;
      if (row.sectionHeader) h += METER_SECTION_HEADER_HEIGHT;
      meters.push({ index: i, y: cursor, height: h });
      cursor += h;
    }
    return {
      width: safeW,
      height: safeH,
      headerCardBottom,
      labelWidth: Math.min(180, Math.floor(safeW * 0.36)),
      outcomeWidth: 0,
      plotLeft: 12,
      plotRight: safeW - 12,
      contentTop: headerCardBottom,
      axisTop: safeH,
      contentHeight: cursor - headerCardBottom,
      totalHeight: Math.max(safeH, cursor + 8),
      lanes: [],
      meters,
      bottomMetersTop: cursor,
      bottomMeters: [],
      sectionHeaders: [],
      sectionItems: [],
      startHour,
      endHour,
      toX: () => 12,
      hourAtX: () => startHour,
    };
  }

  if (model.mode === 'series') {
    const plotLeft = 44;
    const plotRight = Math.max(plotLeft + 40, safeW - (safeW >= 420 ? 110 : 14));
    const plotW = Math.max(1, plotRight - plotLeft);
    const axisTop = Math.max(headerCardBottom + 48, safeH - TIMELINE_AXIS_HEIGHT);
    return {
      width: safeW,
      height: safeH,
      headerCardBottom,
      labelWidth: plotLeft,
      outcomeWidth: safeW - plotRight,
      plotLeft,
      plotRight,
      contentTop: headerCardBottom + 8,
      axisTop,
      contentHeight: axisTop - headerCardBottom,
      totalHeight: safeH,
      lanes: [],
      meters: [],
      bottomMetersTop: axisTop,
      bottomMeters: [],
      sectionHeaders: [],
      sectionItems: [],
      startHour,
      endHour,
      toX: hour => plotLeft + ((hour - startHour) / span) * plotW,
      hourAtX: x => startHour + ((Math.max(plotLeft, Math.min(plotRight, x)) - plotLeft) / plotW) * span,
    };
  }

  const sections = model.sections ?? [];
  const twoCol = safeW >= 440 && sections.some(s => s.column === 1);
  const padX = 12;
  const colGap = 20;
  const colW = twoCol ? Math.floor((safeW - padX * 2 - colGap) / 2) : safeW - padX * 2;
  const sectionHeaders: PlacedSectionHeader[] = [];
  const sectionItems: PlacedSectionItem[] = [];

  if (twoCol) {
    let leftY = headerCardBottom + 6;
    let rightY = headerCardBottom + 6;
    for (let si = 0; si < sections.length; si++) {
      const sec = sections[si]!;
      if (sec.column === 'full') continue;
      const isRight = sec.column === 1;
      const x = isRight ? padX + colW + colGap : padX;
      let y = isRight ? rightY : leftY;
      sectionHeaders.push({ section: sec, x, y, width: colW, height: SECTION_TITLE_HEIGHT });
      y += SECTION_TITLE_HEIGHT;
      for (let ii = 0; ii < sec.items.length; ii++) {
        const item = sec.items[ii]!;
        sectionItems.push({ sectionIndex: si, itemIndex: ii, key: item.key, x, y, width: colW, height: SECTION_ITEM_HEIGHT });
        y += SECTION_ITEM_HEIGHT;
      }
      y += 10;
      if (isRight) rightY = y; else leftY = y;
    }
    let fullY = Math.max(leftY, rightY);
    const fullW = safeW - padX * 2;
    for (let si = 0; si < sections.length; si++) {
      const sec = sections[si]!;
      if (sec.column !== 'full') continue;
      sectionHeaders.push({ section: sec, x: padX, y: fullY, width: fullW, height: SECTION_TITLE_HEIGHT });
      fullY += SECTION_TITLE_HEIGHT;
      for (let ii = 0; ii < sec.items.length; ii++) {
        const item = sec.items[ii]!;
        sectionItems.push({ sectionIndex: si, itemIndex: ii, key: item.key, x: padX, y: fullY, width: fullW, height: SECTION_ITEM_HEIGHT });
        fullY += SECTION_ITEM_HEIGHT;
      }
      fullY += 10;
    }
    return {
      width: safeW,
      height: safeH,
      headerCardBottom,
      labelWidth: 0,
      outcomeWidth: 0,
      plotLeft: padX,
      plotRight: safeW - padX,
      contentTop: headerCardBottom,
      axisTop: safeH,
      contentHeight: fullY - headerCardBottom,
      totalHeight: Math.max(safeH, fullY + 8),
      lanes: [],
      meters: [],
      bottomMetersTop: fullY,
      bottomMeters: [],
      sectionHeaders,
      sectionItems,
      startHour,
      endHour,
      toX: () => padX,
      hourAtX: () => startHour,
    };
  }

  let y = headerCardBottom + 6;
  for (let si = 0; si < sections.length; si++) {
    const sec = sections[si]!;
    sectionHeaders.push({ section: sec, x: padX, y, width: colW, height: SECTION_TITLE_HEIGHT });
    y += SECTION_TITLE_HEIGHT;
    for (let ii = 0; ii < sec.items.length; ii++) {
      const item = sec.items[ii]!;
      sectionItems.push({ sectionIndex: si, itemIndex: ii, key: item.key, x: padX, y, width: colW, height: SECTION_ITEM_HEIGHT });
      y += SECTION_ITEM_HEIGHT;
    }
    y += 10;
  }
  return {
    width: safeW,
    height: safeH,
    headerCardBottom,
    labelWidth: 0,
    outcomeWidth: 0,
    plotLeft: padX,
    plotRight: safeW - padX,
    contentTop: headerCardBottom,
    axisTop: safeH,
    contentHeight: y - headerCardBottom,
    totalHeight: Math.max(safeH, y + 8),
    lanes: [],
    meters: [],
    bottomMetersTop: y,
    bottomMeters: [],
    sectionHeaders,
    sectionItems,
    startHour,
    endHour,
    toX: () => padX,
    hourAtX: () => startHour,
  };
}
