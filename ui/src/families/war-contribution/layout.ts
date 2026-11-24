import {
  WC_MEASURES,
  sortSideMembers,
  type WCMeasureKey,
  type WCMemberRow,
  type WCSideSummary,
  type WCViewMode,
  type WarContributionModel,
} from './model.ts';

export interface WCRect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export interface WCSidesColumnLayout {
  readonly key: WCMeasureKey;
  readonly label: string;
  readonly x: number;
  readonly width: number;
  readonly sorted: boolean;
}

export interface WCSidesPanelLayout {
  readonly side: 'left' | 'right';
  readonly summary: WCSideSummary;
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly countryColWidth: number;
  readonly measureViewportX: number;
  readonly measureViewportWidth: number;
  readonly measureTotalWidth: number;
  readonly columns: readonly WCSidesColumnLayout[];
  readonly rows: readonly {
    readonly member: WCMemberRow;
    readonly y: number;
    readonly height: number;
  }[];
  readonly headerY: number;
  readonly warShareY: number;
  readonly subHeaderY: number;
  readonly rowsTop: number;
  readonly contentHeight: number;
}

export interface WCSidesLayout {
  readonly view: 'sides';
  readonly width: number;
  readonly height: number;
  readonly stacked: boolean;
  readonly sortKey: WCMeasureKey;
  readonly dividerX: number | null;
  readonly panels: readonly [WCSidesPanelLayout, WCSidesPanelLayout];
  readonly contentHeight: number;
}

export interface WCMeasureListRow {
  readonly key: WCMeasureKey;
  readonly label: string;
  readonly comparable: boolean;
  readonly y: number;
  readonly height: number;
  readonly leftFraction: number;
  readonly rightFraction: number;
  readonly dividerAbove: boolean;
}

export interface WCButterflyRow {
  readonly left: {
    readonly member: WCMemberRow;
    readonly valueText: string;
    readonly barFraction: number;
  } | null;
  readonly right: {
    readonly member: WCMemberRow;
    readonly valueText: string;
    readonly barFraction: number;
  } | null;
  readonly y: number;
  readonly height: number;
}

export interface WCMeasureLayout {
  readonly view: 'measure';
  readonly width: number;
  readonly height: number;
  readonly narrow: boolean;
  readonly selectedMeasure: WCMeasureKey;
  readonly measureLabel: string;
  readonly caption: string;
  readonly listArea: WCRect | null;
  readonly listRows: readonly WCMeasureListRow[];
  readonly rightArea: WCRect;
  readonly centerX: number;
  readonly nameColWidth: number;
  readonly valueColWidth: number;
  readonly barMaxWidth: number;
  readonly butterflyTop: number;
  readonly butterflyRows: readonly WCButterflyRow[];
  readonly contentHeight: number;
}

export type WarContributionLayout = WCSidesLayout | WCMeasureLayout;

const STACK_BREAKPOINT = 680;
const MEASURE_NARROW_BREAKPOINT = 600;
const HEADER_H = 34;
const WAR_SHARE_H = 26;
const SUB_HEADER_H = 20;
const ROW_H = 26;
const MEASURE_ROW_H = 22;

function layoutSidePanel(
  summary: WCSideSummary,
  x: number,
  y: number,
  width: number,
  height: number,
  sortKey: WCMeasureKey,
): WCSidesPanelLayout {
  const countryColWidth = Math.min(190, Math.max(120, Math.floor(width * 0.34)));
  const measureViewportX = x + countryColWidth;
  const measureViewportWidth = Math.max(60, width - countryColWidth);

  const columns: WCSidesColumnLayout[] = [];
  let cx = 0;
  for (const m of WC_MEASURES) {
    const sorted = m.key === sortKey;
    const colW = sorted ? 118 : 96;
    columns.push({ key: m.key, label: m.label, x: cx, width: colW, sorted });
    cx += colW;
  }

  const sortedMembers = sortSideMembers(summary.members, sortKey);
  const headerY = y;
  const warShareY = headerY + HEADER_H;
  const subHeaderY = warShareY + WAR_SHARE_H;
  const rowsTop = subHeaderY + SUB_HEADER_H;
  const rows = sortedMembers.map((member, idx) => ({
    member,
    y: idx * ROW_H,
    height: ROW_H,
  }));

  return {
    side: summary.side,
    summary,
    x,
    y,
    width,
    height,
    countryColWidth,
    measureViewportX,
    measureViewportWidth,
    measureTotalWidth: cx,
    columns,
    rows,
    headerY,
    warShareY,
    subHeaderY,
    rowsTop,
    contentHeight: rowsTop - y + rows.length * ROW_H + 14,
  };
}

