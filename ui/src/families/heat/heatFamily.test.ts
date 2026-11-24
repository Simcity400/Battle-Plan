import assert from 'node:assert/strict';
import test from 'node:test';
import type { Canvas2D } from '../../charts/textWidth.ts';
import { computeTimeAxisTicks } from '../../draw/kit.ts';
import type { FamilyTheme } from '../contract.ts';
import { marginColour, sequentialRampColour, signedRampColour, splitStripes } from './scale.ts';
import {
  deriveMatrixAudit,
  deriveMatrixMirror,
  deriveMatrixTooltip,
  matrixGroupSpans,
  type HeatMatrixModel,
  type MatrixAxisEntity,
  type MatrixCell,
} from './matrixModel.ts';
import { hitHeatMatrix, layoutHeatMatrix, matrixKeyStep, revealMatrixOffset } from './matrixLayout.ts';
import { createMatrixPicture, drawHeatMatrix } from './matrixPainter.ts';
import { deriveLanesAudit, deriveLanesMirror, deriveLanesTooltip, findRunAtHour, type HeatLanesModel } from './lanesModel.ts';
import { hitHeatLanes, layoutHeatLanes, projectLaneRunsToPixels } from './lanesLayout.ts';
import { drawHeatLanes } from './lanesPainter.ts';
import { assertFreshPaint, assertQuietFrames } from '../laneRuntime.fixture.ts';
import { registerFamilyProjection } from '../registry.ts';
import { familyHeatLanesLaneView } from './lanesLaneView.ts';

function gridCells(grid: readonly (readonly MatrixCell[])[], width: number) {
  return { present: new Map(grid.flatMap((row, r) => row.map((cell, c) => [r * width + c, cell] as const))), absent: { kind: 'empty' } as MatrixCell };
}

const THEME: FamilyTheme = {
  surface: '#11161f',
  raised: '#1a212e',
  dark: true,
  text: '#e6ecf5',
  muted: '#8c99ad',
  line: '#2a3446',
  lineSoft: '#1f2837',
  accent: '#4f86f7',
  blue: '#4f86f7',
  red: '#e55353',
  amber: '#e2a336',
  green: '#2eb872',
  grey: '#3a4556',
  hatch: 'rgba(255, 255, 255, 0.2)',
  font: 'Inter, sans-serif',
  fontSize: { small: 11, body: 12 },
  lineWidth: 1.5,
  hatchSpacing: 6,
};

interface Op {
  readonly op: string;
  readonly args: readonly unknown[];
}

function makeFakeCtx(): { ctx: Canvas2D; ops: Op[] } {
  const ops: Op[] = [];
  const gradient = {
    addColorStop: (...args: unknown[]) => { ops.push({ op: 'addColorStop', args }); },
  };
  const ctx = {
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 1,
    globalAlpha: 1,
    font: '12px sans-serif',
    textAlign: 'left',
    textBaseline: 'alphabetic',
    save: () => { ops.push({ op: 'save', args: [] }); },
    restore: () => { ops.push({ op: 'restore', args: [] }); },
    beginPath: () => { ops.push({ op: 'beginPath', args: [] }); },
    clip: () => { ops.push({ op: 'clip', args: [] }); },
    rect: (...args: unknown[]) => { ops.push({ op: 'rect', args }); },
    roundRect: (...args: unknown[]) => { ops.push({ op: 'roundRect', args }); },
    moveTo: (...args: unknown[]) => { ops.push({ op: 'moveTo', args }); },
    lineTo: (...args: unknown[]) => { ops.push({ op: 'lineTo', args }); },
    arc: (...args: unknown[]) => { ops.push({ op: 'arc', args }); },
    fill: () => { ops.push({ op: 'fill', args: [] }); },
    stroke: () => { ops.push({ op: 'stroke', args: [] }); },
    fillRect: (...args: unknown[]) => { ops.push({ op: 'fillRect', args }); },
    fillText: (...args: unknown[]) => { ops.push({ op: 'fillText', args }); },
    setLineDash: (...args: unknown[]) => { ops.push({ op: 'setLineDash', args }); },
    createLinearGradient: () => gradient,
    drawImage: (...args: unknown[]) => { ops.push({ op: 'drawImage', args }); },
    measureText: (text: string) => ({ width: text.length * 6.5 }),
  } as unknown as Canvas2D;
  return { ctx, ops };
}

