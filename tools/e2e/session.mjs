import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { log, sleep } from './cdp.mjs';
import { everFailed } from './coverage.mjs';
import { familyOf, scenarioId, scopeScenario } from './perfBaseline.mjs';
import { DRAWN_GRACE_MS, mapDrew, noVisibleChange, pageShown, settingDrawn, stepOver, stillInFlight } from './verdicts.mjs';

export const BUDGET_MS = 100;
export const TOOLTIP_DELAY_MS = 500;

export const settleCall = (timeout, quiet, only = null) => `__e2e.settle(${timeout}, ${quiet}, ${JSON.stringify(only)}, ${stepOver.toString()})`;

export const takesShot = entry => entry.screenshot === true || (!entry.pass && entry.screenshot !== false);

export class Session {
  constructor(app, out, session) {
    this.app = app;
    this.out = out;
    this.session = session;
    this.results = [];
    this.shots = 0;
    this.epoch = null;
    this.history = null;
    this.tier = 'full';
    this.pin = 'all';
  }

  everFailed(area, item, prefix) { return this.history === null || everFailed(this.history, area, item, prefix, this.pin); }

  async init() {
    this.epoch = await this.app.evaluate('performance.timeOrigin');
    await mkdir(path.join(this.out, 'screenshots'), { recursive: true });
  }

  async record(result) {
    const entry = { at: Date.now(), during: this.app.case, ...result, pass: result.pass && !(result.reasons?.length) };
    this.results.push(entry);
    if (!entry.pass) log(`FAIL ${entry.area} ${entry.item} ${entry.label ?? ''}: ${(entry.reasons ?? []).join('; ')}${entry.ms == null ? '' : ` (${Math.round(entry.ms)} ms)`}`);
    if (takesShot(entry)) entry.screenshot = await this.shot(`${entry.area}-${entry.item}-${entry.label ?? ''}`);
    return entry;
  }

  async shot(name) {
    try {
      const file = path.join('screenshots', `${this.session}-${String(++this.shots).padStart(4, '0')}-${name.replace(/[^\w.-]+/g, '_').slice(0, 90)}.png`);
      await writeFile(path.join(this.out, file), await this.app.screenshot());
      return file;
    } catch (error) { return `screenshot failed: ${error.message}`; }
  }

  fail(area, item, label, reason) { return this.record({ area, item, label, ms: null, pass: false, reasons: [reason] }); }

