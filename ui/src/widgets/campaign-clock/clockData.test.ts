import test from 'node:test';
import assert from 'node:assert/strict';
import { OVERLAP_REASON, OVERLAP_WARNING } from './clockData.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { CLOCK_COLUMNS, CLOCK_TABLE_BASE, campaignClockProjection } from './clockProjection.ts';
import { parseFollowed } from '../../test-support/followMessage.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { buildColumnLayouts } from '../../families/table/layout.ts';
import { deriveTableAudit, resolveTableRows } from '../../families/table/model.ts';
import { tableSettingsOf } from '../../families/table/settings.ts';
import { pictureError } from '../../shell/perf.ts';

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
