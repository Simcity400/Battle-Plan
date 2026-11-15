import assert from 'node:assert/strict';
import test from 'node:test';
import { Coverage, HISTORY_FORMAT, controlBase, everFailed, failureCause, failureKey, mergeHistory, placementKey, readHistory, slowestCards } from './coverage.mjs';

const step = (area, item, label, pass, extra = {}) => ({ area, item, label, pass, reasons: pass ? [] : ['x'], ...extra });

test('the history keeps every step that ever failed, a catalogue setup step under the card it followed', () => {
  const earlier = { format: HISTORY_FORMAT, failed: { 'controls|widget/settings|Politics: Decisions: Show': { cause: 'product', why: 'over budget' } }, slowest: ['widget/Old'], updated: 'then' };
  const merged = mergeHistory(earlier, [
    step('catalogue', 'widget/Navy size', 'add', true),
    // A run from before setup steps named their card: the add before it names it.
    step('catalogue', 'setup', 'discard', false),
    step('catalogue', 'setup', 'delete', false, { card: 'view/Ships' }),
    step('controls', 'widget/settings', 'Overview: Map: Shadows', false),
    step('controls', 'widget/settings', 'Overview: Map: Fog', true),
  ]);
  assert.deepEqual(Object.keys(merged.failed), ['catalogue|view/Ships|delete', 'catalogue|widget/Navy size|discard', 'controls|widget/settings|Overview: Map: Shadows',
    'controls|widget/settings|Politics: Decisions: Show']);
  assert.equal(merged.format, HISTORY_FORMAT);
  // A run that did not add the whole catalogue leaves the slowest cards as they were.
  assert.deepEqual(merged.slowest, ['widget/Old']);
  assert.deepEqual(mergeHistory(merged, [], { slowest: ['view/New'] }).slowest, ['view/New']);
  assert.equal(failureKey(step('pages', 'cold/Army', 'switch', false)), 'pages|cold/Army|switch');
});

test('a case ever failed when any step under its name failed, and only then', () => {
  const history = mergeHistory(null, [step('controls', 'widget/settings', 'Politics: Spending: Show categories (with Group = Comintern): Blockade back', false),
    step('catalogue', 'setup', 'discard', false, { card: 'widget/Navy size' })]);
  assert.equal(everFailed(history, 'controls', 'widget/settings', 'Politics: Spending: Show categories (with Group = Comintern): Blockade'), true);
  assert.equal(everFailed(history, 'controls', 'widget/settings', 'Politics: Spending: '), true, 'the placement holds a failed case');
  assert.equal(everFailed(history, 'controls', 'widget/settings', 'Politics: Spending: Show categories (with Group = Comintern): Focus'), false);
  assert.equal(everFailed(history, 'catalogue', 'widget/Navy size', 'discard'), true);
  assert.equal(everFailed(history, 'catalogue', 'widget/Navy', 'discard'), false, 'another card is another case');
  assert.equal(everFailed(null, 'controls', 'widget/settings', 'anything'), false);
});

test('a session with no failure history leaves no case out, as any may have failed', async () => {
  const { Session } = await import('./session.mjs');
  const session = new Session({}, 'out', 'controls');
  assert.equal(session.everFailed('controls', 'widget/settings', 'Overview: Scoreboard: Metric'), true);
  session.history = mergeHistory(null, [step('controls', 'widget/settings', 'Overview: Map: Fog', false)]);
  assert.equal(session.everFailed('controls', 'widget/settings', 'Overview: Scoreboard: Metric'), false);
  assert.equal(session.everFailed('controls', 'widget/settings', 'Overview: Map: Fog'), true);
});

test('each value of a setting is exercised once across its menu\'s states, and again wherever its case ever failed', () => {
  const cover = new Coverage();
  const chips = controlBase({ kind: 'chips', label: 'Show categories', context: '', nth: 0, chips: [{ label: 'Focus' }] });
  // Revealed under another value of its parent, with another list: the same setting.
  assert.equal(chips, controlBase({ kind: 'chips', label: 'Show categories', context: '', nth: 0, chips: [{ label: 'Focus' }, { label: 'Blockade' }] }));
  assert.deepEqual(['Focus', 'Laws'].map(value => cover.take(chips, value)), [true, true]);
  // Under the next group: Focus and Laws were covered, Blockade is new, and Laws failed before so it runs again.
  assert.deepEqual([['Focus', false], ['Laws', true], ['Blockade', false]].map(([value, failed]) => cover.take(chips, value, failed)), [false, true, true]);
  assert.equal(cover.skipped, 1);
  // A control without values is one case.
  const sw = controlBase({ kind: 'switch', label: 'Summary', nth: 0 });
  assert.deepEqual([cover.take(sw, ''), cover.take(sw, '')], [true, false]);
});

test('the quick tier exercises one value of one setting per widget', () => {
  const cover = new Coverage({ limit: 1 });
  assert.equal(cover.full, false);
  assert.equal(cover.take('select|Metric||0', 'Total'), true);
  assert.equal(cover.full, true);
  assert.equal(cover.take('select|Metric||0', 'Share'), false);
  assert.equal(cover.take('switch|Summary||0', '', true), false, 'not even a case that failed before');
});