  async step(area, item, label, action, { done = null, changes = null, drew = null, from = 'first', timeout = 6000, quiet = 200, widget = null, awaits = true, budget = BUDGET_MS, check = true, control = label, unobservable = null, shot = false, extra = {} } = {}) {
    this.app.setCase(`${area}/${item}/${label}`);
    const family = familyOf(area, item, label), playing = item === 'header/play-pause' && label === 'play';
    const context = family || playing ? await this.app.evaluate('__e2e.perfContext()') : null;
    if (playing) this.playOrigin = context;
    let changed = null, sentBefore = null, settingsBefore = null, drawnSame = null;
    const article = key => `document.querySelector(${JSON.stringify(`article.dash-widget[data-widget="${key}"]`)})`;
    const settingsOf = key => `${article(key)}?.dataset.settings ?? null`;
    if (changes?.view) sentBefore = changes.since !== undefined ? changes.since : this.app.lastSent('set_dashboard_maps');
    else if (changes) {
      const before = await this.app.evaluate(`__e2e.signature(${JSON.stringify(changes.key)})`);
      if (before === null) return this.fail(area, item, label, 'its widget is not on the page');
      changed = `(now => now !== null && now !== ${JSON.stringify(before)})(__e2e.signature(${JSON.stringify(changes.key)}))`;
      settingsBefore = await this.app.evaluate(settingsOf(changes.key));
      drawnSame = `(a => !!a && (${settingDrawn.toString()})({ settings: a.dataset.settings ?? null, drawn: a.dataset.drawn ?? null, state: __e2e.stateOf(a).state }, ${JSON.stringify(settingsBefore)}))(${article(changes.key)})`;
    }
    const until = changed === null ? done : done === null ? changed : `(${done}) && ${changed}`;
    const alt = drawnSame === null ? null : done === null ? drawnSame : `(${done}) && ${drawnSame}`;
    await this.app.evaluate(`__e2e.mark(${until === null ? 'null' : JSON.stringify(until)}, ${JSON.stringify(awaits ? null : widget)}, ${alt === null ? 'null' : JSON.stringify(alt)}, ${DRAWN_GRACE_MS})`);
    let acted;
    try { acted = await action(); } catch (error) {
      await this.app.evaluate('__e2e.tracking = false');
      return this.fail(area, item, label, `could not act: ${error.message}`);
    }
    const probe = await this.app.evaluate(settleCall(timeout, quiet, awaits ? widget : '\u0000'), timeout + 30_000);
    const reasons = [];
    const touched = awaits ? probe.widgets.filter(w => w.visible && (widget === null || w.title === widget || w.key === widget)) : [];
    if (awaits && widget !== null && !touched.length) reasons.push('its widget is not in view');
    if (unobservable) reasons.push(unobservable);
    const start = from === 'last' ? probe.last : probe.input;
    let drawnAt = null, drawnEpoch = null;
    if (changes?.view && start !== null) {
      const drawn = await this.mapChange(changes.view, sentBefore, this.epoch + start, timeout);
      if (drawn.reason) reasons.push(drawn.reason);
      else { drawnEpoch = drawn.drawnAt; drawnAt = drawn.drawnAt - this.epoch; }
    }
    if (drew && start !== null) {
      const drawn = await this.mapApplied(drew.view, drew.expected, this.epoch + start, timeout);
      if (drawn.reason) reasons.push(drawn.reason);
      else if (drawn.drawnAt !== undefined) { drawnEpoch = drawn.drawnAt; drawnAt = drawn.drawnAt - this.epoch; }
    }
    let ms = null;
    if (start === null) reasons.push('no trusted input reached the page');
    else {
      if (probe.paint === null) reasons.push('when its input painted is unavailable');
      for (const w of touched) if (w.end !== 'loading' && (w.ready ?? w.first) === null && w.seen) reasons.push(`${w.title}: when its content painted is unavailable`);
      if (until !== null && probe.doneAtSeen && probe.doneAt === null) reasons.push('when it finished painting is unavailable');
      const ends = [probe.paint, probe.doneAt, drawnAt, ...touched.map(w => w.ready ?? w.first)].filter(v => v !== null && v !== undefined);
      ms = ends.length ? Math.max(0, ...ends.map(end => end - start)) : null;
    }
    if (probe.placeholderMs > 0) reasons.push(`showed a loading placeholder for ${Math.round(probe.placeholderMs)} ms (${probe.placeholderAt.join(', ')})`);
    for (const name of probe.placeholderStill ?? []) reasons.push(`${name}: still showing a loading placeholder at settlement`);
    if (until !== null && probe.doneAt === null) {
      if (changed !== null && !await this.holds(changed)) reasons.push(noVisibleChange(settingsBefore, await this.app.evaluate(settingsOf(changes.key))));
      else reasons.push(`never finished: ${until}`);
    }
    for (const w of touched) {
      if (w.end === 'loading') reasons.push(`${w.title}: still loading after ${Math.round(timeout / 1000)} s`);
      for (const error of new Set(w.errors)) reasons.push(`${w.title}: ${error}`);
      if (w.end === 'unavailable') reasons.push(`${w.title}: ${w.notice?.text ?? 'unavailable'}`);
      if (w.end === 'blank') reasons.push(`${w.title}: shows nothing`);
    }
    if (probe.inFlight) reasons.push(stillInFlight(probe.inFlight, timeout));
    if (probe.timedOut && !reasons.length) reasons.push(`not settled within ${timeout} ms`);
    const timing = check && ms !== null && ms > budget ? [`${Math.round(ms)} ms, over ${budget} ms`] : [];
    const screenshot = shot ? true : reasons.every(reason => /ms, over/.test(reason)) ? false : undefined;
    const result = await this.record({ area, item, label, control, check, ms, budget, pass: true, reasons, screenshot, input: start === null ? null : this.epoch + start,
      gestureMs: from === 'last' && probe.input !== null && probe.last !== null ? probe.last - probe.input : null, drawnAt: drawnEpoch,
      widgets: touched.map(({ id, kind, title, first, ready, loadingMax, end, errors, notice }) => ({ id, kind, title,
        first: first === null || probe.input === null ? null : first - probe.input, ready: ready === null || probe.input === null ? null : ready - probe.input,
        loadingMax, end, errors, notice })),
      loafs: probe.loafs, placeholderMs: probe.placeholderMs, placeholderAt: probe.placeholderAt, ...(acted && typeof acted === 'object' ? acted : {}), ...extra });
    if (family) {
      const page = area === 'seeks' ? item : ['pages', 'pages-at'].includes(area) ? item.slice(item.indexOf('/') + 1) : context.page;
      const scenario = scopeScenario(this.recipe, { ...context, page, ...(family === 'pause' && this.playOrigin ? { hour: this.playOrigin.hour } : {}) }, /warm/.test(`${item}/${label}`) || !check ? 'warm' : 'cold');
      result.perf = { family, context, recipe: this.recipe, page, scenario: scenarioId(scenario), start: this.epoch + probe.t0,
        settled: this.epoch + probe.now, complete: context.enabled && probe.busySignal && !probe.timedOut && start !== null };
    }
    result.timing = timing;
    result.probe = probe;
    return result;
  }