test('heat matrix supports grouped non-square axes, bounded viewport windows, hit-testing, painting, and exact audit', () => {
  const rows: MatrixAxisEntity[] = [
    { id: 'ENG', label: 'United Kingdom', tag: 'ENG', group: { id: 'f:1', label: 'Allies', color: '#4f86f7' } },
    { id: 'GER', label: 'German Reich', tag: 'GER', group: { id: 'f:2', label: 'Axis', color: '#e55353' } },
  ];
  const columns: MatrixAxisEntity[] = [
    { id: 'ENG', label: 'United Kingdom', tag: 'ENG', group: { id: 'f:1', label: 'Allies', color: '#4f86f7' } },
    { id: 'FRA', label: 'France', tag: 'FRA', group: { id: 'f:1', label: 'Allies', color: '#4f86f7' } },
    { id: 'GER', label: 'German Reich', tag: 'GER', group: { id: 'f:2', label: 'Axis', color: '#e55353' } },
  ];

  assert.deepEqual(
    matrixGroupSpans(columns).map(s => [s.id, s.first, s.count]),
    [['f:1', 0, 2], ['f:2', 2, 1]],
  );

  const grid: readonly (readonly MatrixCell[])[] = [
    [{ kind: 'diagonal' }, { kind: 'value', color: '#2eb872', text: '+50', value: 50 }, { kind: 'value', color: '#e55353', text: '-80', value: -80 }],
    [{ kind: 'empty', label: 'No network' }, { kind: 'missing', reason: 'Opinions not recorded' }, { kind: 'diagonal' }],
  ];

  const model: HeatMatrixModel = {
    hour: 240,
    rows,
    columns,
    cells: gridCells(grid, columns.length),
    legend: {
      gradient: { lowLabel: '-80', highLabel: '+50', stops: ['#e55353', '#3a4556', '#2eb872'] },
      entries: [{ id: 'missing', label: 'Not recorded', color: '#3a4556', hatched: true }],
    },
  };

  const layout = layoutHeatMatrix({
    width: 520,
    height: 260,
    scrollLeft: 0,
    scrollTop: 0,
    rows,
    columns,
    legend: model.legend,
    widestRowName: 96,
  });
  assert.equal(layout.showNames, true);
  assert.ok(layout.colGroupHeight > 0);
  assert.ok(layout.cellSize >= 26 && layout.cellSize <= 44);

  const compact = layoutHeatMatrix({
    width: 340,
    height: 260,
    scrollLeft: 0,
    scrollTop: 0,
    rows,
    columns,
  });
  assert.equal(compact.showNames, false);

  const cellHit = hitHeatMatrix(
    layout.gridLeft + layout.cellSize + 4,
    layout.gridTop + 4,
    0,
    0,
    layout,
    rows.length,
    columns.length,
  );
  assert.deepEqual(cellHit, { kind: 'cell', row: 0, column: 1 });
  assert.equal(hitHeatMatrix(4, 4, 0, 0, layout, rows.length, columns.length), null);

  assert.deepEqual(matrixKeyStep('ArrowRight', { row: 0, column: 0 }, 2, 3, 4), { row: 0, column: 1 });
  assert.deepEqual(matrixKeyStep('End', { row: 1, column: 0 }, 2, 3, 4), { row: 1, column: 2 });
  assert.equal(revealMatrixOffset(5, 30, 0, 100), 80);

  const tip = deriveMatrixTooltip(model, 1, 1);
  assert.equal(tip?.missingReason, 'Opinions not recorded');
  assert.equal(deriveMatrixTooltip(model, 0, 0), null);

  const audit = deriveMatrixAudit(model);
  assert.deepEqual(audit, [
    { key: 'ENG->FRA', row: 'United Kingdom → France', value: '+50' },
    { key: 'ENG->GER', row: 'United Kingdom → German Reich', value: '-80' },
    { key: 'GER->ENG', row: 'German Reich → United Kingdom', value: 'No network' },
    { key: 'GER->FRA', row: 'German Reich → France', value: 'Not recorded (Opinions not recorded)' },
  ]);

  const mirror = deriveMatrixMirror(model, 0, 2, 0, 3);
  assert.equal(mirror.total, 2);
  assert.equal(mirror.rows[1]!.cells[0]!.spoken, mirror.rows[1]!.name, 'the row is named for a screen reader');
  assert.match(mirror.rows[1]!.cells[2]!.spoken, /^.+: Not recorded \(Opinions not recorded\)$/, 'each cell says which column it is');

  const fake = makeFakeCtx();
  const picture = createMatrixPicture();
  const firstDraw = drawHeatMatrix(fake.ctx, {
    model,
    layout,
    theme: THEME,
    scrollLeft: 0,
    scrollTop: 0,
    hoveredCell: { row: 0, column: 1 },
    focusedCell: { row: 1, column: 0 },
  }, picture);
  assert.equal(firstDraw.drew, true);
  assert.ok(fake.ops.some(op => op.op === 'fillText' && op.args[0] === '-80'));
  assert.ok(fake.ops.some(op => op.op === 'fillText' && op.args[0] === 'Allies'));

  const repeatDraw = drawHeatMatrix(fake.ctx, {
    model,
    layout,
    theme: THEME,
    scrollLeft: 0,
    scrollTop: 0,
    hoveredCell: { row: 0, column: 1 },
    focusedCell: { row: 1, column: 0 },
  }, picture);
  assert.equal(repeatDraw.drew, false);

  const marked: HeatMatrixModel = {
    hour: 1,
    corner: 'Reads',
    rows: rows.map((row, i) => ({ ...row, trailing: `Broken ${i} · Decrypting 1` })),
    columns,
    cells: {
      present: new Map(rows.flatMap((_row, r) => [
        [r * columns.length, { kind: 'value', mark: 'square', color: '#d0607c' } as MatrixCell],
        [r * columns.length + 1, { kind: 'value', mark: 'ring', fraction: 0.4, color: '#d0607c' } as MatrixCell],
      ] as const)),
      absent: { kind: 'empty', mark: 'dot' },
      self: { kind: 'empty', mark: 'dot' },
    },
  };
  const markedLayout = layoutHeatMatrix({ width: 640, height: 260, scrollLeft: 0, scrollTop: 0, rows: marked.rows, columns, widestTrailing: 120 });
  assert.ok(markedLayout.trailingWidth > 120);
  assert.ok(markedLayout.totalWidth >= markedLayout.gridLeft + columns.length * markedLayout.cellSize + markedLayout.trailingWidth);
  const markedCtx = makeFakeCtx();
  drawHeatMatrix(markedCtx.ctx, { model: marked, layout: markedLayout, theme: THEME, scrollLeft: 0, scrollTop: 0, hoveredCell: null, focusedCell: null });
  const texts = markedCtx.ops.filter(op => op.op === 'fillText').map(op => op.args[0]);
  assert.ok(texts.includes('Reads'));
  assert.ok(texts.includes('Broken 0 · Decrypting 1') && texts.includes('Broken 1 · Decrypting 1'));
  assert.equal(markedCtx.ops.filter(op => op.op === 'arc').length, rows.length * 3, 'each row draws a ring (track and arc) and a dot');
});

