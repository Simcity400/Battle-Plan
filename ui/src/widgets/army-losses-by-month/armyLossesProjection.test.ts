import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { engineHour, firstHourOfMonth } from '../../dashboard/calendar.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { InstalledPicture } from '../../families/plan.ts';
import { deriveTimeChartTooltip, hitTestTimeChart } from '../../families/time-chart/hitTest.ts';
import { parseTimeChartHit, timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import { chartNavStops } from '../../families/time-chart/laneView.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import type { MeasureText } from '../../charts/textWidth.ts';
import type { ArmyLossesReply } from '../../generated/dashboard';
import { armyLossesProjection, monthOfHour, resolveArmyMonthDetail } from './armyLossesProjection.ts';

const H1936 = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
const H1941_SEP15 = engineHour({ year: 1941, month: 9, day: 15, hour: 1 })!;
const measure: MeasureText = text => text.length * 6.5;

function makeReply(overrides: Partial<ArmyLossesReply> = {}): ArmyLossesReply {
  const apr = 1941 * 12 + 3;
  const may = 1941 * 12 + 4;
  const sep = 1941 * 12 + 8;
  return {
    tag: 'GER',
    absent: false,
    rollup: {
      unitScale: 1,
      wrapI32: true,
      parts: [
        { key: 'ground', label: 'Ground', otherCount: 0 },
        { key: 'air', label: 'Air', otherCount: 0 },
        { key: 'navy', label: 'Navy', otherCount: 0 },
      ],
      months: [
        { month: apr, firstHour: firstHourOfMonth(apr), lastHour: firstHourOfMonth(apr + 1) - 1, liveRow: null, missing: null, values: ['0', '0', '0'], total: '0' },
        { month: may, firstHour: firstHourOfMonth(may), lastHour: firstHourOfMonth(may + 1) - 1, liveRow: `r:${may}`, missing: null, values: ['120000', '5000', '0'], total: '125000' },
        { month: sep, firstHour: firstHourOfMonth(sep), lastHour: firstHourOfMonth(sep + 1) - 1, liveRow: `r:${sep}`, missing: null, values: ['80000', '0', '0'], total: '80000' },
      ],
    },
    monthDetail: null,
    availability: { armies: null, manpower: null, months: null, ownEquipment: null, enemyEquipment: null },
    warnings: [],
    ...overrides,
  };
}

describe('army-losses-by-month projection (acceptance cases 1-9)', () => {
  it('folds 6+ quiet no-rows months, preserves recorded zero, marks partial current month, and emits nav stops', () => {
    const readView = createBoundedReadView({
      hour: H1941_SEP15,
      panels: new Map([['main', makeReply()]]),
    });
    const model = armyLossesProjection.project(readView, {
      measure: 'manpower',
      ledger: 'combat',
      stack: 'channel',
      foldUnrecorded: true,
      countryName: 'German Reich',
      following: true,
      extent: [H1936, H1941_SEP15],
    });
    assert.equal(model.mode, 'columns');
    assert.equal(model.empty, undefined);
    const layout = layoutTimeChart(model, 720, 260, 'Segoe UI', measure);
    assert.equal(layout.columns[0]?.kind, 'fold');
    assert.equal(layout.columns[0]?.foldLabel, '1936\u201341');
    const aprCol = layout.columns[1]!;
    assert.equal(aprCol.kind, 'month');
    assert.equal(aprCol.month?.status, 'recorded', 'recorded zero is kept distinct from no-rows');
    assert.equal(aprCol.month?.totalValue, 0);
    const sepCol = layout.columns.at(-1)!;
    assert.equal(sepCol.month?.partial, true);
    assert.equal(sepCol.month?.partialToHour, H1941_SEP15);
    const stops = chartNavStops(model, layout);
    assert.equal(stops.length, layout.columns.length);
    assert.ok(stops[1]![0].startsWith('month:1@'));
  });

  it('shows unavailable state when all months are NO_READ_YET, and inline note when partial SliceReads occur', () => {
    const may = 1941 * 12 + 4;
    const allUnknown = makeReply({
      rollup: {
        unitScale: 100000,
        wrapI32: false,
        parts: [],
        months: [{ month: may, firstHour: firstHourOfMonth(may), lastHour: firstHourOfMonth(may + 1) - 1, liveRow: null, missing: 'Combat Log equipment months of a country not read yet are unknown', values: [], total: '0' }],
      },
    });
    const unavail = armyLossesProjection.project(createBoundedReadView({ hour: H1941_SEP15, panels: new Map([['main', allUnknown]]) }), {
      measure: 'equipment',
      ledger: 'combat',
      stack: 'type',
      foldUnrecorded: true,
      countryName: 'German Reich',
      following: true,
      extent: [H1936, H1941_SEP15],
    });
    assert.equal(unavail.empty?.kind, 'unavailable');
    assert.ok(unavail.empty?.message.includes('not read yet'));

    const partialReply = makeReply();
    const aug = 1941 * 12 + 7;
    partialReply.rollup!.months.push({ month: aug, firstHour: firstHourOfMonth(aug), lastHour: firstHourOfMonth(aug + 1) - 1, liveRow: null, missing: 'Combat Log equipment months of a country not read yet are unknown', values: [], total: '0' });
    const partial = armyLossesProjection.project(createBoundedReadView({ hour: H1941_SEP15, panels: new Map([['main', partialReply]]) }), {
      measure: 'manpower',
      ledger: 'combat',
      stack: 'channel',
      foldUnrecorded: true,
      countryName: 'German Reich',
      following: true,
      extent: [H1936, H1941_SEP15],
    });
    assert.equal(partial.empty, undefined);
    assert.ok(partial.note?.includes('Some months could not be read'));
  });

  it('marks a month not-recorded when a live cell is missing instead of showing zero', () => {
    const sep = 1941 * 12 + 8;
    const live = {
      get: (key: string) => (key === liveKey(`r:${sep}`, 'p0') || key === liveKey(`r:${sep}`, 'total') ? 90000 : undefined),
      missing: () => 'CombatLogManpower was not recorded',
    };
    const model = armyLossesProjection.project(createBoundedReadView({
      hour: H1941_SEP15,
      panels: new Map([['main', makeReply()]]),
      live: new Map([['main', live]]),
    }), {
      measure: 'manpower',
      ledger: 'combat',
      stack: 'channel',
      foldUnrecorded: true,
      countryName: 'German Reich',
      following: true,
      extent: [H1936, H1941_SEP15],
    });
    const maySlot = model.columns!.months.find(m => m.year === 1941 && m.month === 5)!;
    assert.equal(maySlot.status, 'not-recorded');
    assert.equal(maySlot.reason, 'CombatLogManpower was not recorded');
    const sepSlot = model.columns!.months.find(m => m.year === 1941 && m.month === 9)!;
    assert.equal(sepSlot.status, 'recorded');
    assert.equal(sepSlot.totalValue, 90000);
    assert.equal(monthOfHour(sepSlot.firstHour), sep);

    const layout = layoutTimeChart(model, 720, 260, 'Segoe UI', measure);
    const sepCol = layout.columns.at(-1)!;
    const hit = hitTestTimeChart(model, layout, [sepCol.x + sepCol.width / 2, layout.top + 20]);
    const tip = deriveTimeChartTooltip(model, layout, hit);
    assert.equal(tip?.payload?.items?.at(-1)?.value, '90,000');
  });

  it('S6: partial bar activation carries installed frame hour, clicked month, and installed country separately', () => {
    const sep = 1941 * 12 + 8;
    const model = armyLossesProjection.project(
      createBoundedReadView({ hour: H1941_SEP15, panels: new Map([['main', makeReply()]]) }),
      {
        measure: 'equipment',
        ledger: 'attrition',
        stack: 'type',
        foldUnrecorded: true,
        countryKey: 'GER',
        countryName: 'German Reich',
        following: true,
        extent: [H1936, H1941_SEP15],
      } as Parameters<typeof armyLossesProjection.project>[1],
    );
    const layout = layoutTimeChart(model, 720, 260, 'Segoe UI', measure);
    const sepCol = layout.columns.at(-1)!;
    const hitKey = hitTestTimeChart(model, layout, [sepCol.x + sepCol.width / 2, layout.top + 20]);
    const parsed = parseTimeChartHit(hitKey);
    assert.ok(parsed);
    const installed: InstalledPicture = {
      id: 1,
      owner: 'army-losses:s:1:b:GER',
      generation: { seek: 0, reset: 0, binding: 1 },
      hour: displayedHour(H1941_SEP15),
      factRevisions: {},
      validInterval: [H1941_SEP15, H1941_SEP15],
      through: H1941_SEP15,
      viewport: { width: 720, height: 260, dpr: 1, scrollTop: 0, scrollLeft: 0 },
      optionsRevision: '1',
      themeRevision: '1',
      actionKeys: new Map(),
      accessibleDescription: null,
    };
    const action = timeChartHostAdapter.resolveHitAction?.(parsed, installed, { meta: { subject: model.subject } });
    assert.ok(action, 'timeChartHostAdapter must resolve month activation from installed picture');
    assert.equal(action.hour, H1941_SEP15, 'frame hour must be installed displayed hour (15 Sep), not 1 Sep');
    assert.equal((action.meta as { monthFirstHour?: number } | undefined)?.monthFirstHour, firstHourOfMonth(sep));
    assert.equal((action.meta as { country?: string } | undefined)?.country, 'GER');

    const emptyLive = { get: () => undefined, missing: () => 'missing' };
    const unreadDetail = resolveArmyMonthDetail(
      makeReply({
        monthDetail: { month: sep, firstHour: firstHourOfMonth(sep), lastHour: firstHourOfMonth(sep + 1) - 1, missing: 'incomplete read', armies: [] },
        availability: { armies: null, manpower: null, months: null, ownEquipment: 'incomplete read', enemyEquipment: null },
      }),
      'equipment',
      'attrition',
      emptyLive,
    );
    assert.equal(unreadDetail.kind, 'unavailable', 'unread table reports unavailable, never empty');
    if (unreadDetail.kind === 'unavailable') assert.equal(unreadDetail.reason, 'incomplete read');

    const withTotal = resolveArmyMonthDetail(
      makeReply({
        monthDetail: {
          month: sep,
          firstHour: firstHourOfMonth(sep),
          lastHour: firstHourOfMonth(sep + 1) - 1,
          missing: null,
          armies: [
            {
              army: { reference: 'a1', ordinal: 0, name: '1. Armee', nameMissing: null, parent: null, parentName: null, parentMissing: null },
              manpower: null,
              manpowerLiveRow: null,
              selectedTotalRaw: '350000',
              selectedLiveRow: 'sel:a1',
              selectedMissing: null,
              ledgers: [],
            },
            {
              army: { reference: 'a2', ordinal: 1, name: '2. Armee', nameMissing: null, parent: null, parentName: null, parentMissing: null },
              manpower: null,
              manpowerLiveRow: null,
              selectedTotalRaw: null,
              selectedLiveRow: null,
              selectedMissing: 'Combat Log equipment of this hour was not read',
              ledgers: [],
            },
          ],
        },
      }),
      'equipment',
      'attrition',
      { get: key => (key === liveKey('sel:a1', 'amount') ? 500000n : undefined), missing: () => 'missing' },
    );
    assert.equal(withTotal.kind, 'rows');
    if (withTotal.kind === 'rows') {
      assert.equal(withTotal.rows[0]?.equipmentText, '5');
      assert.equal(withTotal.rows[1]?.equipmentText, null);
      assert.equal(withTotal.rows[1]?.equipmentMissing, 'Combat Log equipment of this hour was not read');
    }
  });
});
