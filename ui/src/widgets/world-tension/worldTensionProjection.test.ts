import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { IdentityIndex } from '../../families/identity/index.ts';
import { createSnapshotIdentityProvider } from '../../families/identity/provider.ts';
import { deriveTimeChartTooltip, hitTestTimeChart } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import { drawTimeChart } from '../../families/time-chart/painter.ts';
import type { Canvas2D, MeasureText } from '../../charts/textWidth.ts';
import type { WorldTensionStackReply } from '../../generated/dashboard';
import { worldTensionEventsProjection } from './worldTensionEventsProjection.ts';
import { worldTensionLinesProjection } from './worldTensionLinesProjection.ts';
import { worldTensionProjection } from './worldTensionProjection.ts';

const H1936 = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
const H1941 = engineHour({ year: 1941, month: 9, day: 15, hour: 1 })!;
const measure: MeasureText = text => text.length * 6.5;

function makeIdentity() {
  const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: 'b', content: 0 }, [H1936, H1941 + 1000]);
  index.setFacts({
    branchId: 'b',
    known: [[H1936, H1941 + 1000]],
    names: ['German Reich', 'Japan', 'Italy'],
    reasons: [],
    flags: [
      { keys: ['GER_fascism', null, null], reasons: [null, null, null] },
      { keys: ['JAP_fascism', null, null], reasons: [null, null, null] },
      { keys: ['ITA_fascism', null, null], reasons: [null, null, null] },
    ],
    countries: [
      { tag: 'GER', runs: [[H1936, 0, 0x454545, -1, 0]] },
      { tag: 'JAP', runs: [[H1936, 1, 0xcc4444, -1, 1]] },
      { tag: 'ITA', runs: [[H1936, 2, 0x44aa55, -1, 2]] },
    ],
    warnings: [],
  });
  return createSnapshotIdentityProvider(index);
}

function sampleStack(): WorldTensionStackReply {
  return {
    branchId: '1',
    tensionScale: 1000,
    contributionScale: 100000,
    ceilingRaw: '10000000',
    endHour: H1941,
    cappedEndTension: 100,
    hours: [H1936, H1936 + 5000, H1936 + 20000],
    total: [0, 35, 100],
    bands: [
      { slot: 1, tag: 'GER', remembered: false, values: [0, 20, 60], endValue: 60, endRaw: '6000000' },
      { slot: 2, tag: 'JAP', remembered: false, values: [0, 10, 25], endValue: 25, endRaw: '2500000' },
      { slot: 3, tag: 'ITA', remembered: false, values: [0, 2, 3], endValue: 3, endRaw: '300000' },
    ],
    other: { count: 31, values: [0, 3, 12], endValue: 12 },
    events: [
      { hour: H1936 + 5000, tag: 'GER', remembered: false, reason: 'Remilitarised the Rhineland', addedRaw: '500000', firstRead: H1936 + 5000, lastRead: null },
      { hour: H1941 + 200, tag: 'JAP', remembered: false, reason: 'Future event', addedRaw: '1000000', firstRead: H1941 + 200, lastRead: null },
    ],
    missing: [],
    nextChangeHour: H1941 + 24,
    warnings: [],
  };
}

