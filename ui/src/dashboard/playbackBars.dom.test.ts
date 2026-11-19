import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('place names landing after the block is drawn: the rows in view are asked first, in a read of their own, and show their names within a frame of it landing, paused and playing', async t => {
  await withNativePage('place-names', '/ui/src/widgets/battles/placeNames.dom.fixture.tsx', page => page.stage('place names', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.placeNames.${expression}`);
    type Shown = { province: number; place: string }[];
    type Ask = { land: boolean; provinces: number[]; landed: boolean };
    const shown = async () => ({ land: await act<Shown>('landShown()'), divisions: await act<Shown>('divisionsShown()') });
    await act('mount()');
    const paused = await shown();
    assert.ok(paused.land.length >= 5 && paused.divisions.length >= 5, `both lists draw their rows: ${JSON.stringify(paused)}`);
    assert.ok(paused.land.every(row => row.place === 'Loading…') && paused.divisions.every(row => row.place === 'Loading place…'), `no name has landed: ${JSON.stringify(paused)}`);
    const asked = await act<Ask[]>('asks()');
    const first = (land: boolean) => asked.find(ask => ask.land === land);
    for (const [land, rows] of [[true, paused.land], [false, paused.divisions]] as const) {
      const ask = first(land);
      assert.ok(ask, `${land ? 'land battles' : 'the division log'} asked its places`);
      assert.ok(rows.every(row => ask.provinces.includes(row.province)) && ask.provinces.length <= 24,
        `${land ? 'land battles' : 'the division log'}: the rows in view are asked first, in a read of their own: ${ask.provinces.length} places from ${ask.provinces.slice(0, 8).join(', ')}`);
    }
    assert.equal(asked.length, 2, 'the rest wait for the rows in view to be named');
    // Paused: the reads of the rows in view land; the frame they land in takes them and the next shows the names (`frames`
    // counts the frame of the landing), at a commit a widget.
    type Landing = { frames: number; commits: Record<string, number> };
    const landing = await act<Landing>(`landAndCount([${asked.indexOf(first(true)!)}, ${asked.indexOf(first(false)!)}])`);
    t.diagnostic(`paused: named ${landing.frames} frames after landing, commits ${JSON.stringify(landing.commits)}`);
    assert.ok(landing.frames <= 1, `paused, every row in view is named in the frame its read lands: ${landing.frames} frames ${JSON.stringify(await shown())}`);
    assert.ok((landing.commits.land ?? 0) <= 1 && (landing.commits.divisions ?? 0) <= 1, `a landing costs a commit a widget: ${JSON.stringify(landing.commits)}`);
    await act('settle(4)');
    const rest = (await act<Ask[]>('asks()')).slice(2);
    assert.ok(rest.length >= 2 && rest.every(ask => !ask.landed), `the rest of the places are asked once the rows in view are named: ${rest.length} reads`);
    // Playing: battles begin at the top of the view, their places in the reads still out; those land mid-play and the
    // rows show their names in the next frame, as the clock moves 15 hours a frame.
    await act('play(48, 15)');
    const playing = await shown();
    assert.ok(playing.land.some(row => row.province > 300 && row.place === 'Loading…'), `battles begun while playing wait for their read: ${JSON.stringify(playing.land)}`);
    const reads = await act<Ask[]>('asks()');
    const out = reads.flatMap((ask, index) => ask.landed ? [] : [index]);
    const played = await act<Landing>(`landAndCount([${out.join(', ')}], 15)`);
    t.diagnostic(`playing: named ${played.frames} frames after landing, commits ${JSON.stringify(played.commits)}`);
    assert.ok(played.frames <= 1, `playing, every row in view is named in the frame its read lands, painted: ${played.frames} frames ${JSON.stringify(await shown())}`);
    assert.ok((played.commits.land ?? 0) <= 1 && (played.commits.divisions ?? 0) <= 1, `playing, a landing is painted, not committed: ${JSON.stringify(played.commits)}`);
    await act('play(30, 15)');
    const after = await shown();
    assert.ok([...after.land, ...after.divisions].filter(row => row.province <= 300 || row.province > 1000).every(row => row.place === `Province ${row.province}`),
      `once React has taken the names, the rows still show them: ${JSON.stringify(after)}`);
    await act('pause()');
  }, 120_000));
});

// Run 13 (Navy, Army at 1,440 h/s): a block drew through every window held, so the next began two or three windows on,
// in a window nothing had asked; it was asked at the head with the one after it (naval-battles, ships-sunk, then their
// places, details and terrain).
test('following at 1,440 h/s over windows of 720 hours, every window, place and terrain read is asked ahead of the head, each once', async t => {
  await withNativePage('battle-windows', '/ui/src/widgets/battles/battleWindows.dom.fixture.tsx', page => page.stage('battle windows', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.battleWindows.${expression}`);
    await act('mount()');
    assert.ok(await act<number>('rows()') > 0, 'the log draws its battles');
    await act('play(600, 15)');
    type Asked = { product: string; window: number | null; from: number | null; head: number };
    const played = await act<Asked[]>('played()');
    const windows = played.filter(asked => asked.product === 'land-battles');
    t.diagnostic(`to hour ${await act<number>('head()')}: ${windows.length} windows asked (${windows.map(asked => `${asked.window}@${asked.head}`).join(', ')}); ${played.length} requests`);
    assert.ok(windows.length >= 10, `the windows the head crossed were asked while playing: ${windows.length}`);
    const atHead = played.filter(asked => asked.product !== 'follow' && asked.product !== 'follow-clock' && asked.product !== 'unfollow' && (asked.from === null || asked.from <= asked.head));
    assert.deepEqual(atHead, [], 'no request asks for values at the head while playing');
    assert.equal(new Set(windows.map(asked => asked.window)).size, windows.length, 'no window asked twice');
    await act('pause()');
  }, 120_000));
});

