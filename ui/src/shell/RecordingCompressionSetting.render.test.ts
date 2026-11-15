import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { CompressionChoiceProps } from './RecordingCompressionSetting';

test('recording compression says what Low and High cost, and offers Auto Compress only while Low', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { CompressionChoice } = await vite.ssrLoadModule('/ui/src/shell/RecordingCompressionSetting.tsx') as { CompressionChoice: (props: CompressionChoiceProps) => unknown };
    const render = (settings: CompressionChoiceProps['settings']) => renderToStaticMarkup(createElement(CompressionChoice as never, {
      settings, loading: false, saving: false, canCompress: true, canAutoCompress: true, onCompression() {}, onAutoCompress() {},
    } satisfies CompressionChoiceProps));
    const text = (html: string) => html.replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ');
    const low = render({ liveCompression: 'low', autoCompress: true });
    assert.match(text(low), /Compression Low: the recorder keeps pace with the game\. High: a smaller file, and the recorder can fall behind at full speed\./);
    assert.match(text(low), /Auto Compress After saving, compresses the recording to the smaller High size in the background; usable at once meanwhile\./);
    assert.match(low, /<input type="checkbox" role="switch" aria-label="Auto Compress" checked=""/, 'on by default');
    assert.doesNotMatch(render({ liveCompression: 'low', autoCompress: false }), /aria-label="Auto Compress" checked=""/);
    const high = render({ liveCompression: 'high', autoCompress: true });
    assert.match(text(high), /can fall behind at full speed\./);
    assert.doesNotMatch(high, /Auto Compress/, 'no effect while High, so not shown');
    assert.doesNotMatch(render(null), /Auto Compress/, 'not shown before the settings are read');
  } finally {
    await vite.close();
  }
});