describe('world-tension projections', () => {
  it('projects stepped stack with Other at bottom, leader on top, shortName at small, and clamps future events', () => {
    const reply = sampleStack();
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', reply]]),
      identity: makeIdentity(),
    });
    const model = worldTensionProjection.project(readView, {
      extent: [H1936, H1941],
      surface: '#111111',
      layers: 5,
      values: 'share',
      showMarkers: true,
      gaps: [],
    });
    assert.equal(model.mode, 'stack');
    assert.equal(model.stack?.step, true);
    assert.equal(model.stack?.bands[0]?.key, 'GER');
    assert.equal(model.stack?.bands.at(-1)?.id, 'other');
    assert.equal(model.stack?.bands.at(-1)?.endLabel?.name, 'Other (31)');
    assert.equal(model.stack?.bands.at(-1)?.endLabel?.shortName, 'Other');
    assert.equal(model.stack?.total?.endLabel?.value, '100%');
    assert.equal(model.markers?.length, 1, 'future event after displayed hour is excluded');

    const fullLayout = layoutTimeChart(model, 960, 280, 'Segoe UI', measure);
    assert.ok(fullLayout.endLabels.some(l => l.name === 'Other (31)'));
    const totalLabel = fullLayout.endLabels.find(l => l.seriesId === '__total__')!;
    const labelHit = hitTestTimeChart(model, fullLayout, [totalLabel.x + 10, totalLabel.y]);
    assert.equal(labelHit, 'label:__total__');
    const tip = deriveTimeChartTooltip(model, fullLayout, labelHit);
    assert.equal(tip?.payload?.title, 'World tension');

    const minLayout = layoutTimeChart(model, 476, 186, 'Segoe UI', measure);
    assert.ok(minLayout.endLabels.some(l => l.name === 'Other'), 'min layout uses shortName Other');
    assert.equal(minLayout.endLabels.some(l => l.seriesId === 'slot:3'), false, 'thin <10px band label hidden at min size');

    const ops: string[] = [];
    const ctx = new Proxy({}, {
      get: (_t, prop) => {
        if (prop === 'measureText') return (t: string) => ({ width: t.length * 6.5 });
        return (...args: unknown[]) => { ops.push(String(prop)); return args; };
      },
    }) as unknown as Canvas2D;
    drawTimeChart(ctx, model, fullLayout, undefined, `plot:${H1936 + 5000}`, 1);
    assert.ok(ops.includes('fill') && ops.includes('arc'), 'stepped stack paints bands and hover dot');
  });

  it('projects world-tension events table view with dated events clipped to displayed hour', () => {
    const reply = sampleStack();
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', reply]]),
      identity: makeIdentity(),
    });
    const table = worldTensionEventsProjection.project(readView, { branchFirst: H1936 });
    assert.equal(table.rows.length, 1);
    assert.equal(table.rows[0]?.name, 'Remilitarised the Rhineland');
  });

  it('projects world-tension lines view with top-8 coloured lines and stepped inline points', () => {
    const reply = sampleStack();
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', reply]]),
      identity: makeIdentity(),
    });
    const model = worldTensionLinesProjection.project(readView, {
      extent: [H1936, H1941],
      surface: '#111111',
      gaps: [],
    });
    assert.equal(model.mode, 'lines');
    assert.equal(model.series.length, 3);
    assert.equal(model.series[0]?.key, 'GER');
    assert.equal(model.series[0]?.step, true);
    assert.equal(model.series[0]?.endLabel?.value, '60%');
    assert.equal(model.series[0]?.points?.length, 3);
  });

  it('K1: world-tension preserves small shares as nonzero and never turns non-finite inputs into 0%', () => {
    const reply = sampleStack();
    const withEdge = {
      ...reply,
      bands: [
        { slot: 1, tag: 'GER', remembered: false, values: [0, 0.4, 0.4], endValue: 0.4, endRaw: '40000' },
        { slot: 2, tag: 'JAP', remembered: false, values: [0, 0.04, 0.04], endValue: 0.04, endRaw: '4000' },
        { slot: 3, tag: 'ITA', remembered: false, values: [0, 1, 1], endValue: Number.NaN, endRaw: '0' },
      ],
      other: null,
    };
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', withEdge]]),
      identity: makeIdentity(),
    });
    const model = worldTensionProjection.project(readView, {
      extent: [H1936, H1941],
      surface: '#111111',
      layers: 5,
      values: 'share',
      showMarkers: false,
      gaps: [],
    });
    assert.equal(model.stack?.bands[0]?.endLabel?.value, '0.4%');
    assert.equal(model.stack?.bands[1]?.endLabel?.value, '<0.1%');
    assert.equal(model.stack?.bands[2]?.endLabel, null);
  });
});