test('a placement repeats another only with the same kind, size, settings and neighbours', () => {
  const controls = [{ kind: 'select', label: 'Metric', state: 'Total' }, { kind: 'switch', label: 'Summary', state: true }];
  const base = { kind: 'world-tension', width: 600.2, height: 300, controls, neighbours: ['map:900x500', 'scoreboard:600x300'] };
  assert.equal(placementKey(base), placementKey({ ...base, width: 599.8, neighbours: ['scoreboard:600x300', 'map:900x500'] }));
  assert.notEqual(placementKey(base), placementKey({ ...base, height: 420 }), 'another size');
  assert.notEqual(placementKey(base), placementKey({ ...base, controls: [controls[0], { ...controls[1], state: false }] }), 'another setting');
  assert.notEqual(placementKey(base), placementKey({ ...base, neighbours: ['scoreboard:600x300'] }), 'another page around it');
  assert.notEqual(placementKey(base), placementKey({ ...base, kind: 'faction-strength' }), 'another kind');
  assert.notEqual(placementKey(base), placementKey({ ...base, controls: [controls[0], { ...controls[1], disabled: true }] }), 'a setting it cannot use');
});

test('the slowest catalogue cards are the adds slowest to show content', () => {
  const adds = [['widget/A', 40], ['view/B', 900], ['widget/C', null], ['widget/D', 120]].map(([item, ms]) => ({ area: 'catalogue', item, label: 'add', ms }));
  assert.deepEqual(slowestCards([...adds, { area: 'catalogue', item: 'setup', label: 'delete', ms: 5000 }], 2), ['view/B', 'widget/D']);
});

test('a failure is the suite\'s own only when every reason is: no case, nothing to observe, or a refusal the app rightly makes', () => {
  const cause = (...reasons) => failureCause({ pass: false, reasons });
  assert.deepEqual(cause('not exercised: the sweep has no case for this control (a slider)'), { cause: 'suite', why: 'no case' });
  assert.deepEqual(cause('could not act: RuntimeError: no colour dialog with red, green and blue fields within 10 s'), { cause: 'suite', why: 'colour dialog folded' });
  assert.deepEqual(cause('could not act: its row offers no reset'), { cause: 'suite', why: 'colour never set' });
  assert.deepEqual(cause('stopped: Error: RuntimeError: no map window under screen point 640,360'), { cause: 'suite', why: 'no native input' });
  assert.deepEqual(cause('the map reports no graphics or sound state, so whether it applied the setting is unobservable'), { cause: 'suite', why: 'unobservable' });
  assert.deepEqual(cause('Decisions: This group has no members at the end of this campaign.'), { cause: 'suite', why: 'empty group chosen' });
  assert.deepEqual(cause('the recording cannot draw it: Not available in replays: the game computes this map mode itself'), { cause: 'suite', why: 'refused by name' });
  // The app was wrong or over budget, whatever else the step could not see.
  assert.deepEqual(cause('133 ms, over 100 ms'), { cause: 'product', why: 'over budget' });
  assert.deepEqual(cause('its widget is not in view', 'Event feed: still loading after 6 s'), { cause: 'product', why: 'still loading' });
  assert.deepEqual(cause('showed a loading placeholder for 811 ms (Campaign map)'), { cause: 'product', why: 'placeholder' });
  assert.deepEqual(cause('still in flight after 6 s: requests 1'), { cause: 'product', why: 'in flight' });
  assert.deepEqual(cause('no visible change after the setting: the widget took it, and nothing it shows changed'), { cause: 'product', why: 'no visible change' });
  assert.deepEqual(cause('Training queue: The training queue wasn’t recorded at this time.'), { cause: 'product', why: 'other' });
  assert.deepEqual(failureCause({ pass: false }), { cause: 'product', why: 'other' }, 'a failure that says nothing is not excused');
});

test('a step once the product\'s failure stays so; only the product\'s failures pin a case with --pin product', () => {
  const step = (label, ...reasons) => ({ area: 'controls', item: 'widget/settings', label, pass: false, reasons });
  let history = mergeHistory(null, [step('Overview: Map: Colour of Allies', 'could not act: its row offers no reset'), step('Overview: Map: Fog', '140 ms, over 100 ms')]);
  history = mergeHistory(history, [step('Overview: Map: Fog', 'not exercised: the clock stood at 3, not 4')]);
  assert.deepEqual(history.failed['controls|widget/settings|Overview: Map: Fog'], { cause: 'product', why: 'over budget' }, 'never downgraded');
  history = mergeHistory(history, [step('Overview: Map: Colour of Allies', 'Map: shows nothing')]);
  assert.equal(history.failed['controls|widget/settings|Overview: Map: Colour of Allies'].cause, 'product', 'upgraded once the product fails it');
  history = mergeHistory(history, [step('Politics: Spending: Show', 'not exercised: the sweep has no case for this control (x)')]);
  assert.equal(everFailed(history, 'controls', 'widget/settings', 'Politics: Spending: '), true);
  assert.equal(everFailed(history, 'controls', 'widget/settings', 'Politics: Spending: ', 'product'), false, 'only the suite failed it');
  assert.equal(everFailed(history, 'controls', 'widget/settings', 'Overview: Map: Fog', 'product'), true);
});

test('a history without the mark of whose failure each step was is refused by name', () => {
  assert.equal(readHistory(null), null);
  assert.throws(() => readHistory({ failed: ['controls|widget/settings|x'] }, 'history.json'), /history\.json predates the mark .*\(format 1, not 2\)/);
  const history = mergeHistory(null, []);
  assert.equal(readHistory(JSON.parse(JSON.stringify(history))).format, HISTORY_FORMAT);
});
