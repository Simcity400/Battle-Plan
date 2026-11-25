export async function validateHook(page, configured = false) {
  await page.evaluate(async configured => {
    if (window.__hoi4ParityReady) await window.__hoi4ParityReady;
    const hook = window.__hoi4Parity;
    const required = ['registry', 'blank', 'open', 'isolate', 'seek', 'snapshot', 'requests', 'lists', 'prepareLists'];
    if (!hook || required.some(name => typeof hook[name] !== 'function'))
      throw Error('Baseline and head must be instrumented parity builds with all required hook capabilities');
    if (configured && (typeof hook.capabilities !== 'function' || hook.capabilities().scenarioVersion !== 1))
      throw Error('Explicit capture cases require scenarioVersion 1 instrumentation in both worktrees');
  }, configured);
}

export function createApp(page, config) {
  const errors = [], timeout = 60_000;
  const paint = () => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  page.setDefaultTimeout(timeout);
  page.on('console', message => { if (message.type() === 'error') errors.push({ text: message.text() }); });
  page.on('pageerror', error => errors.push({ text: String(error) }));
  page.on('response', async response => {
    if (!response.ok() && response.url().includes('/api/recordings')) {
      const body = await response.text().catch(() => 'Response body unavailable');
      errors.push({ text: `${response.status()}: ${body}` });
    }
  });
  const call = (method, ...args) => page.evaluate(({ method, args }) => window.__hoi4Parity[method](...args), { method, args });
  const app = {
    errors,
    async prepare() {
      await page.addInitScript(() => {
        window.__hoi4ParityBrowserErrors = [];
        window.addEventListener('error', event => {
          if (event instanceof ErrorEvent && !event.error) window.__hoi4ParityBrowserErrors.push({ type: 'browser', text: event.message });
        });
      });
      await page.goto(config.url, { waitUntil: 'domcontentloaded' });
      await page.getByRole('navigation', { name: 'Views' }).getByRole('button', { name: 'Overview', exact: true }).click();
      await page.locator('.dash-grid').waitFor({ state: 'attached' });
      await validateHook(page, !!config.scenario);
      await call('blank');
      await paint();
      const opened = await call('open', config.recording);
      if (opened === null) throw Error(`Recording open failed: ${await page.locator('.dashboard').innerText()} ${JSON.stringify(errors)}`);
      await page.waitForFunction(() => window.__hoi4Parity.snapshot().tracks.length > 0);
      const clock = await call('snapshot');
      if (config.hour < clock.minHour || config.hour > clock.maxHour) throw Error('Fixture hour is outside recording bounds');
      await call('seek', config.hour);
      await page.evaluate(() => document.fonts.ready);
    },
    registry: () => call('registry'),
    async install(kind) {
      errors.length = 0;
      await page.evaluate(() => { window.__hoi4ParityBrowserErrors.length = 0; });
      const scenario = config.scenario;
      await call('isolate', kind, { height: scenario?.height ?? 18, hour: config.hour,
        ...(scenario?.countries !== undefined ? { countries: scenario.countries } : {}),
        ...(scenario?.time !== undefined ? { time: scenario.time } : {}),
        ...(scenario?.options !== undefined ? { options: scenario.options } : {}) });
      await paint();
    },
    async act(locator) {
      const ordinal = config.scenario?.focusDetails;
      if (ordinal === undefined) return null;
      const number = ordinal === 'first-visible' ? locator.locator('.focus-number[aria-label]:visible').first()
        : locator.getByLabel(`Focus ${BigInt(ordinal) + 1n}`, { exact: true });
      const label = await number.getAttribute('aria-label');
      if (!/^Focus \d+$/.test(label ?? '')) throw Error('Requested completion control is unavailable');
      await number.locator('xpath=..').click();
      await paint();
      return { focusDetailsOrdinal: (BigInt(label.slice(6)) - 1n).toString() };
    },
    widget: kind => page.locator(`.dash-widget.kind-${kind}`).first(),
    fullRows: root => root.evaluate(element => window.__hoi4Parity.lists(element)),
    prepareFullRows: (root, budget) => root.evaluate((element, timeout) => window.__hoi4Parity.prepareLists(element, timeout), budget),
    async settle(locator) {
      await locator.waitFor({ state: 'attached', timeout });
      const sample = await locator.evaluate(async (root, budget) => {
        const began = performance.now(); let previous = '', stableAt = began, last;
        while (performance.now() - began < budget) {
        await new Promise(resolve => requestAnimationFrame(resolve));
        const visibleGeometry = element => {
          if (getComputedStyle(element).visibility === 'hidden') return false;
          const box = element.getBoundingClientRect(), bounds = root.getBoundingClientRect();
          let left = Math.max(0, bounds.left, box.left), right = Math.min(innerWidth, bounds.right, box.right);
          let top = Math.max(0, bounds.top, box.top), bottom = Math.min(innerHeight, bounds.bottom, box.bottom);
          for (let ancestor = element.parentElement; ancestor; ancestor = ancestor.parentElement) {
            const style = getComputedStyle(ancestor), clip = ancestor.getBoundingClientRect();
            if (/(auto|scroll|hidden|clip)/.test(style.overflowX)) { left = Math.max(left, clip.left); right = Math.min(right, clip.right); }
            if (/(auto|scroll|hidden|clip)/.test(style.overflowY)) { top = Math.max(top, clip.top); bottom = Math.min(bottom, clip.bottom); }
          }
          return right > left && bottom > top;
        };
        const shown = element => !element.closest('[aria-hidden=true]') && visibleGeometry(element);
        const geometry = element => {
          const box = element.getBoundingClientRect();
          return { x: box.x, y: box.y, width: box.width, height: box.height };
        };
        const canvas = [...root.querySelectorAll('canvas')].filter(visibleGeometry).map(canvas => {
          const pixels = canvas.toDataURL();
          return { width: canvas.width, height: canvas.height, pixels, geometry: geometry(canvas) };
        });
        const loading = [...root.querySelectorAll('.list-skeleton,.spinner,[data-parity-loading],[aria-busy=true],[aria-label^="Loading"],[aria-label="Flag loading"]')].filter(shown).length;
        const images = [...root.querySelectorAll('img')].filter(image => visibleGeometry(image) && !image.complete).length;
        const mounted = !root.matches('.dash-widget') || !!root.querySelector('.dash-widget-header h2') && !!root.querySelector('.dash-widget-scaled')?.children.length;
        const state = { text: root.innerText, geometry: geometry(root), canvas, loading, images, mounted, browserErrors: window.__hoi4ParityBrowserErrors ?? [],
          fonts: document.fonts.status, nativePending: window.__hoi4Parity?.requests().length ?? 0 };
        const signature = JSON.stringify(state), now = performance.now();
        if (signature !== previous) { previous = signature; stableAt = now; }
        last = state;
        if (!state.nativePending && !state.loading && !state.images && state.mounted && state.fonts === 'loaded' && now - stableAt >= 120) {
          return { settled: true, state };
        }
        }
        return { settled: false, state: last };
      }, timeout);
      const state = sample.state;
      for (const error of state.browserErrors) if (!errors.some(existing => existing.type === error.type && existing.text === error.text)) errors.push(error);
      if (sample.settled) return;
      throw new Error(`Visible content did not settle in ${timeout} ms: ${JSON.stringify(state)}`);
    },
  };
  return app;
}
