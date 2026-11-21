import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null } });
const { RecordingNotice, PartialRecordingBadge, partialRecordingText, gapText, recordedHour } =
  await vite.ssrLoadModule('/ui/src/shell/recordingNotice.tsx').finally(() => vite.close()) as typeof import('./recordingNotice.tsx');

test('partial recording has a persistent warning notice naming the hour that couldn’t be recorded, not its raw reason', () => {
  const partial = { sessionId: '7', reason: 'collector allocation failed', failedHour: 1000 };
  const html = renderToStaticMarkup(createElement(RecordingNotice, { partial }));
  assert.match(html, /class="notice warning shell-recording-notice"/);
  assert.match(html, /role="status"/);
  assert.match(html, /role="status"><svg[^>]*>.*?<\/svg><span>/, 'the notice leads with the alert icon');
  assert.ok(html.includes(`Partial recording · ${recordedHour(1000)} couldn’t be recorded`));
  assert.doesNotMatch(html, /nothing after/, 'a later session or reload may have recorded more');
  assert.doesNotMatch(html, /collector allocation failed/, 'raw reasons live in Diagnostics');
  assert.doesNotMatch(html, /[Dd]ismiss|button/);
  assert.equal(partialRecordingText(partial), partialRecordingText({ ...partial, sessionId: '8' }));
});

test('recording list marker says partial and explains it in user terms before or after loading', () => {
  const partial = { sessionId: '7', reason: 'collector allocation failed', failedHour: null };
  const html = renderToStaticMarkup(createElement(PartialRecordingBadge, { partial }));
  assert.match(html, /class="pill warning"/);
  assert.match(html, />Partial<\/span>/);
  assert.match(html, /data-tooltip="Partial recording · part of the recording couldn’t be saved"/);
  assert.equal(renderToStaticMarkup(createElement(PartialRecordingBadge, { partial: null })), '');
});

test('idle failure says partial without inventing an unrecorded hour; healthy replay has no notice', () => {
  const partial = { sessionId: '7', reason: 'collector allocation failed', failedHour: null };
  assert.equal(partialRecordingText(partial), 'Partial recording · part of the recording couldn’t be saved');
  assert.equal(renderToStaticMarkup(createElement(RecordingNotice, { partial: null })), '');
  assert.equal(renderToStaticMarkup(createElement(RecordingNotice)), '');
});

test('damage notice names skipped parts and their known date range', () => {
  const damaged = { parts: 2, firstHour: 1000, lastHour: 1002, reasons: ['block checksum mismatch'] };
  const html = renderToStaticMarkup(createElement(RecordingNotice, { damaged }));
  assert.match(html, /class="notice warning shell-recording-notice"/);
  assert.ok(html.includes(`Damaged or modified recording · 2 parts couldn’t be read · ${recordedHour(1000)} – ${recordedHour(1002)}`));
  assert.doesNotMatch(html, /block checksum mismatch/);
  const unknown = renderToStaticMarkup(createElement(RecordingNotice, { damaged: { ...damaged, parts: 1, firstHour: null, lastHour: null } }));
  assert.match(unknown, /1 part couldn’t be read<\/span>/);
  const changed = renderToStaticMarkup(createElement(RecordingNotice, { damaged: { ...damaged, parts: 0, firstHour: null, lastHour: null } }));
  assert.match(changed, />Damaged or modified recording<\/span>/);
});

test('damage and partial status share one banner and one Damaged pill, with technical reasons left to Diagnostics', () => {
  const damaged = { parts: 1, firstHour: null, lastHour: null, reasons: ['Branch 1 Countries: invalid row'] };
  const partial = { sessionId: '7', reason: 'collector allocation failed', failedHour: null };
  const notice = renderToStaticMarkup(createElement(RecordingNotice, { damaged, partial }));
  assert.equal((notice.match(/shell-recording-notice/g) ?? []).length, 1);
  assert.match(notice, /couldn’t be read · Partial recording · part of the recording couldn’t be saved/);
  const badge = renderToStaticMarkup(createElement(PartialRecordingBadge, { damaged, partial }));
  assert.equal((badge.match(/class="pill warning"/g) ?? []).length, 1);
  assert.match(badge, />Damaged<\/span>/);
  assert.doesNotMatch(badge, /Branch 1 Countries|collector allocation failed/);
  assert.match(badge, /Partial recording · part of the recording couldn’t be saved/);
});

test('the recording notice names the recording without its file extension', () => {
  const path = 'C:/Germany 1936–1941.battleplan', partial = { sessionId: '7', reason: 'The game closed.', failedHour: null };
  const markup = renderToStaticMarkup(createElement(RecordingNotice, { partial, path }));
  assert.match(markup, /<strong>Germany 1936–1941<\/strong>/);
  assert.doesNotMatch(markup, /\.battleplan/, 'no file extension, even in a tooltip');
  assert.match(markup, /Partial recording · part of the recording couldn’t be saved/);
});

test('a recording gap says what is missing and when, without its raw reason', () => {
  assert.equal(gapText({ firstHour: 1000, lastHour: 1036 }), `Not recorded · ${recordedHour(1000)} – ${recordedHour(1036)}`);
  assert.equal(gapText({ firstHour: 1000, lastHour: 1000 }), `Not recorded · ${recordedHour(1000)}`);
  assert.doesNotMatch(recordedHour(43_800_000 + 24 * 400 + 13), /·/, 'one hour reads as a date and a 24-hour time');
  assert.match(recordedHour(43_800_000 + 24 * 400 + 13), /^\d{1,2} [A-Z][a-z]{2} \d+ \d\d:00$/);
});