  async mapChange(view, before, after, timeout) {
    const end = Date.now() + timeout;
    const setting = entry => entry && JSON.stringify({ ...entry, visible: undefined, clip: undefined });
    const previous = before?.args.views.find(entry => entry.id === view) ?? null;
    let sent = null;
    while (!sent && Date.now() < end) {
      const last = this.app.lastSent('set_dashboard_maps');
      const entry = last && last !== before ? last.args.views.find(candidate => candidate.id === view) : null;
      if (entry && setting(entry) !== setting(previous)) sent = entry;
      else await sleep(16);
    }
    if (!sent) return { reason: 'no visible change after the setting (completion unobservable): the app sent its map no new request' };
    const options = JSON.stringify(sent.options ?? {});
    const drew = `(x => { const sent = ${options}; return Object.entries(sent).every(([key, value]) => JSON.stringify(x.options?.[key] ?? null) === JSON.stringify(value ?? null)); })`;
    while (Date.now() < end) {
      const state = await this.app.evaluate(`__e2e.mapState(${JSON.stringify(view)}, ${after}, x => x.sequence >= ${sent.sequence} && !x.loading && ${drew}(x))`);
      if (state) return { drawnAt: state.drawnAt };
      await sleep(16);
    }
    return { reason: `the map never drew its new options within ${Math.round(timeout / 1000)} s` };
  }

  async mapApplied(view, expected, after, timeout) {
    const end = Date.now() + timeout;
    const states = since => this.app.evaluate(`__e2e.mapStates.filter(x => x.id === ${JSON.stringify(view)} && (x.drawnAt ?? 0) > ${since} && !x.loading && !x.error)`);
    let last = null;
    while (Date.now() < end) {
      for (const state of await states(after)) { last = state; if (mapDrew(state, expected) === null) return { drawnAt: state.drawnAt }; }
      await sleep(50);
    }
    if (last) return { reason: `the map never drew the saved settings within ${Math.round(timeout / 1000)} s: ${mapDrew(last, expected)}` };
    const latest = (await states(0)).at(-1) ?? null;
    if (latest && mapDrew(latest, expected) === null) return {};
    return { reason: latest ? `the map drew no frame after the setting within ${Math.round(timeout / 1000)} s, and its last frame ${mapDrew(latest, expected)}`
      : 'the map reported no frame it drew' };
  }


  async locate(selector, { text = null, index = 0, within = null, partial = false } = {}) {
    return this.app.evaluate(`(() => {
      const root = ${within === null ? 'document' : `document.querySelector(${JSON.stringify(within)})`};
      if (!root) return null;
      const all = [...root.querySelectorAll(${JSON.stringify(selector)})].filter(e => e.getClientRects().length
        && ${text === null ? 'true' : `((e.textContent || '').trim()${partial ? `.includes(${JSON.stringify(text)})` : ` === ${JSON.stringify(text)}`} || e.getAttribute('aria-label') === ${JSON.stringify(text)})`});
      const e = all[${index}];
      if (!e) return null;
      e.scrollIntoView({ block: 'nearest', inline: 'nearest' });
      const r = e.getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 2, w: r.width, h: r.height, left: r.left, top: r.top };
    })()`);
  }

