import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';
import type { fixture } from './VirtualList.dom.fixture';
import type { TabCase, TabControls, TabMutation, TabShape, TabViewport } from './VirtualList.tab.fixture';

type Snapshot = ReturnType<typeof fixture.snapshot>;

test('mounted virtual histories preserve scrolling, expansion and native column traversal', async context => {
  await withNativePage('virtual-list-dom', '/ui/src/components/VirtualList.dom.fixture.tsx', async native => {
    const { evaluate, page } = native;
    const act = (expression: string) => evaluate(`window.virtualListFixture.${expression}`);
    const snapshot = () => act('snapshot()') as Promise<Snapshot>;
    const press = async (key: 'Tab' | 'Home' | 'End' | 'ArrowUp' | 'ArrowDown', shift = false) => {
      await native.press(key, shift);
      await act('settle()');
    };
    if (!(process.env.BATTLEPLAN_VIRTUAL_TAB_ONLY ?? process.env.HOI4_VIRTUAL_TAB_ONLY)) await native.stage('VirtualList DOM assertions', async () => {
      await context.test('a parent and child mounted together bind scroll, row measurements and nested expansion', async () => {
        await act('mount()');
        const initial = await snapshot();
        assert.ok(initial.columns[0].rows.length < 40, 'the full history stays virtual');
        await act('scroll(3000)');
        const scrolled = await snapshot();
        assert.ok(scrolled.columns[0].rows.some(row => row.index === 54), 'scrolling alone mounts the reached viewport');
        await act('scroll(0)'); await act('expand(0, 0)');
        const expanded = await snapshot(), first = expanded.columns[0].rows.find(row => row.index === 0)!;
        assert.deepEqual(first.opened, [true, true]);
        assert.equal(expanded.columns[0].rows.find(row => row.index === 1)!.top, first.height, 'row measurements update prefix offsets after expansion');
        assert.equal(expanded.columns[0].totalHeight, 99 * 56 + first.height, 'the measured height updates the whole index');
        await act('focus("0:0:row")'); await press('End');
        const ended = await snapshot(); assert.equal(ended.active, '0:99:row');
        assert.ok(!ended.columns[0].rows.some(row => row.index === 0), 'the expanded first row really leaves the DOM');
        await press('Home');
        const restored = await snapshot(); assert.equal(restored.active, '0:0:row');
        assert.deepEqual(restored.columns[0].rows.find(row => row.index === 0)!.opened, [true, true], 'End/Home restore conditional nested details');
      });
      await context.test('a render that keeps the row keys keeps every row observed: none that stays is released or observed again', async () => {
        type Seen = { observed: number; unobserved: number; keptObserved: number; keptUnobserved: number };
        for (const strict of [true, false]) {
          await act(`mount(1, false, 'nested', [], ${strict})`); await act('observations(true)');
          await act('rerender()');
          const kept = await act('observations(true)') as Seen;
          assert.deepEqual([kept.observed, kept.keptUnobserved], [0, 0],
            `a render with the same keys${strict ? ' under StrictMode' : ''} mounts no row and releases none that stays: ${JSON.stringify(kept)}`);
        }
        await act('scroll(3000)');
        const scrolled = await act('observations()') as Seen;
        assert.ok(scrolled.keptObserved > 0 && scrolled.unobserved > scrolled.keptUnobserved,
          `rows that enter are observed and rows that leave released: ${JSON.stringify(scrolled)}`);
      });
      await context.test('authored CSS row gaps survive expansion, offscreen prefixes and style changes without a trailing gap', async () => {
        await act('mount()'); await act('rowGap(1)');
        const initial = (await snapshot()).columns[0];
        assert.equal(initial.rows.find(row => row.index === 1)!.top, 57, 'the next collapsed row follows its height and authored gap');
        assert.equal(initial.totalHeight, 100 * 56 + 99, 'only gaps between rows enter the measured total');
        await act('expand(0, 0)');
        const expanded = (await snapshot()).columns[0], first = expanded.rows.find(row => row.index === 0)!;
        assert.equal(expanded.rows.find(row => row.index === 1)!.top, first.height + 1, 'an expanded body retains the following gap');
        assert.equal(expanded.totalHeight, 99 * 56 + first.height + 99);
        await act('focus("0:0:row")'); await press('End');
        const end = (await snapshot()).columns[0], last = end.rows.find(row => row.index === 99)!;
        assert.equal(last.top, first.height + 98 * 56 + 99, 'the complete offscreen prefix retains every authored gap');
        assert.equal(end.totalHeight, last.top + last.height, 'the final row has no trailing gap');
        await press('Home'); await act('rowGap(2.5)');
        const changed = (await snapshot()).columns[0];
        assert.equal(changed.rows.find(row => row.index === 1)!.top, first.height + 2.5);
        assert.equal(changed.totalHeight, 99 * 56 + first.height + 99 * 2.5, 'changed CSS gaps preserve retained expansion measurements');
      });
      await context.test('fractional row and scroller boxes retain layout pixels under native content zoom', async () => {
        const sameLayout = (actual: number, expected: number, message: string) =>
          assert.ok(Math.abs(actual - expected) < 1 / 128, `${message}: ${actual} versus ${expected}`);
        await act('mount()'); await act('rowGap(1)');
        const measured = await act('fractionalRow(56.984375, 1.25)') as number;
        assert.notEqual(measured, Math.round(measured), 'the authored row has a fractional layout height');
        const initial = (await snapshot()).columns[0];
        sameLayout(initial.rows.find(row => row.index === 1)!.top, measured + 1, 'the following prefix preserves the unrounded native border box');
        sameLayout(initial.totalHeight, 99 * 56 + measured + 99, 'the total preserves the fractional row');
        await act('focus("0:0:row")'); await press('End');
        const end = await snapshot(), last = end.columns[0].rows.find(row => row.index === 99)!;
        assert.equal(end.active, '0:99:row');
        sameLayout(last.top, measured + 98 * 56 + 99, 'zoom keeps offscreen prefixes in layout pixels');
        sameLayout(end.columns[0].totalHeight, last.top + last.height, 'the total ends at the final row');
        await press('Home');
        sameLayout((await snapshot()).columns[0].rows.find(row => row.index === 1)!.top, measured + 1, 'remounting preserves the fractional measurement');
      });
      await context.test('native Tab and Shift+Tab reach complete adjacent columns, nested controls and external exits', async () => {
        await act('mount(2, true)'); await act('focus("0:0:row")'); await press('End');
        assert.equal((await snapshot()).active, '0:99:row');
        await press('Tab'); assert.equal((await snapshot()).active, '1:0:row', 'Tab enters the next column at its logical first completion');
        await press('Tab', true); assert.equal((await snapshot()).active, '0:99:row', 'Shift+Tab enters the previous column at its logical last completion');
        await act('expand(0, 99)'); await act('focus("0:99:row")');
        for (const target of ['0:99:details', '0:99:input', '0:99:button', '1:0:row']) {
          await press('Tab'); assert.equal((await snapshot()).active, target, 'nested controls retain native Tab order');
        }
        await press('Tab', true); assert.equal((await snapshot()).active, '0:99:button', 'backward entry reaches the last nested control');
        await act('focus("1:99:row")'); await press('End'); await press('Tab');
        assert.equal((await snapshot()).active, 'after', 'the final column exits to the external control');
        await act('scroll(0)'); await press('Tab', true);
        assert.equal((await snapshot()).active, '1:99:row', 'backward external entry reaches the complete last column at a top viewport');
        await act('scroll(0)'); await act('focus("0:0:row")'); await press('Tab', true);
        assert.equal((await snapshot()).active, 'before', 'the first column exits backward to the external control');
        await act('scroll(3000)'); await press('Tab');
        assert.equal((await snapshot()).active, '0:0:row', 'forward external entry reaches the complete first column at a middle viewport');
      });
      await context.test('a noninteractive final ordered row permits forward exit and backward entry', async () => {
        await act('mount(1, false, "description", [99])');
        await act('scroll(5500)'); await act('focus("0:98:row")'); await press('Tab');
        assert.equal((await snapshot()).active, 'after', 'Tab exits after the final completed summary');
        const last = (await snapshot()).columns[0].rows.find(row => row.index === 99)!;
        assert.equal(last.position, 100); assert.equal(last.size, 100, 'unfinished rows retain full ordered-list membership');
        await act('scroll(0)'); await press('Tab', true);
        assert.equal((await snapshot()).active, '0:98:row', 'backward external entry skips the offscreen unfinished row');
        await act('expand(0, 98)'); await act('focus("after")'); await act('scroll(0)'); await press('Tab', true);
        assert.equal((await snapshot()).active, '0:98:details', 'backward entry restores the final interactive row\'s nested control');
        await press('Home'); await press('End');
        assert.equal((await snapshot()).active, '0:98:row', 'End targets the final interactive row');
        await press('ArrowDown'); assert.equal((await snapshot()).active, '0:98:row');
      });
      await context.test('noninteractive first and middle rows preserve entry, exit and keyboard destinations', async () => {
        await act('mount(1, false, "description", [0, 2])'); await act('focus("before")'); await act('scroll(3000)'); await press('Tab');
        assert.equal((await snapshot()).active, '0:1:row', 'forward external entry skips the offscreen noninteractive first row');
        const first = (await snapshot()).columns[0].rows.find(row => row.index === 0)!;
        assert.equal(first.position, 1); assert.equal(first.size, 100);
        await press('Tab', true); assert.equal((await snapshot()).active, 'before', 'the first interactive row exits backward');
        await press('Tab'); await press('Tab'); assert.equal((await snapshot()).active, '0:3:row', 'Tab skips noninteractive middle rows');
        await press('Tab', true); assert.equal((await snapshot()).active, '0:1:row');
        await press('ArrowDown'); assert.equal((await snapshot()).active, '0:3:row');
        await press('ArrowUp'); assert.equal((await snapshot()).active, '0:1:row');
        await press('End'); await press('Home'); assert.equal((await snapshot()).active, '0:1:row', 'Home targets the first interactive row');
      });
      for (const count of [100, 5000]) for (const backwards of [false, true]) {
        await context.test(`${count} sparse rows traverse offscreen static runs ${backwards ? 'backward' : 'forward'} and eventually exit`, async () => {
          const eligible = [0, Math.floor(count / 2), count - 1];
          await act(`mountSparse(${count}, ${JSON.stringify(eligible)})`);
          if (backwards) await act(`scroll(${(count - 1) * 56})`);
          await act(`focus("0:${backwards ? count - 1 : 0}:row")`);
          const order = backwards ? [...eligible].reverse() : eligible;
          for (const next of [...order.slice(1).map(id => `0:${id}:row`), backwards ? 'before' : 'after']) {
            await press('Tab', backwards);
            const state = await snapshot();
            assert.equal(state.active, next, 'each native key reaches the next logical control and exits at the boundary');
            assert.ok(state.columns[0].rows.length < 20, 'long static runs never mount a full history');
            for (const row of state.columns[0].rows) {
              assert.equal(row.position, row.index + 1); assert.equal(row.size, count);
            }
          }
          await press('Tab', !backwards);
          assert.equal((await snapshot()).active, `0:${order.at(-1)}:row`, 'native reentry returns to the exit boundary');
          await press('Tab', backwards);
          assert.equal((await snapshot()).active, backwards ? 'before' : 'after', 'repeated entry and exit never trap focus');
        });
      }
      for (const backwards of [false, true]) {
        await context.test(`native ${backwards ? 'backward' : 'forward'} entry reaches the eligible row through a wholly static viewport`, async () => {
          const id = backwards ? 0 : 4999;
          await act(`mountSparse(5000, [${id}])`);
          await act(`focus("${backwards ? 'after' : 'before'}")`);
          await act(`scroll(${backwards ? 5000 * 56 : 0})`);
          assert.equal(await evaluate(`(() => {
            const bounds = document.querySelector('[data-scroll]').getBoundingClientRect();
            return [...document.querySelectorAll('summary')].filter(node => {
              const row = node.getBoundingClientRect(); return row.bottom > bounds.top && row.top < bounds.bottom;
            }).length;
          })()`), 0, 'the actual viewport contains only static content before native entry');
          await press('Tab', backwards);
          const state = await snapshot();
          assert.equal(state.active, `0:${id}:row`);
          assert.ok(state.columns[0].rows.length < 20);
          assert.equal(await evaluate('document.querySelectorAll("ol [tabindex]").length'), 0, 'entry uses existing row controls without additional tab stops');
          await press('Tab', backwards);
          assert.equal((await snapshot()).active, backwards ? 'before' : 'after', 'the sole interactive row permits native exit');
        });
      }
      await context.test('sparse offscreen handoffs restore expanded description controls in native order', async () => {
        await act('mountSparse(5000, [0, 4999], "description")'); await act('expand(0, 0)');
        await act('focus("0:0:row")'); await press('End'); await act('expand(0, 4999)'); await act('focus("0:4999:details")');
        await press('Tab'); assert.equal((await snapshot()).active, 'after');
        for (const expected of ['0:4999:details', '0:4999:row', '0:0:details', '0:0:row', 'before']) {
          await press('Tab', true); assert.equal((await snapshot()).active, expected, 'backward traversal reaches restored final descriptions then exits');
        }
        for (const expected of ['0:0:row', '0:0:details', '0:4999:row', '0:4999:details', 'after']) {
          await press('Tab'); assert.equal((await snapshot()).active, expected, 'forward traversal retains completion and description order then exits');
        }
        assert.ok((await snapshot()).columns[0].rows.length < 20);
      });
      await context.test('a wholly static list keeps native scroller entry and exit in both directions', async () => {
        await act('mountSparse(5000, [])'); await act('focus("before")');
        await press('Tab');
        assert.equal(await evaluate('document.activeElement === document.querySelector("[data-scroll]")'), true);
        await press('Tab'); assert.equal((await snapshot()).active, 'after');
        await press('Tab', true);
        assert.equal(await evaluate('document.activeElement === document.querySelector("[data-scroll]")'), true);
        await press('Tab', true); assert.equal((await snapshot()).active, 'before');
        assert.ok((await snapshot()).columns[0].rows.length < 20);
        assert.equal(await evaluate('document.querySelectorAll("ol summary, ol button, ol [tabindex]").length'), 0);
      });
      await context.test('nested native Tab does not scan 5000 rows before reaching a row edge', async () => {
        await act('mountSparse(5000, [0, 4999])'); await act('expandOuter(0, 0)'); await act('focus("0:0:row")');
        await act('predicateCalls(true)');
        const calls: number[] = [];
        for (const [backwards, expected] of [[false, '0:0:input'], [false, '0:0:button'], [true, '0:0:input'], [true, '0:0:row']] as const) {
          await press('Tab', backwards); assert.equal((await snapshot()).active, expected);
          calls.push(await act('predicateCalls(true)'));
        }
        assert.deepEqual(calls, [0, 0, 0, 0], 'native traversal never reevaluates row eligibility');
      });
      await context.test('5000-row sparse edge lookup stays bounded and refreshes after eligibility and content changes', async () => {
        await act('mountSparse(5000, [0, 4999])'); await act('focus("0:0:row")'); await act('predicateCalls(true)');
        await press('Tab');
        assert.equal(await act('predicateCalls(true)'), 0, 'one Tab reuses indexed eligibility across the complete static run');
        assert.equal((await snapshot()).active, '0:4999:row');
        await act('eligibility([0, 2500])'); await act('focus("before")'); await press('Tab');
        assert.equal((await snapshot()).active, '0:0:row');
        await act('predicateCalls(true)'); await press('End');
        assert.equal((await snapshot()).active, '0:2500:row', 'changed eligibility replaces the old final destination');
        assert.equal(await act('predicateCalls(true)'), 0);
        await press('Tab'); assert.equal((await snapshot()).active, 'after');
        await act('reverseSparseRows()'); await act('focus("before")'); await press('Tab');
        assert.equal((await snapshot()).active, '0:2500:row', 'changed content order rebuilds eligible positions');
        const first = (await snapshot()).columns[0].rows.find(row => row.index === 2499)!;
        assert.equal(first.position, 2500); assert.equal(first.size, 5000);
        await act('predicateCalls(true)'); await press('End');
        assert.equal((await snapshot()).active, '0:0:row');
        assert.equal(await act('predicateCalls(true)'), 0);
      });
      await context.test('backward column entry waits for conditionally mounted controls without nested details', async () => {
        await act('mount(2, true, "plain")'); await act('focus("0:0:row")'); await press('End');
        await act('expandOuter(0, 99)'); await act('focus("0:99:button")'); await press('Tab');
        assert.equal((await snapshot()).active, '1:0:row');
        await press('Tab', true);
        assert.equal((await snapshot()).active, '0:99:button', 'backward entry waits for the retained outer expansion to build its controls');
      });
      await context.test('backward traversal restores an expanded middle row while later expanded neighbours remount', async () => {
        await act('mount(1, false, "description")'); await act('scroll(1000)');
        await act('expand(0, 19)'); await act('expand(0, 21)');
        assert.equal(await evaluate('document.querySelector("[data-scroll]").querySelectorAll("input, button").length'), 0, 'research rows expose only completion and Description summaries');
        await act('focus("0:20:row")'); await act('scroll(4000)');
        const before = await snapshot();
        assert.equal(before.active, '0:20:row');
        for (const index of [19, 21]) assert.ok(!before.columns[0].rows.some(row => row.index === index), `expanded row ${index} leaves the DOM`);
        await press('Tab', true);
        const restored = await snapshot();
        for (const index of [19, 21]) assert.deepEqual(restored.columns[0].rows.find(row => row.index === index)!.opened, [true, true], `row ${index} retains both expansions`);
        assert.equal(restored.active, '0:19:details', 'Shift+Tab reaches the preceding row\'s final Description summary');
      });
      await context.test('an existing list rebinds a changed scroller element and cleans up both owners', async () => {
        await act('mountRebinding()');
        assert.deepEqual(await act('bindingListeners()'), [1, 0]);
        await act('scroll(3000)'); assert.ok((await snapshot()).columns[0].rows.some(row => row.index === 54));
        await act('rebind()');
        assert.deepEqual(await act('bindingListeners()'), [0, 1], 'the old scroller releases its listener while the new one binds');
        assert.ok((await snapshot()).columns[0].rows.some(row => row.index === 0), 'the new scroller sets its own viewport');
        await act('scroll(3000)'); const rebound = await snapshot();
        assert.ok(rebound.columns[0].rows.some(row => row.index === 54));
        await act('oldScroll(1000)');
        assert.deepEqual((await snapshot()).columns[0].rows, rebound.columns[0].rows, 'the abandoned scroller cannot change the current viewport');
        assert.deepEqual(await act('unmountRebinding()'), [0, 0], 'unmount releases every scroller listener');
      });
    }, 60_000);
    const socket = new WebSocket(page);
    const pending = new Map<number, { resolve(value: any): void; reject(error: Error): void; timer: ReturnType<typeof setTimeout> }>();
    let sequence = 0;
    socket.addEventListener('message', event => {
      const reply = JSON.parse(String(event.data)), request = pending.get(reply.id);
      if (!request) return;
      pending.delete(reply.id); clearTimeout(request.timer);
      if (reply.error) request.reject(new Error(JSON.stringify(reply.error))); else request.resolve(reply.result);
    });
    const disconnected = () => {
      for (const request of pending.values()) { clearTimeout(request.timer); request.reject(new Error('Matrix CDP disconnected')); }
      pending.clear();
    };
    socket.addEventListener('close', disconnected); socket.addEventListener('error', disconnected);
    await new Promise<void>((done, reject) => {
      const timer = setTimeout(() => { socket.close(); reject(new Error('Matrix CDP connection timed out')); }, 10_000);
      socket.addEventListener('open', () => { clearTimeout(timer); done(); }, { once: true });
      socket.addEventListener('error', () => { clearTimeout(timer); reject(new Error('Matrix CDP connection failed')); }, { once: true });
    });
    const send = (method: string, params: Record<string, unknown>) => new Promise<any>((resolve, reject) => {
      if (socket.readyState !== WebSocket.OPEN) { reject(new Error('Matrix CDP is closed')); return; }
      const id = ++sequence;
      const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Matrix ${method} timed out`)); }, 10_000);
      pending.set(id, { resolve, reject, timer }); socket.send(JSON.stringify({ id, method, params }));
    });
    const tab = async (expression: string) => {
      const result = await send('Runtime.evaluate', { expression: `window.virtualListFixture.tab.${expression}`, awaitPromise: true, returnByValue: true });
      assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails)); return result.result.value;
    };
    const dispatchTab = async (backwards: boolean) => {
      await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9, modifiers: backwards ? 8 : 0 });
      await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Tab', code: 'Tab', modifiers: backwards ? 8 : 0 });
    };
    const nativeTab = async (backwards: boolean) => { await dispatchTab(backwards); await tab('keySettled()'); return tab('active()'); };
    const mountCase = (spec: TabCase) => tab(`mount(${JSON.stringify(spec)})`);
    const fullSweep = (process.env.BATTLEPLAN_VIRTUAL_TAB_FULL ?? process.env.HOI4_VIRTUAL_TAB_FULL) === '1';
    const mutationTrace = async (side: 'reference' | 'virtual', id: string, mutation: TabMutation, backwards: boolean,
      viewport: TabViewport, control = 'summary') => {
      await tab('reset()'); await tab(`activate("${side}", "before")`);
      await tab(`anchor("${side}", "${id}", "${control}")`);
      await tab(`viewport("${side}", "${viewport}")`);
      await tab(`mutate("${mutation}", "${id}", "${control}")`);
      const trace = [await tab('active()')];
      for (let step = 0; step < 4; step++) {
        trace.push(await nativeTab(backwards));
        if (trace.at(-1) === (backwards ? 'before' : 'after')) break;
      }
      return trace;
    };
    const walk = async (side: 'reference' | 'virtual', backwards: boolean, limit: number, anchor?: string, viewport: TabViewport = 'start') => {
      await tab(`activate("${side}", "${backwards ? 'after' : 'before'}")`);
      await tab(`viewport("${side}", "${viewport}")`);
      if (anchor !== undefined) await tab(`anchor("${side}", ${JSON.stringify(anchor)})`);
      const order: string[] = [];
      for (let step = 0; step < limit; step++) {
        const control = await nativeTab(backwards); order.push(control);
        if (control === (backwards ? 'before' : 'after')) break;
      }
      return order;
    };
    try {
      await native.stage('VirtualList native differential assertions', async () => {
        await context.test('native differential static pointer clicks resume from the clicked Description in both directions', async () => {
          const divergences = [];
          for (const backwards of [false, true]) for (const external of [undefined, 'focus', 'pointer'] as const) {
            const traces = [];
            for (const side of ['reference', 'virtual'] as const) {
              await mountCase({ count: 100, shape: 'all', controls: 'summary', viewport: 'middle', description: 'conditional' });
              await tab(`activate("${side}", "before")`); await tab(`anchor("${side}", "50")`);
              await tab(`toggle("${side}", "50:summary")`); await tab(`toggle("${side}", "50:description")`);
              await tab(`programmatic("${side}", "50:description")`);
              const point = await tab(`point("${side}", "50:text")`);
              await send('Input.dispatchMouseEvent', { type: 'mousePressed', ...point, button: 'left', clickCount: 1 });
              await send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...point, button: 'left', clickCount: 1 });
              await tab('settled()');
              assert.equal(await tab('active()'), 'BODY', 'static text establishes a native starting point without focus');
              if (external) {
                const point = await tab(`external("${side}", "${external}", "${backwards ? 'after' : 'before'}")`);
                if (external === 'pointer') {
                  await send('Input.dispatchMouseEvent', { type: 'mousePressed', ...point, button: 'left', clickCount: 1 });
                  await send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...point, button: 'left', clickCount: 1 });
                  await tab('settled()');
                  assert.equal(await tab('active()'), 'BODY');
                }
              }
              const trace = [await tab('active()')];
              for (let step = 0; step < 3; step++) {
                await dispatchTab(backwards); await tab('settled()'); trace.push(await tab('active()'));
              }
              traces.push(trace);
            }
            if (!external) assert.deepEqual(traces[0], backwards ? ['BODY', '50:description', '50:summary', '49:summary']
              : ['BODY', '51:summary', '52:summary', '53:summary'], 'reference follows the clicked text');
            if (JSON.stringify(traces[0]) !== JSON.stringify(traces[1])) divergences.push({ backwards, external, reference: traces[0], virtual: traces[1] });
          }
          assert.deepEqual(divergences, [], 'static clicks retain native sequential navigation');
        });
        await context.test('native differential conditional Description destruction resets expansion while mounted descendants retain it', async () => {
          const divergences = [];
          for (const description of ['conditional', 'unconditional'] as const) {
            const states = [];
            for (const side of ['reference', 'virtual'] as const) {
              await mountCase({ count: 100, shape: 'all', controls: 'summary', viewport: 'middle', description });
              await tab(`activate("${side}", "before")`); await tab(`anchor("${side}", "50")`);
              await tab(`toggle("${side}", "50:summary")`); await tab(`toggle("${side}", "50:description")`);
              assert.deepEqual(await tab(`details("${side}", "50")`), [true, true]);
              await tab(`activate("${side}", "before")`); await tab(`scrollAway("${side}")`);
              assert.equal(await tab(`mounted("${side}", "50")`), side === 'reference', 'whole-row offscreen restoration has a real unmount');
              await tab(`anchor("${side}", "50")`); await tab('settled()');
              assert.deepEqual(await tab(`details("${side}", "50")`), [true, true], 'whole-row remount preserves both expansions');
              await tab(`toggle("${side}", "50:summary")`);
              const closed = await tab(`details("${side}", "50")`);
              await tab(`toggle("${side}", "50:summary")`);
              states.push([closed, await tab(`details("${side}", "50")`)]);
            }
            assert.deepEqual(states[0], description === 'conditional' ? [[false], [true, false]] : [[false, true], [true, true]]);
            if (JSON.stringify(states[0]) !== JSON.stringify(states[1])) divergences.push({ description, reference: states[0], virtual: states[1] });
          }
          assert.deepEqual(divergences, [], 'recreated descriptions start closed; surviving descriptions keep native state');
        });
        await context.test('native differential explicitly preserves a surviving focused identity across reverse and preceding removal', async () => {
          const divergences = [];
          for (const [count, id, mutation] of [[100, '0', 'reverse'], [5000, '2500', 'remove-before']] as const) {
            const traces = [];
            for (const side of ['reference', 'virtual'] as const) {
              await mountCase({ count, shape: 'runs', controls: 'nested', viewport: 'start' });
              await tab(`activate("${side}", "before")`); await tab(`anchor("${side}", "${id}")`);
              await tab(`scrollAway("${side}")`); await tab(`mutate("${mutation}", "${id}")`);
              traces.push([await tab('active()'), await nativeTab(false), await nativeTab(true)]);
            }
            if (JSON.stringify(traces[1]) !== JSON.stringify(traces[0])) divergences.push({ count, mutation, reference: traces[0], virtual: traces[1] });
          }
          assert.deepEqual(divergences, [], 'focus and subsequent native keys match keyed fully mounted DOM');
        });
        await context.test('native differential explicitly excludes negative tabindex and disabled hidden inert descendants from sparse handoffs', async () => {
          const divergences = [];
          await mountCase({ count: 5000, shape: 'runs', controls: 'excluded', viewport: 'start' });
          for (const backwards of [false, true]) {
            const reference = await walk('reference', backwards, 20);
            const virtual = await walk('virtual', backwards, 20);
            if (JSON.stringify(virtual) !== JSON.stringify(reference)) divergences.push({ backwards, reference, virtual });
            assert.ok(reference.every(control => !/button|disabled|fieldset|hidden|display|visibility|inert|closed/.test(control)));
          }
          assert.deepEqual(divergences, []);
        });
        await context.test('native differential traverses from programmatically focused negative-tabindex controls', async () => {
          const divergences = [];
          for (const backwards of [false, true]) {
            const traces = [];
            for (const side of ['reference', 'virtual'] as const) {
              await mountCase({ count: 5000, shape: 'runs', controls: 'excluded', viewport: 'start' });
              await tab(`activate("${side}", "before")`); await tab(`anchor("${side}", "${backwards ? 4999 : 0}")`);
              await tab(`programmatic("${side}", "${backwards ? 4999 : 0}:button")`);
              const trace = []; for (let step = 0; step < 4; step++) trace.push(await nativeTab(backwards)); traces.push(trace);
            }
            if (JSON.stringify(traces[0]) !== JSON.stringify(traces[1])) divergences.push({ backwards, reference: traces[0], virtual: traces[1] });
          }
          assert.deepEqual(divergences, []);
        });
        await context.test('native differential pending handoffs follow the destination key through content mutations before their frame', async () => {
          const divergences = [];
          for (const mutation of ['reverse', 'remove-before', 'insert-before'] satisfies TabMutation[])
            for (const backwards of [false, true]) for (const controls of ['summary', 'excluded'] satisfies TabControls[]) {
            const traces = [];
            for (const side of ['reference', 'virtual'] as const) {
              await mountCase({ count: 5000, shape: 'runs', controls, viewport: 'start' });
              await tab(`activate("${side}", "before")`); await tab(`anchor("${side}", "${backwards ? 4999 : 0}")`);
              if (!backwards && controls === 'excluded') await tab(`programmatic("${side}", "0:input")`);
              await tab('gateFrames()');
              try { await dispatchTab(backwards); await tab(`mutateImmediately("${mutation}", "2500")`); }
              finally { await tab('releaseFrames()'); }
              traces.push([await tab('active()'), await nativeTab(false), await nativeTab(true)]);
            }
            if (JSON.stringify(traces[0]) !== JSON.stringify(traces[1])) divergences.push({ mutation, backwards, controls, reference: traces[0], virtual: traces[1] });
          }
          assert.deepEqual(divergences, []);
        });
        await context.test('native differential removed rows and descendants retain BODY and the sequential starting position', async () => {
          const divergences: unknown[] = [];
          for (const controls of ['nested', 'excluded'] satisfies TabControls[]) {
            await mountCase({ count: 5000, shape: 'runs', controls, viewport: 'start' });
            for (const mutation of ['remove-focus', 'remove-control'] satisfies TabMutation[])
              for (const control of mutation === 'remove-control' ? ['input', 'button'] : ['button']) for (const backwards of [false, true]) {
              const reference = await mutationTrace('reference', '2500', mutation, backwards, 'start', control);
              const virtual = await mutationTrace('virtual', '2500', mutation, backwards, 'start', control);
              assert.equal(reference[0], 'BODY', 'the oracle leaves BODY active until the next native key');
              if (JSON.stringify(reference) !== JSON.stringify(virtual)) divergences.push({ controls, mutation, control, backwards, reference, virtual });
            }
          }
          assert.deepEqual(divergences, []);
        });
        await context.test('native differential dense deletion resumes where the row was, at middle and end keys', async () => {
          await mountCase({ count: 5000, shape: 'all', controls: 'nested', viewport: 'start' });
          const divergences: unknown[] = [];
          for (const id of ['2500', '4999']) for (const backwards of [false, true]) {
            const reference = await mutationTrace('reference', id, 'remove-focus', backwards, 'start', 'button');
            const virtual = await mutationTrace('virtual', id, 'remove-focus', backwards, 'start', 'button');
            if (JSON.stringify(reference) !== JSON.stringify(virtual)) divergences.push({ id, backwards, reference, virtual });
          }
          assert.deepEqual(divergences, []);
        });
        await context.test('native differential removed-control ownership clears on external focus and pointer interaction', async () => {
          await mountCase({ count: 5000, shape: 'runs', controls: 'nested', viewport: 'start' });
          const divergences: unknown[] = [];
          for (const interaction of ['focus', 'pointer'] as const) for (const backwards of [false, true]) {
            const traces = [];
            for (const side of ['reference', 'virtual'] as const) {
              await tab('reset()'); await tab(`activate("${side}", "before")`); await tab(`anchor("${side}", "2500", "button")`);
              await tab(`viewport("${side}", "start")`); await tab('mutate("remove-focus", "2500")');
              const point = await tab(`external("${side}", "${interaction}", "${backwards ? 'after' : 'before'}")`);
              if (interaction === 'pointer') {
                await send('Input.dispatchMouseEvent', { type: 'mousePressed', ...point, button: 'left', clickCount: 1 });
                await send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...point, button: 'left', clickCount: 1 });
                assert.equal(await tab('active()'), 'BODY', 'clicking noninteractive external content keeps BODY active');
              }
              traces.push([await tab('active()'), await nativeTab(backwards)]);
            }
            if (JSON.stringify(traces[0]) !== JSON.stringify(traces[1])) divergences.push({ interaction, backwards, reference: traces[0], virtual: traces[1] });
          }
          assert.deepEqual(divergences, []);
        });
        await context.test('native differential removal boundaries follow wrapped controls and further mutations while BODY is active', async () => {
          const divergences: unknown[] = [];
          for (const wrapped of [false, true]) {
            await mountCase({ count: 5000, shape: 'runs', controls: 'nested', viewport: 'start', wrapped });
            for (const mutation of ['remove-control', 'remove-focus'] satisfies TabMutation[]) for (const backwards of [false, true]) {
              const traces = [];
              for (const side of ['reference', 'virtual'] as const) {
                await tab('reset()'); await tab(`activate("${side}", "before")`); await tab(`anchor("${side}", "2500", "button")`);
                await tab(`viewport("${side}", "start")`); await tab(`mutate("${mutation}", "2500", "button")`);
                if (mutation === 'remove-focus') await tab('mutate("insert-before", "2501")');
                const trace = [await tab('active()')]; for (let step = 0; step < 3; step++) trace.push(await nativeTab(backwards));
                traces.push(trace);
              }
              if (JSON.stringify(traces[0]) !== JSON.stringify(traces[1])) divergences.push({ wrapped, mutation, backwards, reference: traces[0], virtual: traces[1] });
            }
          }
          assert.deepEqual(divergences, []);
        });
        await context.test('native differential completes a 64-row dense nested-control traversal in both directions', async () => {
          await mountCase({ count: 64, shape: 'all', controls: 'nested', viewport: 'middle' });
          for (const backwards of [false, true]) {
            const reference = await walk('reference', backwards, 200, undefined, 'middle');
            const virtual = await walk('virtual', backwards, 200, undefined, 'middle');
            assert.equal(reference.length, 193, 'every summary/input/button and the external boundary is reached');
            assert.deepEqual(virtual, reference);
          }
        });
        await context.test('seeded native differential Tab matrix compares complete small/sparse paths and dense edges/middles', async () => {
          const started = performance.now(), cases: TabCase[] = [];
          for (const count of [0, 1, 9, 5000]) for (const shape of ['static', 'all', 'start', 'middle', 'end', 'alternating', 'runs'] satisfies TabShape[])
            for (const controls of ['summary', 'nested', 'excluded'] satisfies TabControls[])
              for (const viewport of ['start', 'middle', 'end'] satisfies TabViewport[]) {
                if (!fullSweep && (count < 5000 ? viewport !== 'start'
                  : !(['static', 'runs', 'start', 'middle', 'end'].includes(shape)
                    && viewport === (shape === 'start' ? 'end' : shape === 'end' ? 'start' : 'middle')))) continue;
                cases.push({ count, shape, controls, viewport });
              }
          const generatedCases = cases.length, seen = new Set<string>();
          const distinct = cases.filter(spec => {
            const shape = spec.count === 0 ? 'static' : spec.count === 1 && spec.shape !== 'static' ? 'all' : spec.shape;
            const identity = JSON.stringify([spec.count, shape, shape === 'static' ? 'summary' : spec.controls, spec.count <= 1 ? 'start' : spec.viewport]);
            if (seen.has(identity)) return false; seen.add(identity); return true;
          });
          let seed = 0x71ab;
          for (let at = distinct.length - 1; at > 0; at--) { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; const next = (seed >>> 0) % (at + 1); [distinct[at], distinct[next]] = [distinct[next], distinct[at]]; }
          const content = (spec: TabCase) => JSON.stringify([spec.count, spec.shape, spec.controls]);
          const grouped = new Map<string, TabCase[]>();
          for (const spec of distinct) {
            const key = content(spec), group = grouped.get(key) ?? [];
            group.push(spec); grouped.set(key, group);
          }
          const divergences: { spec: TabCase; backwards: boolean; anchor?: string; reference: string[]; virtual: string[] }[] = [];
          let paths = 0, keys = 0;
          let previousContent: string | undefined;
          for (const spec of [...grouped.values()].flat()) {
            if (previousContent === content(spec)) await tab('reset()'); else await mountCase(spec);
            previousContent = content(spec);
            const ids = await tab('ids()') as string[], dense = spec.count === 5000 && ids.length > 3;
            const limit = dense ? 7 : ids.length * 3 + 4;
            for (const backwards of [false, true]) for (const anchor of dense ? [undefined, ids[Math.floor(ids.length / 2)]] : [undefined]) {
              const reference = await walk('reference', backwards, limit, anchor, spec.viewport), virtual = await walk('virtual', backwards, limit, anchor, spec.viewport);
              paths++; keys += reference.length + virtual.length;
              if (JSON.stringify(reference) !== JSON.stringify(virtual)) divergences.push({ spec, backwards, anchor, reference, virtual });
              if (spec.count === 5000) assert.ok(await tab('rows()') < 25, 'the native oracle never makes the virtual list mount the logical history');
            }
          }
          context.diagnostic(JSON.stringify({ fullSweep, generatedCases, distinctCases: distinct.length, paths, nativeKeys: keys, wallMs: Math.round(performance.now() - started), divergences }));
          assert.deepEqual(divergences, [], 'all generated native focus sequences agree');
        });
        await context.test('native differential mutations compare immediate focused state and both subsequent Tab directions', async () => {
          const started = performance.now(), divergences: unknown[] = [];
          let cases = 0;
          for (const count of fullSweep ? [9, 5000] : [5000]) for (const controls of ['summary', 'nested', 'excluded'] satisfies TabControls[]) {
            await mountCase({ count, shape: 'runs', controls, viewport: 'middle' });
            for (const mutation of ['reverse', 'remove-before', 'remove-after', 'insert-before', 'toggle-focus', 'toggle-before', 'toggle-after', 'predicate-focus'] satisfies TabMutation[])
              for (const shape of mutation === 'toggle-before' || mutation === 'toggle-after' ? ['runs', 'all'] as const : ['runs'] as const)
                for (const backwards of [false, true]) {
                if (!fullSweep && !['toggle-focus', 'predicate-focus'].includes(mutation)) continue;
                const id = String(Math.floor(count / 2)), traces: string[][] = [];
                if (shape === 'all') await mountCase({ count, shape, controls, viewport: 'middle' });
                for (const side of ['reference', 'virtual'] as const) {
                  traces.push(await mutationTrace(side, id, mutation, backwards, 'start'));
                }
                cases++; if (JSON.stringify(traces[0]) !== JSON.stringify(traces[1])) divergences.push({ count, shape, controls, mutation, backwards, reference: traces[0], virtual: traces[1] });
                if (shape === 'all') await mountCase({ count, shape: 'runs', controls, viewport: 'middle' });
              }
          }
          context.diagnostic(JSON.stringify({ mutationCases: cases, wallMs: Math.round(performance.now() - started), divergences }));
          assert.deepEqual(divergences, [], 'focused mutations preserve native logical navigation');
        });
        if (fullSweep) await context.test('extended native differential mutation sweep covers removal replacement insertion and clearing at every viewport', async () => {
          const started = performance.now(), divergences: unknown[] = [];
          let cases = 0;
          for (const shape of ['all', 'runs', 'static'] satisfies TabShape[]) for (const viewport of ['start', 'middle', 'end'] satisfies TabViewport[]) {
            await mountCase({ count: 5000, shape, controls: 'nested', viewport });
            for (const mutation of ['remove-focus', 'remove-control', 'remove-neighbours', 'insert-before', 'insert-after', 'replace-focus', 'clear'] satisfies TabMutation[])
              for (const control of shape === 'static' ? ['static'] : mutation === 'remove-control' ? ['input', 'button'] : ['summary', 'button'])
                for (const id of ['0', '2500', '4999']) for (const backwards of [false, true]) {
                  const reference = await mutationTrace('reference', id, mutation, backwards, viewport, control);
                  const virtual = await mutationTrace('virtual', id, mutation, backwards, viewport, control);
                  cases++;
                  if (JSON.stringify(reference) !== JSON.stringify(virtual)) divergences.push({ shape, viewport, id, mutation, control, backwards, reference, virtual });
                }
          }
          context.diagnostic(JSON.stringify({ extendedMutationCases: cases, wallMs: Math.round(performance.now() - started), divergences }));
          assert.deepEqual(divergences, [], 'the extended mutation family agrees with native mounted DOM');
        });
      }, fullSweep ? 900_000 : 180_000);
    } finally { socket.close(); }
  });
});
