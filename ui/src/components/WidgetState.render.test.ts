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
    assert.match(refusedText, /Land owners are not prepared natively yet/);
    assert.doesNotMatch(refusedText, /Retry/);
    assert.doesNotMatch(refusedText, /Couldn’t load/);
    assert.doesNotMatch(refused, /is-error|role="alert"/);

    const causedRefusal = render({ kind: 'error', what: 'research', failure: new RefusedError('Map mode', { kind: 'capture-mode', mode: 'Map' }), onRetry: () => {} });
    assert.match(causedRefusal, /Research isn’t in this recording\./);
    assert.match(causedRefusal, /This is a Map recording, which holds the campaign map and battle windows only\./);
    assert.doesNotMatch(causedRefusal, /Retry|is-error|role="alert"/);

    const outside = render({ kind: 'outside', pinned: true, onChangeTime: () => {} });
    assert.match(outside, /data-state="outside"/);
    assert.match(outside, /This time is outside the recording\./);
    assert.match(outside, /Change time/);
    assert.doesNotMatch(outside, /is-error|role="alert"/);

    const gap = render({ kind: 'not-recorded', reason: 'Block unreadable', span: { firstHour: 43800000, lastHour: 43800024 }, pinned: true, onChangeTime: () => {} });
    assert.match(gap, /data-state="not-recorded"/);
    assert.match(gap, /Not recorded at this time\./);
    assert.match(gap, /Block unreadable/);
    assert.match(gap, /Change time/);

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
    assert.match(renderToStaticMarkup(createElement(ValueState, { value: { kind: 'missing', reason: 'Gap' } })), /data-value-state="missing"/);
    assert.match(renderToStaticMarkup(createElement(ValueState, { value: { kind: 'partial', text: '12', reason: '2 countries missing' } })), /data-value-state="partial"/);
    assert.match(renderToStaticMarkup(createElement(MissingPicture, { kind: 'flag', width: 20, height: 14 })), /val-picture-missing/);
    assert.match(renderToStaticMarkup(createElement(MissingName, { reason: 'Not recorded' })), /Name not recorded/);
  } finally { await vite.close(); }
});