// Run 16 (Army at 1,440 h/s, 60% of frames dropped): every block of Land battles attached its rows' contents again
// (about 3,000 elements a commit), because each flag was keyed by the hour its side was read, which moves every block,
// and every row's cells rendered again for numbers the painter already writes.
test('Land battles at a world war\'s density, following at 1,440 h/s: rows are bound by the frame loop into the same boxes, never mounted, each state once', async t => {
  await withNativePage('land-playback', '/ui/src/widgets/land-battles/landPlayback.dom.fixture.tsx', page => page.stage('land playback', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.landPlayback.${expression}`);
    await act('mount()');
    assert.ok(await act<number>('rows()') > 0, 'the log draws its battles');
    const played = await act<{ commits: { ms: number; kept: number; redrawn: number }[]; why: Record<string, number>; errors: string[]; pooled: boolean; sameBoxes: boolean;
      bindings: number; boundStates: number }>('play(300, 15)');
    const sum = (pick: (commit: { ms: number; kept: number; redrawn: number }) => number) => played.commits.reduce((total, commit) => total + pick(commit), 0);
    t.diagnostic(`${played.commits.length} commits, ${(sum(commit => commit.ms) / Math.max(1, played.commits.length)).toFixed(1)} ms each in React; rows kept ${sum(commit => commit.kept)}, elements drawn again inside them ${sum(commit => commit.redrawn)}; ${played.bindings} bindings of ${played.boundStates} states; why ${JSON.stringify(played.why)}`);
    assert.deepEqual(played.errors, [], 'every request was served');
    assert.ok(played.commits.length >= 5 && sum(commit => commit.kept) > 0, `blocks drew rows the block before had drawn: ${played.commits.length} commits`);
    assert.equal(sum(commit => commit.redrawn), 0, 'nothing inside a row a block keeps is attached again (a battle\'s new state aside)');
    assert.ok(played.pooled && played.sameBoxes, 'following, the rows are the list\'s pooled boxes, the same elements from the first frame to the last');
    assert.deepEqual(Object.keys(played.why).filter(cause => cause.startsWith('BattleCells')), [], 'no battle row is rendered by React while following');
    assert.ok(played.bindings > 0 && played.bindings <= 2 * played.boundStates,
      `each state of a row is bound about once (again only when its names land): ${played.bindings} bindings of ${played.boundStates} states`);
  }, 180_000));
});

// Run 14: a cold Economy seek read the campaign (equipment produced 3.6 to 4.7 s); its replies, and those of trade flows,
// production spending, naval supremacy and aircraft losses, are windows of hours now. Following, each widget asks the next
// window ahead of the head once per window crossed and commits only as a window lands (naval supremacy and trade flows:
// also as their view moves by a month), never per hour.
test('the widgets whose replies are windows of hours, following at 1,440 h/s, ask each next window ahead of the head once and commit nothing per hour', async t => {
  await withNativePage('windowed-widgets', '/ui/src/dashboard/windowedWidgets.dom.fixture.tsx', page => page.stage('windowed widgets', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.windowedWidgets.${expression}`);
    await act('mount()');
    assert.deepEqual(await act('errors()'), {}, 'no widget threw');
    assert.deepEqual(await act('states()'), {}, `each widget drew its data, none a state: ${JSON.stringify(await act('all()'))}`);
    // Aircraft losses' window carries every total before it: its race asks the window shown and the next, never the
    // campaign's windows before it.
    const opened = await act<{ product: string; window: number | null }[]>('opened()');
    const shownWindow = Math.floor(30_000 / (4 * 720));
    assert.deepEqual(opened.filter(item => item.product === 'military-air-ledger').map(item => item.window).sort((a, b) => (a ?? 0) - (b ?? 0)), [shownWindow, shownWindow + 1],
      'aircraft losses open with the window shown and the next ahead');
    const played = await act<{ from: number; head: number; rendered: Record<string, number> }>('play(600, 15)');
    type Asked = { product: string; window: number | null; head: number };
    const asked = await act<Asked[]>('played()');
    const window = 4 * 720, crossed = Math.floor(played.head / window) - Math.floor(played.from / window);
    t.diagnostic(`hour ${played.from} to ${played.head}, ${crossed} windows crossed: renders ${JSON.stringify(played.rendered)}; asked ${asked.map(item => `${item.product}#${item.window}@${item.head}`).join(', ') || 'nothing'}`);
    assert.ok(crossed >= 3, `the head crossed windows: ${crossed}`);
    const products = { produced: 'equipment-produced', supremacy: 'naval-supremacy', spending: 'production-spending', trade: 'trade-flows', air: 'military-air-ledger' };
    const known = new Set([...Object.values(products), 'follow', 'unfollow', 'follow-clock', 'country-styles', 'country-facts']);
    assert.deepEqual(asked.filter(item => !known.has(item.product)).map(item => item.product), [], 'nothing else is asked while playing');
    for (const [id, product] of Object.entries(products)) {
      const windows = asked.filter(item => item.product === product);
      assert.deepEqual(windows.filter(item => item.window === null || item.window * window <= item.head), [], `${id}: every window asked is ahead of the head`);
      assert.equal(new Set(windows.map(item => item.window)).size, windows.length, `${id}: no window asked twice`);
      assert.ok(windows.length <= crossed + 1, `${id}: a window ahead once per window crossed: ${windows.length} for ${crossed}`);
      // Naval supremacy's view and the trade diagram move by whole months while playing, as main's do: beside a window
      // landing, a commit a month at most (two a second at 1,440 h/s).
      const monthly = id === 'supremacy' || id === 'trade';
      const allowed = crossed + 2 + (monthly ? Math.ceil((played.head - played.from) / 720) + 1 : 0);
      assert.ok((played.rendered[id] ?? 0) <= allowed, `${id}: a commit as a window lands${monthly ? ' or a month passes' : ''}, never per hour: ${played.rendered[id]} for ${crossed} windows`);
    }
    assert.ok(asked.some(item => item.product === products.produced) && asked.some(item => item.product === products.air), 'the races asked the windows they crossed');
    await act('pause()');
  }, 120_000));
});

