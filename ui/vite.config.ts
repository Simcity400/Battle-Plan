import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig, loadEnv } from 'vite';
import { browserBridge } from './browserBridge.ts';

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, fileURLToPath(new URL('.', import.meta.url)), 'BATTLEPLAN_');
  const measureUngated = process.env.BATTLEPLAN_MEASURE_UNGATED === '1';
  return {
    root: fileURLToPath(new URL('.', import.meta.url)),
    define: { __BATTLEPLAN_MEASURE_UNGATED__: JSON.stringify(measureUngated) },
    plugins: [react(), browserBridge(fileURLToPath(new URL('..', import.meta.url)), env.BATTLEPLAN_RECORDINGS_DIR ?? '', env.BATTLEPLAN_GAME_DIR ?? '', env.BATTLEPLAN_USER_DIR ?? '')],
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
