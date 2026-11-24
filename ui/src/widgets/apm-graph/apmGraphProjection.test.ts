import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import { boundTiledSeries, createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { IdentityIndex } from '../../families/identity/index.ts';
import { createSnapshotIdentityProvider } from '../../families/identity/provider.ts';
import type { ApmReply } from '../../generated/dashboard';
import { apmGraphProjection } from './apmGraphProjection.ts';

const H1936 = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
const H1943 = engineHour({ year: 1943, month: 9, day: 7, hour: 1 })!;

function makeTiled(id: string, val: number): TiledLine {
  return {
    tag: id,
    head: 'h',
    decimals: 0,
    summary: {
      from: H1936,
      to: H1943,
      width: 2,
      buckets: [{
        firstHour: H1936, lastHour: H1943, firstValue: val, lastValue: val, lastValueHour: H1943,
        minHour: H1936, minValue: val, maxHour: H1943, maxValue: val, firstBreak: false, hasBreak: false,
      }],
      tiles: { binHours: 96, valueAt: () => null },
    },
    missing: [],
    parts: [],
    marks: null,
  };
}

describe('apm-graph projection', () => {
  it('shows named unavailable state on single-player recordings with no player orders', () => {
    const readView = createBoundedReadView({ hour: H1943 });
    const model = apmGraphProjection.project(readView, {
      group: 'player',
      extent: [H1936, H1943],
      surface: '#111111',
      gaps: [],
      yMin: null,
      yMax: null,
      available: false,
    });
    assert.equal(model.empty?.kind, 'unavailable');
    assert.equal(model.empty?.message, 'Player orders were not recorded in this campaign.');
  });

  it('draws top 8 ranked players with end labels and flags, and ranks 9+ as grey tail lines', () => {
    const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: 'b', content: 0 }, [H1936, H1943]);
    index.setFacts({
      branchId: 'b',
      known: [[H1936, H1943]],
      names: ['Italy'],
      reasons: [],
      flags: [{ keys: ['ITA_fascism', null, null], reasons: [null, null, null] }],
      countries: [{ tag: 'ITA', runs: [[H1936, 0, 0x3b8e4a, -1, 0]] }],
      warnings: [],
    });
    const series = Array.from({ length: 10 }, (_, i) => ({
      id: `user:${i}`,
      name: `Player ${i + 1}`,
      nameMissing: null,
      tag: 'ITA',
      hour: H1943,
      summary: { apm: 100 - i * 8, confirmedApm: 100 - i * 8, orders: '100', pausedOrders: '5', unpausedMinutes: 10, complete: true, gaps: false },
      points: [],
    }));
    const head: ApmReply = { available: true, players: [], series, warnings: [] };
    const tiledMap = new Map(series.map(s => [s.id, boundTiledSeries(makeTiled(s.id, s.summary.apm!), displayedHour(H1943))]));
    const readView = createBoundedReadView({
      hour: H1943,
      panels: new Map([['head', head]]),
      series: new Map([['apm', tiledMap]]),
      identity: createSnapshotIdentityProvider(index),
    });
    const model = apmGraphProjection.project(readView, {
      group: 'player',
      extent: [H1936, H1943],
      surface: '#111111',
      gaps: [],
      yMin: null,
      yMax: null,
      available: true,
    });
    assert.equal(model.series.length, 10);
    assert.equal(model.series.filter(s => s.role === 'primary').length, 8);
    assert.equal(model.series.filter(s => s.role === 'tail').length, 2);
    assert.equal(model.series[0]?.endLabel?.name, 'Player 1');
    assert.equal(model.series[0]?.endLabel?.value, '100');
    assert.equal(model.series[8]?.endLabel, null);
    assert.equal(model.auditRows?.length, 8);
  });
});
