import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { CompressionChoiceProps } from './RecordingCompressionSetting';
import { AUTO_COMPRESS_NOTE, COMPRESSION_NOTE } from '../recorder/compression.ts';

test('recording compression is a Low / High choice that says what each costs, and offers the Auto compress switch only while Low', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { CompressionChoice } = await vite.ssrLoadModule('/ui/src/shell/RecordingCompressionSetting.tsx') as { CompressionChoice: (props: CompressionChoiceProps) => unknown };
    const render = (settings: CompressionChoiceProps['settings'], canCompress = true) => renderToStaticMarkup(createElement(CompressionChoice as never, {
      settings, loading: false, saving: false, canCompress, canAutoCompress: true, onCompression() {}, onAutoCompress() {},
    } satisfies CompressionChoiceProps));
    const text = (html: string) => html.replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ');
    const low = render({ liveCompression: 'low', autoCompress: true });
    assert.ok(text(low).includes(`Recording compression ${COMPRESSION_NOTE}`));
    assert.match(low, /aria-label="About recording compression" data-tooltip="Compression runs outside the game and applies to what is recorded next\."/);
    assert.match(low, /id="recording-auto-compress"/, 'Auto compress has its own search target');
    assert.doesNotMatch(text(low), /block/, 'no recording-format terms');
    assert.match(low, /<div class="segmented" role="group" aria-label="Recording compression"><button[^>]*aria-pressed="true"[^>]*>Low<\/button><button[^>]*aria-pressed="false"[^>]*>High<\/button><\/div>/);
    assert.ok(text(low).includes(`Auto compress ${AUTO_COMPRESS_NOTE}`));
    assert.match(low, /<input type="checkbox" role="switch" aria-label="Auto compress" checked=""/, 'on by default');
    assert.doesNotMatch(render({ liveCompression: 'low', autoCompress: false }), /aria-label="Auto compress" checked=""/);
    const high = render({ liveCompression: 'high', autoCompress: true });
    assert.match(high, /aria-pressed="true"[^>]*>High<\/button>/);
    assert.doesNotMatch(high, /aria-label="Auto compress"/, 'no switch while High, where it has no effect');
    assert.match(text(high), /Auto compress Only with Low compression\. High recordings are already compressed as they’re made\. Low only/, 'the dependency is explained instead');
    assert.match(high, /id="recording-auto-compress"/);
    const unread = render(null);
    assert.doesNotMatch(unread, /Auto compress/i, 'not shown before the settings are read');
    assert.doesNotMatch(unread, /segmented/, 'no choice is shown as made before the setting is read');
    assert.match(text(unread), /Unavailable/);
    assert.match(text(render(null, false)), /Desktop only/, 'without the desktop app the cause is named');
  } finally {
    await vite.close();
  }
});