export function layoutWarContribution(input: {
  readonly model: WarContributionModel;
  readonly view: WCViewMode;
  readonly sortKey: WCMeasureKey;
  readonly selectedMeasure: WCMeasureKey;
  readonly width: number;
  readonly height: number;
}): WarContributionLayout {
  const { model, view, sortKey, selectedMeasure, width, height } = input;
  const padX = 14;
  const padTop = 6;
  const padBottom = 10;

  if (view === 'sides') {
    const stacked = width < STACK_BREAKPOINT;
    if (stacked) {
      const panelW = Math.max(200, width - padX * 2);
      const leftPanel = layoutSidePanel(model.left, padX, padTop, panelW, Math.floor((height - padTop - padBottom - 16) / 2), sortKey);
      const rightTop = padTop + leftPanel.contentHeight + 12;
      const rightPanel = layoutSidePanel(model.right, padX, rightTop, panelW, Math.max(120, height - rightTop - padBottom), sortKey);
      return {
        view: 'sides',
        width,
        height,
        stacked: true,
        sortKey,
        dividerX: null,
        panels: [leftPanel, rightPanel],
        contentHeight: rightTop + rightPanel.contentHeight + padBottom,
      };
    }

    const gap = 28;
    const panelW = Math.max(200, Math.floor((width - padX * 2 - gap) / 2));
    const panelH = Math.max(120, height - padTop - padBottom);
    const leftPanel = layoutSidePanel(model.left, padX, padTop, panelW, panelH, sortKey);
    const rightPanel = layoutSidePanel(model.right, padX + panelW + gap, padTop, panelW, panelH, sortKey);
    return {
      view: 'sides',
      width,
      height,
      stacked: false,
      sortKey,
      dividerX: padX + panelW + Math.floor(gap / 2),
      panels: [leftPanel, rightPanel],
      contentHeight: Math.max(leftPanel.contentHeight, rightPanel.contentHeight) + padTop + padBottom,
    };
  }

  const narrow = width < MEASURE_NARROW_BREAKPOINT;
  const listW = narrow ? 0 : Math.min(330, Math.floor((width - padX * 2) * 0.24));
  const listGap = narrow ? 0 : 20;
  const listArea: WCRect | null = narrow
    ? null
    : { x: padX, y: padTop, width: listW, height: Math.max(100, height - padTop - padBottom) };

  const orderedMeasures = [
    ...WC_MEASURES.filter(m => m.comparable),
    ...WC_MEASURES.filter(m => !m.comparable),
  ];
  const listRows: WCMeasureListRow[] = [];
  let ly = padTop + 4;
  for (let i = 0; i < orderedMeasures.length; i++) {
    const m = orderedMeasures[i]!;
    const dividerAbove = !m.comparable;
    if (dividerAbove) ly += 8;
    const lt = model.left.totals[m.key];
    const rt = model.right.totals[m.key];
    const lFrac = m.comparable ? (lt.warShare ?? 0) : 0;
    const rFrac = m.comparable ? (rt.warShare ?? 0) : 0;
    listRows.push({
      key: m.key,
      label: m.comparable ? m.label : m.fullLabel,
      comparable: m.comparable,
      y: ly,
      height: MEASURE_ROW_H,
      leftFraction: lFrac,
      rightFraction: rFrac,
      dividerAbove,
    });
    ly += MEASURE_ROW_H + 2;
  }

  const rightX = padX + listW + listGap;
  const rightW = Math.max(200, width - rightX - padX);
  const rightArea: WCRect = {
    x: rightX,
    y: padTop,
    width: rightW,
    height: Math.max(100, height - padTop - padBottom),
  };

  const activeMeta = WC_MEASURES.find(m => m.key === selectedMeasure) ?? WC_MEASURES[1]!;
  const caption = selectedMeasure === 'participation'
    ? 'Each country’s share of its own side’s total contribution'
    : model.show === 'totals'
      ? `Each country’s ${activeMeta.fullLabel.toLowerCase()} in this war`
      : `Each country’s share of all ${activeMeta.fullLabel.toLowerCase()} in this war`;

  const leftActive = sortSideMembers(model.left.members, selectedMeasure).filter(m => !m.cells[selectedMeasure].isZero);
  const rightActive = sortSideMembers(model.right.members, selectedMeasure).filter(m => !m.cells[selectedMeasure].isZero);

  const metricOf = (member: WCMemberRow): number => {
    const cell = member.cells[selectedMeasure];
    if (selectedMeasure === 'participation') return Math.abs(cell.shareOfSide ?? 0);
    if (model.show === 'totals') return Math.abs(Number(cell.raw + cell.unknown));
    return Math.abs(cell.shareOfWar ?? cell.shareOfSide ?? 0);
  };

  let maxVal = 0;
  for (const m of leftActive) maxVal = Math.max(maxVal, metricOf(m));
  for (const m of rightActive) maxVal = Math.max(maxVal, metricOf(m));

  const centerGap = 36;
  const halfW = Math.max(80, Math.floor((rightW - centerGap) / 2));
  const nameColWidth = Math.min(150, Math.max(76, Math.floor(halfW * 0.42)));
  const valueColWidth = 64;
  const barMaxWidth = Math.max(24, halfW - nameColWidth - valueColWidth - 12);
  const centerX = rightX + Math.floor(rightW / 2);
  const butterflyTop = padTop + 30;

  const rowCount = Math.max(leftActive.length, rightActive.length);
  const butterflyRows: WCButterflyRow[] = [];
  for (let i = 0; i < rowCount; i++) {
    const lm = leftActive[i] ?? null;
    const rm = rightActive[i] ?? null;
    const labelFor = (member: WCMemberRow) => {
      const c = member.cells[selectedMeasure];
      if (selectedMeasure === 'participation') return c.displayText;
      if (model.show === 'totals') return c.amountText;
      const frac = c.shareOfWar ?? c.shareOfSide;
      return frac == null ? '—' : `${(frac * 100).toFixed(1)}%`;
    };
    butterflyRows.push({
      left: lm ? { member: lm, valueText: labelFor(lm), barFraction: maxVal > 0 ? metricOf(lm) / maxVal : 0 } : null,
      right: rm ? { member: rm, valueText: labelFor(rm), barFraction: maxVal > 0 ? metricOf(rm) / maxVal : 0 } : null,
      y: i * ROW_H,
      height: ROW_H,
    });
  }

  return {
    view: 'measure',
    width,
    height,
    narrow,
    selectedMeasure,
    measureLabel: activeMeta.fullLabel,
    caption,
    listArea,
    listRows,
    rightArea,
    centerX,
    nameColWidth,
    valueColWidth,
    barMaxWidth,
    butterflyTop,
    butterflyRows,
    contentHeight: Math.max(ly, butterflyTop + rowCount * ROW_H) + padBottom,
  };
}