test('heat matrix scales to 1,500 countries with bounded visible row and column windows', () => {
  const entities: MatrixAxisEntity[] = Array.from({ length: 1500 }, (_, i) => ({
    id: `C${i}`,
    label: `Country ${i}`,
    tag: `C${String(i).padStart(2, '0')}`,
  }));
  const layout = layoutHeatMatrix({
    width: 800,
    height: 600,
    scrollLeft: 3000,
    scrollTop: 4000,
    rows: entities,
    columns: entities,
  });
  assert.ok(layout.endRow - layout.firstRow < 30);
  assert.ok(layout.endCol - layout.firstCol < 35);
});

test('a 1,500 by 1,500 matrix reads only the cells a 20 by 20 viewport shows, its tooltip and its audit', () => {
  const entities: MatrixAxisEntity[] = Array.from({ length: 1500 }, (_, i) => ({ id: `C${i}`, label: `Country ${i}`, tag: `C${i}` }));
  let visits = 0;
  class CountingMap extends Map<number, MatrixCell> {
    override get(key: number): MatrixCell | undefined {
      visits++;
      return super.get(key);
    }
  }
  const present = new CountingMap();
  for (let i = 0; i < 1500; i++) present.set(i * 1500 + ((i + 1) % 1500), { kind: 'value', color: '#2eb872', text: '1', value: 1 });
  const model: HeatMatrixModel = { hour: 1, rows: entities, columns: entities, cells: { present, absent: { kind: 'empty', label: 'None' } } };
  const layout = layoutHeatMatrix({ width: 20 * 22 + 200, height: 20 * 22 + 120, scrollLeft: 22 * 700, scrollTop: 22 * 700, rows: entities, columns: entities });
  const shown = (layout.endRow - layout.firstRow) * (layout.endCol - layout.firstCol);
  assert.ok(shown <= 32 * 32, `the viewport is small: ${shown}`);
  const fake = makeFakeCtx();
  drawHeatMatrix(fake.ctx, { model, layout, theme: THEME, scrollLeft: 22 * 700, scrollTop: 22 * 700, hoveredCell: null, focusedCell: null }, createMatrixPicture());
  deriveMatrixMirror(model, layout.firstRow, layout.endRow, layout.firstCol, layout.endCol);
  deriveMatrixAudit(model, { firstRow: layout.firstRow, endRow: layout.endRow, firstCol: layout.firstCol, endCol: layout.endCol });
  deriveMatrixTooltip(model, layout.firstRow, layout.firstCol);
  assert.ok(visits <= shown * 3 + 1, `cells read: ${visits} for ${shown} shown`);
  visits = 0;
  assert.equal(deriveMatrixAudit(model).length, 1501, 'the whole-model audit lists the present pairs and the absent cell once');
  assert.equal(visits, 0, 'and reads no absent pair');
});

