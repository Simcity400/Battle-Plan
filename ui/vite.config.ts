import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig, loadEnv, type Plugin } from 'vite';
import { browserBridge } from './browserBridge.ts';

function widgetChunksPlugin(): Plugin {
  return {
    name: 'bp-widget-chunks',
    transformIndexHtml: {
      order: 'post',
      handler(html, ctx) {
        if (!ctx.bundle) return html;
        const chunks: Record<string, string> = {};
        for (const item of Object.values(ctx.bundle)) {
          if (item.type !== 'chunk') continue;
          const kind = item.facadeModuleId?.replace(/\\/g, '/').match(/(?:^|\/)ui\/src\/widgets\/([^/]+)\/index\.tsx?$/)?.[1];
          if (kind) chunks[kind] = `./${item.fileName}`;
        }
        if (!Object.keys(chunks).length) return html;
        return [{ tag: 'meta', attrs: { name: 'bp-widget-chunks', content: JSON.stringify(chunks) }, injectTo: 'head' }];
      },
    },
  };
}

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, fileURLToPath(new URL('.', import.meta.url)), 'BATTLEPLAN_');
  return {
    root: fileURLToPath(new URL('.', import.meta.url)),
    plugins: [react(), widgetChunksPlugin(), browserBridge(fileURLToPath(new URL('..', import.meta.url)), env.BATTLEPLAN_RECORDINGS_DIR ?? '', env.BATTLEPLAN_GAME_DIR ?? '', env.BATTLEPLAN_USER_DIR ?? '')],
    clearScreen: false,
    server: { host: '127.0.0.1', port: 1420, strictPort: true },
    build: { outDir: 'dist', emptyOutDir: true, target: 'es2022', rolldownOptions: { input: {
      main: fileURLToPath(new URL('index.html', import.meta.url)),
      recorder: fileURLToPath(new URL('recorder.html', import.meta.url)),
    }, output: {
      chunkFileNames: chunk => {
        const kind = chunk.facadeModuleId?.replace(/\\/g, '/').match(/(?:^|\/)ui\/src\/widgets\/([^/]+)\/index\.tsx?$/)?.[1];
        return kind ? `assets/widget-${kind}-[hash].js` : 'assets/[name]-[hash].js';
      },
    } } },
  };
});
