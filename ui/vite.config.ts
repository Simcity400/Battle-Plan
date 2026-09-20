import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig, loadEnv } from 'vite';
import { browserBridge } from './browserBridge.ts';

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, fileURLToPath(new URL('.', import.meta.url)), 'HOI4_');
  return {
    root: fileURLToPath(new URL('.', import.meta.url)),
    plugins: [react(), browserBridge(fileURLToPath(new URL('..', import.meta.url)), env.BATTLE_PLAN_RECORDINGS_DIR ?? '', env.BATTLE_PLAN_GAME_DIR ?? '', env.BATTLE_PLAN_USER_DIR ?? '')],
    clearScreen: false,
    server: { host: '127.0.0.1', port: 1420, strictPort: true },
    build: { outDir: 'dist', emptyOutDir: true, target: 'es2022' },
  };
});
