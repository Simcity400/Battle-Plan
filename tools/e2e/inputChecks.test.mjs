import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const source = readFileSync(new URL('./sweep.mjs', import.meta.url), 'utf8');

test('scrub waits for the released timeline target and fails when it is not reached', () => {
  const release = source.indexOf("await app.mouse('mouseReleased', x(.31), line.y);");
  assert.ok(release >= 0, 'the scripted scrub release is present');
  const wait = source.indexOf('scrub target', release);
  assert.ok(wait > release, 'the release is followed by a named target wait');
  assert.match(source.slice(release, wait + 300), /if \(!await s\.until\([\s\S]+?\)\)\s*throw new Error\([\s\S]*?scrub/i,
    'a failed scrub target wait stops the input sequence');
});

test('scroll tries each scrollable area and fails when none moves under the wheel', () => {
  const wheel = source.indexOf("type: 'mouseWheel'");
  const need = source.lastIndexOf('const point = await s.locate(target);', wheel);
  const before = source.indexOf('const [before, wheel, at] = await app.evaluate', need);
  assert.ok(need >= 0 && before > need, 'the baseline is captured after revealing the target');
  assert.ok(wheel >= 0, 'the scripted wheel is present');
  const wait = source.indexOf('scrolled = await s.until(', wheel), fail = source.indexOf('if (!scrolled) throw new Error(', wait);
  assert.ok(wait > wheel, 'the wheel is followed by a wait on the target');
  assert.ok(fail > wait && /moved under the wheel/.test(source.slice(fail, fail + 200)), 'a scroll no candidate makes stops the input sequence');
});

test('pause verifies that the playback clock stopped after the click', () => {
  const pause = source.indexOf("await mark('pause');");
  assert.ok(pause >= 0, 'the pause action is present');
  const end = source.indexOf("const inputs = await app.evaluate('__e2e.endInputs()')", pause);
  assert.ok(end > pause, 'pause verification precedes input recording');
  assert.match(source.slice(pause, end), /if \(!await s\.until\([^\n]+(?:playing|Pause playback|Play recording|header-play)/i,
    'the pause action waits for a stopped playback state');
});

test('settlement probing records a bounded failure instead of throwing after timeout', () => {
  const settle = source.indexOf('const settleInputs = async () =>');
  const end = source.indexOf("await mark('rewind');", settle);
  assert.ok(settle >= 0 && end > settle, 'the shared settlement helper is present');
  const helper = source.slice(settle, end);
  assert.match(helper, /try\s*\{[\s\S]*app\.evaluate\(unsettled\)[\s\S]*\}\s*catch/,
    'a timed-out settlement probe is converted into evidence');
  assert.match(helper, /settlement probe failed/, 'the probe failure is named');
});

test('pin restore reopens the widget that was pinned and waits for its time controls', () => {
  const pin = source.indexOf("__e2e.setPinWidget(");
  const restore = source.indexOf("if (pinApplied)", pin);
  assert.ok(pin >= 0 && restore > pin, 'the pin action records the widget and later restores it');
  const body = source.slice(restore, source.indexOf('return {', restore));
  assert.match(body, /data-widget[^;]*pinWidget|pinnedConfig/, 'restore targets the widget recorded by the pin action');
  assert.match(body, /await s\.until\([\s\S]*?time[\s\S]*?\)/, 'restore waits for the time controls after opening the menu');
});

test('picture audits are consumed through bounded evaluate chunks', () => {
  const audit = source.indexOf("window.__perf.auditPicturesBatch(64)");
  assert.ok(audit >= 0, 'the playback phase runs the picture audit');
  const start = source.lastIndexOf('while (!auditDone', audit);
  const end = source.indexOf("const summary = await app.evaluate('window.__perf.stop()'", audit);
  assert.ok(start >= 0, 'the picture audit has a bounded loop');
  assert.ok(end > audit, 'the picture audit completes before perf stop');
  assert.match(source.slice(start, end), /while|for\s*\(/, 'the audit result is collected in more than one evaluate when needed');
  assert.match(source.slice(start, end), /auditPicturesBatch\([^)]*\d+/, 'each audit evaluate has a bounded picture count');
});
