import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { chromium } from 'playwright-core';
import { fixture, contextOptions, deterministicPage, launchOwnedBrowser } from './settings.mjs';
import { outsideRepository, resolveUI, writeJSON } from './cache.mjs';
import { startUI } from './server.mjs';
import { createApp } from './app.mjs';
import { captureWidget } from './capture.mjs';
import { compareCaptures } from './compare.mjs';
import { captureCases, readMatrix } from './matrix.mjs';

if (!(process.env.BATTLEPLAN_OWNED_JOB ?? process.env.HOI4_OWNED_JOB)) throw Error('Launch through python ui/e2e/owned_job.py node ui/e2e/run.mjs');
const [command, ...args] = process.argv.slice(2);
if (command !== 'compare' || args.length !== 2) throw Error('Usage: compare <prepared-instrumented-worktree> <prepared-instrumented-worktree>');
const config = { ...fixture, cache: outsideRepository(fixture.cache) }, started = performance.now(), phases = {};
const matrix = await readMatrix(config.matrix);
const inputs = await Promise.all(args.map(resolveUI));
const browser = await launchOwnedBrowser(chromium);
async function capture(ui, role) {
  const began = performance.now(), output = path.join(config.cache, `${Date.now()}-${role}-${randomUUID()}`);
  const manifest = { complete: false, ui, matrix, items: [] };
  await writeJSON(path.join(output, 'manifest.json'), manifest);
  let server;
  try {
    server = await startUI(ui, config, output);
    manifest.nativeInputs = server.nativeInputs;
    console.log(`${role} server ready: ${server.url}`);
    const appConfig = { ...config, url: server.url };
    async function withPage(run) {
      const context = await browser.newContext(contextOptions);
      try { const page = await context.newPage(); await deterministicPage(page); return await run(page); }
      finally { await context.close(); }
    }
    const catalogue = await withPage(async page => { const app = createApp(page, appConfig); await app.prepare(); return app.registry(); });
    const cases = captureCases(catalogue, matrix, config.hour);
    let next = 0;
    async function worker() {
      while (next < cases.length) {
        const index = next++, scenario = cases[index], kind = scenario.kind;
        const item = await withPage(page => captureWidget({ page,
          app: createApp(page, { ...appConfig, hour: scenario.hour, scenario: matrix ? scenario : undefined }),
          kind, directory: output, ...(matrix ? { scenario, fileStem: String(index) } : {}) }));
        manifest.items[index] = item;
        console.log(`${role} ${kind}: ${item.status} (${Math.round(item.wallMs)} ms)${item.error ? ` ${item.error}` : ''}`);
      }
    }
    const workers = await Promise.allSettled([worker(), worker()]);
    const failed = workers.find(result => result.status === 'rejected');
    if (failed) throw failed.reason;
    manifest.complete = manifest.items.every(item => item.status === 'captured');
  } finally {
    await server?.close(); manifest.wallMs = performance.now() - began;
    await writeJSON(path.join(output, 'manifest.json'), manifest);
  }
  phases[role] = manifest.wallMs;
  console.log(JSON.stringify({ phase: role, wallMs: manifest.wallMs, complete: manifest.complete, output }));
  return output;
}
const base = await capture(inputs[0], 'baseline'), head = await capture(inputs[1], 'head');
const began = performance.now(), comparison = await compareCaptures(base, head);
phases.compare = performance.now() - began;
const result = { ...comparison, base, head, phases, wallMs: performance.now() - started };
await writeJSON(path.join(head, 'comparison.json'), result);
console.log(JSON.stringify(result, null, 2));
process.exit(comparison.pass ? 0 : 1);