test('heat lanes never leak readings past displayed hour, never stretch over uncovered days, and keep 1px midnight terminal column', () => {
  const runs = [
    { firstHour: 0, lastHour: 239, readHour: 239, cell: { kind: 'solid' as const, color: '#4f86f7', label: 'Rain' } },
    { firstHour: 240, lastHour: 479, readHour: 479, cell: { kind: 'solid' as const, color: '#4f86f7', label: 'Rain' } },
    { firstHour: 504, lastHour: 504, readHour: 504, cell: { kind: 'solid' as const, color: '#2eb872', label: 'Clear' } },
    { firstHour: 528, lastHour: 551, readHour: 551, cell: { kind: 'solid' as const, color: '#e55353', label: 'Future' } },
  ];

  const spans = projectLaneRunsToPixels(runs, 0, 504, 504, 100, 400);
  assert.equal(spans.length, 2, 'adjacent equal runs coalesce, uncovered gap 480..503 stays unstretched, future run >504 dropped');
  assert.equal(spans[0]!.firstHour, 0);
  assert.equal(spans[0]!.lastHour, 479);
  assert.equal(spans[1]!.firstHour, 504);
  assert.equal(spans[1]!.lastHour, 504);
  assert.ok(spans[1]!.width >= 1, 'terminal 1-hour midnight column keeps at least 1px visibility');
  assert.ok(spans[0]!.x + spans[0]!.width < spans[1]!.x, 'uncovered day 480..503 is not stretched over');

  const earlier = projectLaneRunsToPixels(runs, 0, 551, 300, 100, 400);
  assert.equal(earlier.length, 1, 'run with readHour 479 > displayedHour 300 is withheld');
  assert.equal(earlier[0]!.lastHour, 239);

  const midnightRuns = [
    { firstHour: 0, lastHour: 4799, readHour: 4799, cell: { kind: 'solid' as const, color: '#4f86f7' } },
    { firstHour: 4800, lastHour: 4800, readHour: 4800, cell: { kind: 'solid' as const, color: '#e55353' } },
  ];
  const midnightSpans = projectLaneRunsToPixels(midnightRuns, 0, 4800, 4800, 100, 300);
  assert.equal(midnightSpans.length, 2);
  assert.ok(midnightSpans[1]!.width >= 1);
  assert.ok(midnightSpans[0]!.x + midnightSpans[0]!.width <= midnightSpans[1]!.x);
});

