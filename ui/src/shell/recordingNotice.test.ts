import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

// Node strips TypeScript but needs the UI toolchain to compile JSX.
const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null } });
const { RecordingNotice, PartialRecordingBadge, partialRecordingText } =
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
