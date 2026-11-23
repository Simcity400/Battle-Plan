import { defineConfig } from 'vite';
import stockConfig from './vite.config.ts';

export default defineConfig(env => {
  const base = typeof stockConfig === 'function' ? stockConfig(env) : stockConfig;
  return {
    ...base,
    define: { ...base.define, __BATTLEPLAN_MEASURE_UNGATED__: 'true' },
  };
});
