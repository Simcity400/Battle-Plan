// Prepares the fixture recording in the running app, as its Prepare button does, and waits until it is
// ready. Prints one JSON line when it asks and one when it ends, so the host samples the app's job then.
// usage: node tools/e2e/prepare.mjs --port <port> --recording <file> --out <file.json> [--timeout-minutes 60]
import { writeFile } from 'node:fs/promises';
import { connect, sleep } from './cdp.mjs';

const flag = (name, fallback = null) => { const at = process.argv.indexOf(name); return at < 0 ? fallback : process.argv[at + 1]; };
const [port, recording, out] = [Number(flag('--port')), flag('--recording'), flag('--out')];
const limit = Number(flag('--timeout-minutes', '60')) * 60_000;
if (!port || !recording || !out) throw new Error('usage: prepare.mjs --port <port> --recording <file> --out <file.json>');
const say = event => console.log(JSON.stringify({ ...event, at: Date.now() }));

const app = await connect(port);
app.setCase('preparation');
let sequence = 0;
// A refusal or failure comes back as the host's own words: `refused: <text>` or `failed: <message>`.
const call = (command, args) => app.evaluate(`(async () => {
  try {
    const reply = await window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(args)});
    return { reply: reply instanceof ArrayBuffer || reply instanceof Uint8Array ? JSON.parse(new TextDecoder().decode(reply)) : reply };
  } catch (error) {
    return { error: error?.kind === 'refused' ? 'refused: ' + error.text : error?.kind === 'failed' ? 'failed: ' + error.message
      : typeof error === 'string' ? error : JSON.stringify(error) };
  }
})()`, 120_000).then(({ reply, error }) => { if (error !== undefined) throw new HostError(error); return reply; });
class HostError extends Error {}
const request = source => ({ sourceId: source.id, revision: source.revision, requestId: `e2e-prepare-${++sequence}` });

// The host measures from the app's launch; this bounds the wait.
const asked = Date.now();
let source;
try {
  source = await call('dashboard_open_source', { path: recording, requestId: `e2e-open-${++sequence}` });
} catch (error) {
  if (!(error instanceof HostError)) throw error;
  // The app would not open the fixture: say so plainly, as it said it.
  say({ event: 'ended', state: 'refused' });
  await writeFile(out, JSON.stringify({ recording, state: 'refused', error: `the app would not open the fixture: ${error.message}`, failedSections: [], console: app.messages }, null, 1));
  app.close();
  process.exit(0);
}
let status = await call('dashboard_preparation_status', request(source));
const result = { recording, sourceId: source.id, revision: source.revision, initial: status.state, polls: [] };
if (status.state === 'ready') throw new Error('the fixture was already prepared in a fresh profile');
// As its Prepare button would, unless opening it already started.
if (status.state === 'idle' || status.state === 'cancelled') status = await call('dashboard_prepare_source', request(source));
for (;;) {
  result.polls.push({ at: Date.now(), state: status.state, sections: status.sections.filter(section => section.state !== 'ready').length });
  if (status.state === 'ready' || status.state === 'failed' || status.state === 'cancelled') break;
  if (status.state === 'paused') { result.error = `preparation paused: ${status.pauseReason ?? 'no reason given'}`; break; }
  if (Date.now() - asked > limit) { result.error = `not ready after ${limit / 60_000} minutes (${status.state})`; break; }
  await sleep(1000);
  status = await call('dashboard_preparation_status', request(source));
}
const ended = Date.now();
say({ event: 'ended', state: status.state });
Object.assign(result, {
  state: status.state, error: result.error ?? status.error ?? null, bytes: Number(status.bytes),
  failedSections: status.sections.filter(section => section.state !== 'ready').map(section => `${section.label}: ${section.state}${section.error ? ` (${section.error})` : ''}`),
  console: app.messages,
});
await call('dashboard_release_source', { sourceId: source.id });
await writeFile(out, JSON.stringify(result, null, 1));
app.close();
