import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetStateProps } from './WidgetState';

test('technical errors stay generic with Retry and Details, named refusals are neutral with no Retry, and all body and value states match the standard', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { queryFailure, RefusedError, ShownError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts');
    const { default: WidgetState } = await vite.ssrLoadModule('/ui/src/components/WidgetState.tsx') as { default: (props: WidgetStateProps) => unknown };
    const { ValueState, MissingPicture, MissingName, valueStateText } = await vite.ssrLoadModule('/ui/src/components/ValueState.tsx') as typeof import('./ValueState');
    const render = (props: WidgetStateProps) => renderToStaticMarkup(createElement(WidgetState as never, props as never));

    const whole = render({ kind: 'error', failure: queryFailure({ kind: 'failed', message: 'The unit leaders reply is malformed.' }), onRetry: () => {} });
    const visible = whole.replace(/<[^>]*>/g, ' ');
    assert.match(visible, /Couldn’t load\./);
    assert.match(visible, /BattlePlan couldn’t tell why\. Details has the technical reason\./);
    assert.match(visible, /Retry/);
    assert.match(visible, /Details/);
    assert.doesNotMatch(visible, /malformed/, 'internal failure detail is kept inside Details, not on the surface');
    assert.match(whole, /data-detail="The unit leaders reply is malformed\."/);
    assert.match(whole, /is-error/);
    assert.match(whole, /role="alert"/);

    const shown = render({ kind: 'error', what: 'factories', failure: new ShownError('Desktop host disconnected.', 'ipc error 502'), onRetry: () => {} });
    assert.match(shown, /Couldn’t load factories\./);
    assert.match(shown, /Desktop host disconnected\./);
    assert.match(shown, /Retry/);
    assert.match(shown, />Details</);

    const part = render({ kind: 'error', what: 'faction members' }).replace(/<[^>]*>/g, ' ');
    assert.match(part, /Couldn’t load faction members\./);
    assert.doesNotMatch(part, /Retry/, 'no Retry without a way to retry');

    const withheld = render({ kind: 'unavailable', message: '2 leaders can’t be shown.', detail: 'Commander: Template missing\nLeader 2: Owner missing' });
    assert.match(withheld, /2 leaders can’t be shown\./);
    assert.match(withheld, /<small class="widget-state-detail">Commander: Template missing\nLeader 2: Owner missing<\/small>/);
    assert.doesNotMatch(withheld, /is-error|role="alert"/);

    const refused = render({ kind: 'error', failure: queryFailure({ kind: 'refused', text: 'Land owners are not prepared natively yet' }), onRetry: () => {} });
    const refusedText = refused.replace(/<[^>]*>/g, ' ');
    assert.match(refusedText, /Not in this recording\./);
    assert.match(refusedText, /BattlePlan couldn’t tell why\. Details has the technical reason\./);
    assert.match(refused, /data-detail="Land owners are not prepared natively yet"/);
    assert.doesNotMatch(refusedText, /Land owners are not prepared/, 'raw backend refusal stays in Details');
    assert.doesNotMatch(refusedText, /Retry/);
    assert.doesNotMatch(refusedText, /Couldn’t load/);
    assert.doesNotMatch(refused, /is-error|role="alert"/);

    const technicalUnavailable = render({ kind: 'unavailable', technical: 'SupplyHealth is not in this playhead\'s files' });
    assert.match(technicalUnavailable, /Not in this recording\./);
    assert.match(technicalUnavailable, /BattlePlan couldn’t tell why\. Details has the technical reason\./);
    assert.match(technicalUnavailable, />Details</);

    const sevenCauses = [
      [{ kind: 'capture-mode', mode: 'Map' }, /This is a Map recording, which holds the campaign map and battle windows only\./],
      [{ kind: 'older-format' }, /This recording was made before BattlePlan recorded research\./],
      [{ kind: 'format-changed' }, /BattlePlan changed how it records research after this recording was made, so it isn’t shown\./],
      [{ kind: 'mod', modName: 'Kaiserreich', what: 'focus trees' }, /Kaiserreich changes focus trees in a way BattlePlan can’t read yet\./],
      [{ kind: 'dlc', dlcName: 'No Step Back' }, /This campaign was played without No Step Back, which research needs\./],
      [{ kind: 'game-build', version: '1.15.4' }, /Research couldn’t be checked against game version 1\.15\.4, so it was withheld\./],
      [{ kind: 'damage' }, /Part of this recording is damaged, so research is withheld\./],
    ] as const;
    for (const [cause, expected] of sevenCauses) {
      const html = render({ kind: 'error', what: 'research', failure: new RefusedError('raw cause', cause as never), onRetry: () => {} });
      assert.match(html, /Research isn’t in this recording\./);
      assert.match(html, expected);
      assert.doesNotMatch(html, /Retry|is-error|role="alert"/);
    }

    const { WidgetFrameContext } = await vite.ssrLoadModule('/ui/src/dashboard/widgetFrame.tsx') as typeof import('../dashboard/widgetFrame');
    const outsideWithBounds = renderToStaticMarkup(createElement(WidgetFrameContext.Provider, {
      value: {
        title: 'Factories', subject: 'factories', things: 'factories', timeMode: 'pin', pinned: true,
        bounds: { firstHour: 43800000, lastHour: 43800024 }, hasSubject: true, captionSlot: null,
        reportState() {}, announce() {}, clearAnnouncement() {}, setDynamicViewSwitch() {}, skeleton: 'generic', setSkeleton() {},
      },
    }, createElement(WidgetState as never, { kind: 'outside', pinned: true, onChangeTime: () => {} } as never)));
    assert.match(outsideWithBounds, /data-state="outside"/);
    assert.match(outsideWithBounds, /This time is outside the recording\./);
    assert.match(outsideWithBounds, /The recording covers /);
    assert.match(outsideWithBounds, /Change time/);
    assert.doesNotMatch(outsideWithBounds, /is-error|role="alert"/);

    const gap = render({ kind: 'not-recorded', reason: 'Branch 1 block 1005 – 1006 is unreadable: checksum', span: { firstHour: 43800000, lastHour: 43800024 }, pinned: true, onChangeTime: () => {} });
    const gapVisible = gap.replace(/<[^>]*>/g, ' ');
    assert.match(gap, /data-state="not-recorded"/);
    assert.match(gapVisible, /Not recorded at this time\./);
    assert.match(gapVisible, /From /);
    assert.doesNotMatch(gapVisible, /unreadable: checksum/, 'raw gap decoder diagnostic stays inside Details');
    assert.match(gap, /data-detail="Branch 1 block 1005 – 1006 is unreadable: checksum"/);
    assert.match(gapVisible, /Change time/);
    assert.match(gapVisible, /Details/);

    const { RegionContext } = await vite.ssrLoadModule('/ui/src/dashboard/widgetTime.ts') as typeof import('../dashboard/widgetTime');
    const gapOutside = renderToStaticMarkup(createElement(RegionContext.Provider, {
      value: {
        heads: { any: () => true, subscribe: () => () => {} } as never,
        outside: { kind: 'not-recorded', message: 'Not recorded at this time.', detail: 'Branch 1 block 1005 – 1006 is unreadable: checksum', span: { firstHour: 43800000, lastHour: 43800024 }, pinned: true },
        key: 'gap',
      },
    }, createElement(WidgetState as never, { kind: 'outside', onChangeTime: () => {} } as never)));
    const gapOutsideVisible = gapOutside.replace(/<[^>]*>/g, ' ');
    assert.match(gapOutside, /data-state="not-recorded"/);
    assert.match(gapOutsideVisible, /From /);
    assert.doesNotMatch(gapOutsideVisible, /unreadable: checksum/, 'Outside gap hides raw decoder reason behind Details');
    assert.match(gapOutside, /data-detail="Branch 1 block 1005 – 1006 is unreadable: checksum"/);
    assert.match(gapOutsideVisible, /Details/);

    const nothingChosen = render({ kind: 'nothing-chosen', prompt: 'Choose a country to see its factories.', onChoose: () => {} });
    assert.match(nothingChosen, /Choose a country to see its factories\./);
    assert.match(nothingChosen, /Choose country/);

    const noMembers = render({ kind: 'no-members', scope: 'date', onChoose: () => {} });
    assert.match(noMembers, /This group has no members at this date\./);
    assert.match(noMembers, /Choose countries/);

    const partialEmpty = render({ kind: 'empty', message: 'No battles yet.', partial: true });
    assert.match(partialEmpty, /No battles yet\./);
    assert.match(partialEmpty, /Some hours in this period weren’t recorded\./);

    const loadingNow = render({ kind: 'loading', label: 'Loading factories…', skeleton: 'table' });
    assert.match(loadingNow, /data-state="loading"/);
    assert.match(loadingNow, /widget-skeleton-table/);
    assert.doesNotMatch(loadingNow, /spinner/);
    assert.doesNotMatch(loadingNow, /role="status"/, 'loading uses the frame announcer rather than a duplicate live region');
    assert.doesNotMatch(loadingNow, /widget-state-loading-label/, 'label waits until 2 s');

    const loadingLate = render({ kind: 'loading', label: 'Loading factories…', delayMs: 0 });
    assert.match(loadingLate, /widget-state-loading-label">Loading factories…/);

    const sectionError = render({ kind: 'error', what: 'details', scale: 'section', onRetry: () => {} });
    assert.match(sectionError, /is-section/);
    assert.match(sectionError, /is-error/);

    for (const kind of ['empty', 'unavailable'] as const) {
      const message = kind === 'empty' ? 'No generals yet.' : 'Leaders weren’t recorded at this time.';
      const detail = createElement('span', { 'data-detail': 'Synthetic country styles decoder failed' }, 'Couldn’t load country name.');
      const html = render({ kind, message, detail });
      assert.ok(html.includes(`<span class="widget-state-title">${message}</span>`));
      assert.ok(html.includes('<small class="widget-state-detail"><span data-detail="Synthetic country styles decoder failed">Couldn’t load country name.</span></small>'));
      assert.doesNotMatch(html, /is-error|role="alert"/);
    }
    const { NO_MEMBERS } = await vite.ssrLoadModule('/ui/src/dashboard/noMembers.ts') as typeof import('../dashboard/noMembers');
    for (const message of Object.values(NO_MEMBERS)) assert.match(render({ kind: 'unavailable', message }), /data-state="empty"/);
    assert.match(render({ kind: 'unavailable', message: 'Group membership is unavailable at this time.' }), /data-state="unavailable"/);

    assert.equal(valueStateText({ kind: 'known', text: '42', unit: 'factories' }), '42 factories');
    assert.equal(valueStateText({ kind: 'zero', unit: 'divisions' }), '0 divisions');
    assert.equal(valueStateText({ kind: 'missing', reason: 'Gap' }), 'Not recorded: Gap');
    assert.equal(valueStateText({ kind: 'notApplicable', reason: 'No navy' }), 'Not applicable: No navy');
    assert.equal(valueStateText({ kind: 'partial', text: '12', reason: '2 countries missing', atLeast: true }), 'at least 12, partial: 2 countries missing');
    assert.match(renderToStaticMarkup(createElement(ValueState, { kind: 'missing', reason: 'Gap', narrow: true })), /data-value-state="missing"/);
    assert.match(renderToStaticMarkup(createElement(ValueState, { kind: 'missing', reason: 'Gap', narrow: true })), /is-compact/);
    assert.match(renderToStaticMarkup(createElement(ValueState, { kind: 'partial', text: '12', reason: '2 countries missing' })), /data-value-state="partial"/);
    assert.match(renderToStaticMarkup(createElement(MissingPicture, { kind: 'flag', width: 20, height: 14 })), /val-picture-missing/);
    assert.match(renderToStaticMarkup(createElement(MissingName, { reason: 'Not recorded' })), /Name not recorded/);
  } finally { await vite.close(); }
});