export function hitTestWarContribution(
  layout: WarContributionLayout,
  x: number,
  y: number,
  scrollTop: number,
  scrollLeftLeft: number,
  scrollLeftRight: number,
): string | null {
  if (layout.view === 'sides') {
    for (const panel of layout.panels) {
      if (x < panel.x || x > panel.x + panel.width) continue;
      const sl = panel.side === 'left' ? scrollLeftLeft : scrollLeftRight;
      if (y >= panel.headerY && y < panel.headerY + HEADER_H && x >= panel.measureViewportX) {
        const mx = x - panel.measureViewportX + sl;
        for (const col of panel.columns) {
          if (mx >= col.x && mx < col.x + col.width) return `sort:${col.key}`;
        }
      }
      const localY = y - panel.rowsTop + scrollTop;
      if (y >= panel.rowsTop && localY >= 0) {
        for (const row of panel.rows) {
          if (localY >= row.y && localY < row.y + row.height) {
            return `row:${row.member.key}`;
          }
        }
      }
    }
    return null;
  }

  if (layout.listArea && x >= layout.listArea.x && x <= layout.listArea.x + layout.listArea.width) {
    for (const row of layout.listRows) {
      if (y >= row.y && y < row.y + row.height) {
        return `measure:${row.key}`;
      }
    }
  }

  if (y >= layout.butterflyTop && x >= layout.rightArea.x && x <= layout.rightArea.x + layout.rightArea.width) {
    const localY = y - layout.butterflyTop + scrollTop;
    for (const row of layout.butterflyRows) {
      if (localY >= row.y && localY < row.y + row.height) {
        if (x < layout.centerX && row.left) return `row:${row.left.member.key}`;
        if (x >= layout.centerX && row.right) return `row:${row.right.member.key}`;
      }
    }
  }
  return null;
}