  async need(selector, options = {}) {
    const at = await this.locate(selector, options);
    if (!at) throw new Error(`no ${selector}${options.text ? ` "${options.text}"` : ''}`);
    return at;
  }

  async click(selector, options) { const at = await this.need(selector, options); await this.app.click(at.x, at.y); return at; }
  async hover(x, y) { await this.app.mouse('mouseMoved', x, y); }
  key(name, modifiers) { return this.app.key(name, modifiers); }
  type(text) { return this.app.type(text); }
  async escape() { await this.app.key('Escape'); await this.frames(2); }
  frames(count = 2) { return this.app.evaluate(`new Promise(resolve => { let left = ${count}; const next = () => (--left < 0 ? resolve(true) : requestAnimationFrame(next)); next(); })`); }
  until(expression, timeout = 2000) {
    return this.app.evaluate(`new Promise(resolve => { const end = performance.now() + ${timeout};
      const test = () => { let ok = false; try { ok = !!(${expression}); } catch {} if (ok || performance.now() > end) resolve(ok); else requestAnimationFrame(test); }; test(); })`, timeout + 30_000);
  }
  watch() { return this.app.evaluate('__e2e.watch()'); }
  async unwatch() {
    const shown = await this.app.evaluate('__e2e.unwatch()');
    return [...(shown.ms > 0 ? [`showed a loading placeholder for ${Math.round(shown.ms)} ms (${shown.where.join(', ')})`] : []),
      ...(shown.still ?? []).map(name => `${name}: still showing a loading placeholder at settlement`)];
  }

  holds(expression) { return this.app.evaluate(`!!(${expression})`); }

  async drag(from, to, moves = 24) {
    await this.app.evaluate('__e2e.startFrames()');
    try {
      const start = await this.app.evaluate('performance.now()');
      await this.app.mouse('mouseMoved', from.x, from.y);
      await this.app.mouse('mousePressed', from.x, from.y, { buttons: 1 });
      for (let i = 1; i <= moves; i++) {
        await this.app.mouse('mouseMoved', from.x + (to.x - from.x) * i / moves, from.y + (to.y - from.y) * i / moves, { buttons: 1 });
        await sleep(16);
      }
      const end = await this.app.evaluate('performance.now()');
      await this.app.mouse('mouseReleased', to.x, to.y);
      const gaps = (await this.app.evaluate(`__e2e.gaps(${start}, ${end})`)).sort((a, b) => a - b);
      return { frameGapMax: gaps.length ? gaps.at(-1) : null, frameGapP95: gaps.length ? gaps[Math.min(gaps.length - 1, Math.ceil(gaps.length * 0.95) - 1)] : null };
    } finally {
      await this.app.evaluate('__e2e.stopFrames()').catch(() => {});
    }
  }

  async wheel(deltaY) {
    const at = await this.need('.dash-grid-scroll');
    const x = at.left + at.w - 4;
    await this.app.mouse('mouseMoved', x, at.y);
    await this.app.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x, y: at.y, deltaX: 0, deltaY });
  }

  pages() { return this.app.evaluate(`[...document.querySelectorAll('.sidebar-page-list button.sidebar-subrow')].map(b => b.textContent.trim())`); }
  currentPage() { return this.app.evaluate(`document.querySelector('.sidebar-page-list button[aria-current=page]')?.textContent.trim() ?? null`); }

  async openPage(label, { force = false } = {}) {
    if (!force && await this.holds(pageShown(label))) return;
    await this.click('.sidebar-page-list button.sidebar-subrow', { text: label });
    await this.app.evaluate(`(__e2e.mark(${JSON.stringify(pageShown(label))}), ${settleCall(15000, 300)})`, 60_000);
    await this.app.evaluate(`document.querySelector('.dash-grid-scroll')?.scrollTo({ top: 0 })`);
  }

  static CLOCK_HOUR = 'window.__perf.hour()';

  timeline() {
    return this.app.evaluate(`(() => { const r = document.querySelector('input.header-timeline'); if (!r) return null; const b = r.getBoundingClientRect();
      return { left: b.left, top: b.top, width: b.width, y: b.top + b.height / 2, min: Number(r.min), max: Number(r.max), value: Number(r.value) }; })()`);
  }

  static timelineX(line, fraction) { return line.left + 7 + fraction * (line.width - 14); }
}