test('heat lanes painter, hit test, tooltips, ticks, and audit honour displayed-hour bounds', () => {
  const model: HeatLanesModel = {
    hour: 119,
    firstHour: 0,
    lastHour: 167,
    dayHours: 24,
    playhead: 119,
    gaps: [{ firstHour: 48, lastHour: 71, reason: 'Recorder paused' }],
    legend: {
      entries: [
        { id: 'allies', label: 'Allies', color: '#4f86f7' },
        { id: 'axis', label: 'Axis', color: '#e55353' },
      ],
    },
    rows: [
      {
        id: '10',
        label: 'English Channel',
        runs: [
          {
            firstHour: 0,
            lastHour: 47,
            readHour: 47,
            cell: {
              kind: 'parts',
              look: 'split',
              parts: [
                { id: 'allies', label: 'Allies', color: '#4f86f7', share: 0.7 },
                { id: 'axis', label: 'Axis', color: '#e55353', share: 0.3 },
              ],
            },
          },
          {
            firstHour: 72,
            lastHour: 119,
            readHour: 119,
            cell: { kind: 'solid', color: '#4f86f7', label: 'Allies' },
          },
        ],
        readout: { kind: 'value', primary: '70%', secondary: 'Since 4 Jan', tone: 'positive' },
      },
      {
        id: '11',
        label: 'Northern France',
        runs: [{ firstHour: 0, lastHour: 119, readHour: 119, cell: { kind: 'missing', reason: 'Scores withheld' } }],
        readout: { kind: 'missing', figure: 'Air', reason: 'Scores withheld' },
      },
    ],
  };

  const layout = layoutHeatLanes({
    width: 720,
    height: 220,
    scrollTop: 0,
    firstHour: model.firstHour,
    lastHour: model.lastHour,
    displayedHour: model.hour,
    rows: model.rows,
    legend: model.legend,
  });
  assert.equal(layout.showReadout, true);

  const hit = hitHeatLanes(layout.plotLeft + 20, layout.plotTop + 10, 0, layout, model.rows.length);
  assert.equal(hit?.kind, 'lane');
  if (hit?.kind === 'lane') {
    assert.equal(hit.rowIndex, 0);
    assert.ok(hit.hour <= model.hour);
  }

  assert.equal(findRunAtHour(model.rows[0]!.runs, 100, 119)?.firstHour, 72);
  assert.equal(findRunAtHour(model.rows[0]!.runs, 130, 119), null);

  const gapTip = deriveLanesTooltip(model, 0, 55);
  assert.equal(gapTip?.missingReason, 'Recorder paused');

  const audit = deriveLanesAudit(model);
  assert.deepEqual(audit, [
    { key: '10', row: 'English Channel', value: '70% (Since 4 Jan)' },
    { key: '11', row: 'Northern France', value: 'Air not recorded' },
  ]);

  const mirror = deriveLanesMirror(model, 0, 2);
  assert.equal(mirror.total, 2);
  assert.equal(mirror.rows[0]!.name, 'English Channel');
  assert.equal(mirror.rows[0]!.cells[0]!.spoken, 'English Channel');

  const ticks = computeTimeAxisTicks(0, 167, layout.plotWidth, 24);
  assert.ok(ticks.length >= 2);

  const fake = makeFakeCtx();
  drawHeatLanes(fake.ctx, {
    model,
    layout,
    theme: THEME,
    scrollTop: 0,
    hoveredRow: 0,
    hoveredHour: 96,
    selectedRowId: null,
    focusedRowId: '10',
  });
  assert.ok(fake.ops.some(op => op.op === 'fillText' && op.args[0] === '70%'));
  assert.ok(fake.ops.some(op => op.op === 'stroke'));

  registerFamilyProjection({ kind: 'test-heat-lanes', family: 'heat-lanes', project: view => view.panel('main') as HeatLanesModel });
  const lane = (panel: HeatLanesModel, scrollTop = 0) => ({ family: 'heat-lanes', kind: 'test-heat-lanes', options: {}, sources: {}, staticPanels: { main: panel }, scrollTop, theme: THEME, width: 720, height: 220 });
  const held = lane(model);
  const renamed = { ...model, rows: model.rows.map(row => (row.id === '11' ? { ...row, label: 'Normandy' } : row)) };
  const reordered = { ...renamed, rows: [...renamed.rows].reverse() };
  const surface = { width: 720, height: 220, dpr: 1 };
  assertQuietFrames(() => familyHeatLanesLaneView(() => {}), { label: 'lanes', binding: held, hour: 119 }, 24, surface);
  assertFreshPaint(() => familyHeatLanesLaneView(() => {}), [
    { label: 'held', binding: held, hour: 119, check: true },
    { label: 'later', binding: held, hour: 140, speed: 24 },
    { label: 'seek back', binding: held, hour: 119, check: true },
    { label: 'correction', binding: lane(renamed), hour: 119, check: true },
    { label: 'reorder', binding: lane(reordered), hour: 119, check: true },
    { label: 'scrolled', binding: lane(reordered, 12), hour: 119, check: true },
  ], surface);
});

