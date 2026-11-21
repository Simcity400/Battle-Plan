import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { formatDateTime } from '../dashboard/calendar.ts';

const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null } });
const { BuildNotice, RecordingNotice, PartialRecordingBadge, partialRecordingText } =
  await vite.ssrLoadModule('/ui/src/shell/recordingNotice.tsx').finally(() => vite.close()) as typeof import('./recordingNotice.tsx');

test('partial recording has a persistent warning notice naming the failure and known hour', () => {
  const partial = { sessionId: '7', reason: 'collector allocation failed', failedHour: 1000 };
  const html = renderToStaticMarkup(createElement(RecordingNotice, { partial }));
  assert.match(html, /class="notice warning shell-recording-notice"/);
  assert.match(html, /role="status"/);
  assert.match(html, /Partial recording/);
  assert.match(html, /collector allocation failed/);
  assert.match(html, /not recorded/);
  assert.doesNotMatch(html, /[Dd]ismiss|button/);
  assert.equal(partialRecordingText(partial), partialRecordingText({ ...partial, sessionId: '8' }));
});

test('recording list marker says partial and retains its failure reason before or after loading', () => {
  const partial = { sessionId: '7', reason: 'collector allocation failed', failedHour: null };
  const html = renderToStaticMarkup(createElement(PartialRecordingBadge, { partial }));
  assert.match(html, /class="pill warning"/);
  assert.match(html, />Partial<\/span>/);
  assert.match(html, /data-tooltip="Partial recording · collector allocation failed"/);
  assert.equal(renderToStaticMarkup(createElement(PartialRecordingBadge, { partial: null })), '');
});

test('idle failure says partial without inventing an unrecorded hour; healthy replay has no notice', () => {
  const partial = { sessionId: '7', reason: 'collector allocation failed', failedHour: null };
  assert.equal(partialRecordingText(partial), 'Partial recording · collector allocation failed');
  assert.equal(renderToStaticMarkup(createElement(RecordingNotice, { partial: null })), '');
  assert.equal(renderToStaticMarkup(createElement(RecordingNotice)), '');
});

test('damage notice names skipped parts and their known date range', () => {
  const damaged = { parts: 2, firstHour: 1000, lastHour: 1002, reasons: ['block checksum mismatch'] };
  const html = renderToStaticMarkup(createElement(RecordingNotice, { damaged }));
  assert.match(html, /class="notice warning shell-recording-notice"/);
  assert.ok(html.includes(`Damaged or modified recording · 2 parts couldn&#x27;t be read · ${formatDateTime(1000)} – ${formatDateTime(1002)}`));
  assert.doesNotMatch(html, /block checksum mismatch/);
  const unknown = renderToStaticMarkup(createElement(RecordingNotice, { damaged: { ...damaged, parts: 1, firstHour: null, lastHour: null } }));
  assert.match(unknown, /1 part couldn&#x27;t be read<\/span>/);
  const changed = renderToStaticMarkup(createElement(RecordingNotice, { damaged: { ...damaged, parts: 0, firstHour: null, lastHour: null } }));
  assert.match(changed, />Damaged or modified recording<\/span>/);
});

test('damage and partial status share one banner and one Damaged pill with technical reasons', () => {
  const damaged = { parts: 1, firstHour: null, lastHour: null, reasons: ['Branch 1 Countries: invalid row'] };
  const partial = { sessionId: '7', reason: 'collector allocation failed', failedHour: null };
  const notice = renderToStaticMarkup(createElement(RecordingNotice, { damaged, partial }));
  assert.equal((notice.match(/shell-recording-notice/g) ?? []).length, 1);
  assert.match(notice, /couldn&#x27;t be read · Partial recording · collector allocation failed/);
  const badge = renderToStaticMarkup(createElement(PartialRecordingBadge, { damaged, partial }));
  assert.equal((badge.match(/class="pill warning"/g) ?? []).length, 1);
  assert.match(badge, />Damaged<\/span>/);
  assert.match(badge, /Branch 1 Countries: invalid row/);
  assert.match(badge, /Partial recording · collector allocation failed/);
});

test('the recording notice names the recording by its file name', () => {
  const path = 'C:/Germany 1936–1941.battleplan', partial = { sessionId: '7', reason: 'The game closed.', failedHour: null };
  const markup = renderToStaticMarkup(createElement(RecordingNotice, { partial, path }));
  assert.match(markup, /<strong[^>]*>Germany 1936–1941<\/strong>/);
  assert.match(markup, /data-tooltip="Germany 1936–1941.battleplan"/);
  assert.match(markup, /Partial recording · The game closed\./);
});

test('game build warnings take one header pill, every warning in full on hover', () => {
  const warnings = ['Unrecognised game build: synthetic', 'Synthetic table withheld'];
  const html = renderToStaticMarkup(createElement(BuildNotice, { warnings }));
  assert.match(html, /^<span class="pill warning shell-build-notice"/);
  assert.match(html, />Game build · 2 notes<\/span>/);
  assert.ok(html.includes('data-tooltip="Unrecognised game build: synthetic\nSynthetic table withheld"'));
  assert.match(renderToStaticMarkup(createElement(BuildNotice, { warnings: warnings.slice(0, 1) })), />Game build<\/span>/);
  assert.equal(renderToStaticMarkup(createElement(BuildNotice, { warnings: [] })), '');
});