// Run 13 (Overview at 1,440 h/s): the event feed asked its pages of 64 two blocks of rows ahead, a quarter second of a
// feed that dense; a page took a second, landed after the head had passed it, and was asked again at the head (95), and
// the pages of one block all asked from the cut's hour (ten "asked twice").
test('a dense paged log following at 1,440 h/s asks every page ahead of the head, each once, from its own hour', async t => {
  await withNativePage('pages-ahead', '/ui/src/charts/log/logPagesAhead.dom.fixture.tsx', page => page.stage('pages ahead', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.pagesAhead.${expression}`);
    await act('mount()');
    assert.ok(await act<number>('rows()') > 0, 'the log shows its rows at the head');
    await act('play(600, 15)');
    type Asked = { cursor: number; from: number | null; head: number; cancelled: boolean };
    const played = await act<Asked[]>('played()');
    t.diagnostic(`to hour ${await act<number>('head()')}: ${played.length} pages asked, ${played.filter(asked => asked.cancelled).length} cancelled`);
    assert.ok(played.length >= 10, `the pages the head crossed were asked while playing: ${played.length}`);
    const atHead = played.filter(asked => asked.from === null || asked.from <= asked.head);
    assert.deepEqual(atHead, [], `no page asked at the head while playing: ${JSON.stringify(await act('all()'))}`);
    assert.equal(new Set(played.map(asked => asked.cursor)).size, played.length, 'no page asked twice');
    assert.equal(new Set(played.map(asked => asked.from)).size, played.length, 'each page asks from an hour of its own');
    assert.equal(await act<number>('loading()'), 0, 'no page of the rows shown is still loading at the end');
    await act('pause()');
  }, 120_000));
});

// A page that fails (its block does not decode, though the counts are read) was asked again at every render, and with
// no page of the head's bin held the list read as one with no rows.
test('a paged log whose pages fail asks each once until retried, and says why it shows no rows', async () => {
  await withNativePage('pages-failed', '/ui/src/charts/log/logPagesAhead.dom.fixture.tsx', page => page.stage('pages failed', async () => {
    const failed = await page.evaluate<{ asked: number[]; error: string | null; retried: number[] }>('window.pagesAhead.failed(200)');
    assert.ok(failed.asked.length > 0, 'the pages at the head are asked');
    assert.equal(new Set(failed.asked).size, failed.asked.length, `a failed page is not asked again: ${failed.asked}`);
    assert.equal(failed.error, 'The page block does not decode', 'the failure is said, never a list without rows');
    assert.ok(failed.retried.length > 0 && new Set(failed.retried).size === failed.retried.length, `retried, each is asked once again: ${failed.retried}`);
    // Every row of the campaign shown, no cut waits on the failed pages: still said, with a retry that asks them again.
    const campaign = await page.evaluate<{ asked: number[]; error: string | null; retried: number[] }>('window.pagesAhead.failed(200, true)');
    assert.ok(campaign.asked.length > 0 && new Set(campaign.asked).size === campaign.asked.length, `the pages in view are asked once: ${campaign.asked}`);
    assert.equal(campaign.error, 'The page block does not decode', 'a failed page of the whole campaign is said');
    assert.ok(campaign.asked.every(index => campaign.retried.includes(index)) && new Set(campaign.retried).size === campaign.retried.length,
      `retried, each failed page is asked once again: ${campaign.asked} then ${campaign.retried}`);
  }, 120_000));
});

// Run 17's Politics: Event choices, below the fold, asked eleven pages of 120 KB as play began (each parsed for up to
// 30 ms on the main thread): its counts were held by the prepare lane, and nothing but the counts' request checked that
// the widget was off screen.
test('a paged log off screen asks no page while playing, though its counts are held; shown, it asks the rows it draws', async () => {
  await withNativePage('pages-inactive', '/ui/src/charts/log/logPagesAhead.dom.fixture.tsx', page => page.stage('pages inactive', async () => {
    const asked = await page.evaluate<{ offScreen: number; shown: number }>('window.pagesAhead.inactive(300, 15)');
    assert.equal(asked.offScreen, 0, `off screen, no page is asked: ${asked.offScreen}`);
    assert.ok(asked.shown > 0, `shown, the log asks the pages of its rows: ${asked.shown}`);
  }, 120_000));
});

// Run 18's candidate: after the playback phase, every paused step waited out its timeout on "commits 1". A paged log left
// while playing (a page switch; the prepare lane's copy dropped as play begins) asked its pages again as its list let them
// go, after its own cleanup had cancelled what it asked, and they landed on a hook gone.
test('a paged log left while playing asks nothing more and leaves nothing in flight; paused, a setting changed settles within two frames of its commit', async t => {
  await withNativePage('pages-leaving', '/ui/src/charts/log/logPagesAhead.dom.fixture.tsx', page => page.stage('pages leaving', async () => {
    type Kinds = Record<string, number>;
    const left = await page.evaluate<{ askedAfter: number; left: Kinds; shown: number; changed: { commits: number; frames: number; busy: Kinds } }>('window.pagesAhead.leaving(15)');
    t.diagnostic(JSON.stringify(left));
    assert.equal(left.askedAfter, 0, `a log gone asks no page: ${left.askedAfter}`);
    assert.deepEqual(Object.entries(left.left).filter(([, count]) => count), [], 'paused after it left, nothing is in flight');
    assert.equal(left.shown, 0, 'a log shown again, paused, settles');
    assert.ok(left.changed.commits >= 1, `the setting committed: ${left.changed.commits}`);
    assert.ok(left.changed.frames <= 2, `nothing in flight two frames after the setting's commit: ${left.changed.frames} frames, ${JSON.stringify(left.changed.busy)}`);
  }, 120_000));
});

