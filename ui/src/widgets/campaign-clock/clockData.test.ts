import test from 'node:test';
import assert from 'node:assert/strict';
import { INCOMPLETE_WARNING, OVERLAP_REASON, OVERLAP_WARNING } from './clockData.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { CLOCK_COLUMNS, CLOCK_TABLE_BASE, campaignClockDeclaration, campaignClockProjection } from './clockProjection.ts';
import { parseFollowed } from '../../test-support/followMessage.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { buildColumnLayouts } from '../../families/table/layout.ts';
import { deriveTableAudit, resolveTableRows } from '../../families/table/model.ts';
import { tableSettingsOf } from '../../families/table/settings.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import { pictureError } from '../../shell/perf.ts';

function floatBits(value: number): bigint {
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, value, true);
  return view.getBigInt64(0, true);
}

test('overlapping campaign-clock refusal carries the same warning on followed and exact paths', () => {
  const settings = tableSettingsOf(undefined, CLOCK_COLUMNS, CLOCK_TABLE_BASE);
  const cols = buildColumnLayouts(CLOCK_COLUMNS, settings);
  const exactReply = {
    available: true,
    recordedSeconds: null,
    playingSeconds: null,
    pausedSeconds: null,
    complete: false,
    warnings: [OVERLAP_WARNING],
  };
  const followedAnswer = parseFollowed(message(100, [{ id: 1, answer: { notRecorded: OVERLAP_REASON } }])).answers[0]!.answer;
  const exactModel = campaignClockProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['main', exactReply]]) }), {});
  const followedModel = campaignClockProjection.project(createBoundedReadView({ hour: 100, answers: new Map([['main', followedAnswer]]) }), {});
  assert.deepEqual(followedModel.warnings, [OVERLAP_WARNING]);
  const exactAudit = deriveTableAudit({ model: exactModel, flatRows: resolveTableRows(exactModel, settings), visibleColumns: cols });
  const followedAudit = deriveTableAudit({ model: followedModel, flatRows: resolveTableRows(followedModel, settings), visibleColumns: cols });
  assert.equal(pictureError('table', followedAudit, { values: new Map(exactAudit.map(r => [r.key, r.value!])), ordered: true }), null);
});

test('incomplete coverage keeps measured clock totals and labels both paths honestly', () => {
  const settings = tableSettingsOf(undefined, CLOCK_COLUMNS, CLOCK_TABLE_BASE);
  const cols = buildColumnLayouts(CLOCK_COLUMNS, settings);
  const exactReply = {
    available: true,
    recordedSeconds: 960,
    playingSeconds: 960,
    pausedSeconds: 0,
    complete: false,
    warnings: [INCOMPLETE_WARNING],
  };
  const followedAnswer = parseFollowed(message(115, [{
    id: 1,
    answer: {
      keys: [0n],
      columns: 4,
      values: [floatBits(960), floatBits(960), floatBits(0), 0n],
      names: ['clock'],
    },
  }])).answers[0]!.answer;
  const exactModel = campaignClockProjection.project(createBoundedReadView({ hour: 115, panels: new Map([['main', exactReply]]) }), { firstHour: 100 });
  const followedModel = campaignClockProjection.project(createBoundedReadView({ hour: 115, answers: new Map([['main', followedAnswer]]) }), { firstHour: 100 });
  for (const model of [exactModel, followedModel]) {
    assert.equal(model.clockSummary?.totalText, '0:16:00');
    assert.equal(model.clockSummary?.coverageLabel, 'Incomplete coverage');
    assert.equal(model.clockSummary?.complete, false);
    assert.deepEqual(model.warnings, [INCOMPLETE_WARNING]);
    const coverageRow = model.rows.find(row => row.key === 'coverage');
    assert.deepEqual(coverageRow?.cells.time?.value, { kind: 'known', value: 'Incomplete coverage' });
  }
  const exactAudit = deriveTableAudit({ model: exactModel, flatRows: resolveTableRows(exactModel, settings), visibleColumns: cols });
  const followedAudit = deriveTableAudit({ model: followedModel, flatRows: resolveTableRows(followedModel, settings), visibleColumns: cols });
  assert.equal(pictureError('table', followedAudit, { values: new Map(exactAudit.map(r => [r.key, r.value!])), ordered: true }), null);
});

test('range mode uses the selected range start instead of the branch start after a gap', () => {
  const firstAfterGap = 110 * 24;
  const lastAfterGap = 115 * 24;
  const branchFirst = 100 * 24;
  const rangeOpts = campaignClockDeclaration.projectionOptions!({
    branch: { id: '4', firstHour: branchFirst, lastHour: lastAfterGap },
    widget: { time: { mode: 'range' } },
    staticSpan: [firstAfterGap, lastAfterGap],
  } as never);
  assert.equal(rangeOpts.firstHour, firstAfterGap);
  const followOpts = campaignClockDeclaration.projectionOptions!({
    branch: { id: '4', firstHour: branchFirst, lastHour: lastAfterGap },
    widget: { time: { mode: 'follow' } },
    staticSpan: [firstAfterGap, lastAfterGap],
  } as never);
  assert.equal(followOpts.firstHour, branchFirst);
  const reply = {
    available: true,
    recordedSeconds: 360,
    playingSeconds: 360,
    pausedSeconds: 0,
    complete: true,
    warnings: [],
  };
  const model = campaignClockProjection.project(
    createBoundedReadView({ hour: lastAfterGap, panels: new Map([['main', reply]]) }),
    rangeOpts,
  );
  assert.equal(model.clockSummary?.coverageLabel, 'Every hour recorded');
  assert.equal(model.clockSummary?.spanText, `${formatDateOnly(firstAfterGap)} \u2013 ${formatDateOnly(lastAfterGap)} in game`);
});