test('heat scale helpers preserve zero neutrality, signed extremes, margin blend, and proportional stripes', () => {
  assert.equal(signedRampColour(0, -100, 100, '#444444', '#e55353', '#2eb872'), '#444444');
  assert.notEqual(signedRampColour(50, -100, 100, '#444444', '#e55353', '#2eb872'), '#444444');
  assert.equal(sequentialRampColour(0, 10, '#444444', '#4f86f7'), '#444444');
  assert.notEqual(sequentialRampColour(5, 10, '#444444', '#4f86f7'), '#444444');
  const blend = marginColour(
    [
      { id: 'a', label: 'A', color: '#4f86f7', share: 0.75 },
      { id: 'b', label: 'B', color: '#e55353', share: 0.25 },
    ],
    '#444444',
  );
  assert.ok(blend && blend !== '#444444');
  const stripes = splitStripes(
    [
      { id: 'a', label: 'A', color: '#4f86f7', share: 0.25 },
      { id: 'b', label: 'B', color: '#e55353', share: 0.75 },
    ],
    10,
    20,
  );
  assert.deepEqual(stripes, [
    { color: '#4f86f7', top: 10, height: 5 },
    { color: '#e55353', top: 15, height: 15 },
  ]);
});

test('a sub-pixel last observation gets one pixel at its own hour, and an uncovered hour is not the current readout', () => {
  const runs = [{ firstHour: 500, lastHour: 500, readHour: 500, cell: { kind: 'solid' as const, color: '#4f86f7' } }];
  const spans = projectLaneRunsToPixels(runs, 0, 1000, 500, 0, 100);
  assert.equal(spans.length, 1);
  assert.equal(spans[0]!.width, 1, 'the observation is one pixel wide, not stretched to the plot edge');
  assert.ok(Math.abs(spans[0]!.x - 49.95) < 0.1, 'at its witnessed position');
  const model: HeatLanesModel = {
    hour: 500, firstHour: 0, lastHour: 1000, dayHours: 24, playhead: 500,
    rows: [{ id: 'r', label: 'Region', runs, readout: { kind: 'value', primary: '71%', secondary: 'Allies' } }],
  };
  assert.deepEqual(deriveLanesTooltip(model, 0, 100), { title: 'Region', hour: 100, missingReason: 'Not recorded at this hour' });
  assert.equal(deriveLanesTooltip(model, 0, null)?.exactValue, '71%', 'the readout answers only when the readout itself is hit');
  const layout = layoutHeatLanes({ width: 400, height: 120, scrollTop: 0, firstHour: 0, lastHour: 1000, displayedHour: 500, rows: model.rows });
  const pastPlayhead = layout.plotLeft + layout.plotWidth * 0.9;
  assert.equal(hitHeatLanes(pastPlayhead, layout.plotTop + 4, 0, layout, 1), null, 'nothing after the displayed hour is hit');
});

test('stripes draw the fractions they are given and leave an unknown remainder unfilled', () => {
  const stripes = splitStripes(
    [
      { id: 'a', label: 'A', color: '#4f86f7', share: 0.25 },
      { id: 'b', label: 'B', color: '#e55353', share: 0.35 },
    ],
    10,
    20,
  );
  assert.deepEqual(stripes, [
    { color: '#4f86f7', top: 10, height: 5 },
    { color: '#e55353', top: 15, height: 7 },
  ]);
});

test('a stable row id is never read as a row position, and a reorder refreshes the published rows', async () => {
  const { heatLanesHostAdapter } = await import('./hostAdapter.ts');
  const { laneIdentities } = await import('./lanesModel.ts');
  const { matrixExtentSignature } = await import('./matrixLaneView.ts');
  const rows = [{ id: '1', label: 'One', runs: [] }, { id: '0', label: 'Zero', runs: [] }];
  const installed = { id: 7, owner: 'w', hour: 50 } as unknown as Parameters<NonNullable<typeof heatLanesHostAdapter.resolveHitAction>>[1];
  const fields = { extent: { totalHeight: 100, rowStride: 30, plotTop: 0, plotHeight: 100, rows: laneIdentities(rows) } };
  const byId = heatLanesHostAdapter.resolveHitAction!({ key: '1' } as never, installed, fields as never);
  assert.equal(byId?.subject, '1', 'Enter on stable id "1" activates "1", not the row at position 1');
  const byPointer = heatLanesHostAdapter.resolveHitAction!({ key: '#1@40' } as never, installed, fields as never);
  assert.equal(byPointer?.subject, '0', 'a pointer token names a position');
  const grid = { totalWidth: 100, totalHeight: 100, cellSize: 20, gridLeft: 10, gridTop: 10 };
  const a = { id: 'A', label: 'A' }, b = { id: 'B', label: 'B' };
  assert.notEqual(matrixExtentSignature(grid, { rows: [a, b], columns: [a, b] }), matrixExtentSignature(grid, { rows: [b, a], columns: [b, a] }));
});