// Followed panels are pushed only when their data changes: one held unchanged chose its rows at its last push's hour, so
// crossing a hole in the chosen group's history (or a join) kept the empty result it had there.
test('a held followed panel is chosen again where the chosen group changes: known empty, then unavailable, then its members, with no new push', async () => {
  await withNativePage('choice-hour', '/ui/src/dashboard/choiceHour.dom.fixture.tsx', page => page.stage('choice hour', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.choiceHour.${expression}`);
    type Shown = { exile: string; balance: string; state: string; agreement: string; at: number };
    const all = (value: string, at: number): Shown => ({ exile: value, balance: value, state: value, agreement: value, at });
    await act('mount()');
    assert.deepEqual(await act<Shown>('shown()'), all('none', 100), 'no chosen country at the push: known empty');
    await act('play(105)');
    const renders = await act<number>('renders()');
    await act('play(108)');
    assert.equal(await act<number>('renders()'), renders, 'no render between the group’s changes');
    await act('play(115)');
    assert.deepEqual(await act<Shown>('shown()'), all('Group membership is unavailable at this time.', 110), 'its history not recorded: never none');
    await act('play(130)');
    assert.deepEqual(await act<Shown>('shown()'), { exile: '7', balance: '0', state: '1', agreement: '1', at: 125 }, 'the members who joined at 125');
    // Settled and paused with the panel pushed at 130, a seek back to 115 lands the same panel at 115 with no change.
    await act('land(130)');
    await act('rerender()');
    assert.deepEqual(await act<Shown>('shown()'), { exile: '7', balance: '0', state: '1', agreement: '1', at: 130 });
    const settled = await act<number>('renders()');
    await act('land(115)');
    assert.deepEqual(await act<Shown>('shown()'), all('Group membership is unavailable at this time.', 115), 'the same panel landed back in the gap: never none');
    assert.equal(await act<number>('renders()') - settled, 1, 'one render, for the hour the rows are chosen at');
  }, 60_000));
});
