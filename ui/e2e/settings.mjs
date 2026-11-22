import { fileURLToPath } from 'node:url';
export const repository = fileURLToPath(new URL('../..', import.meta.url));
const underCaptures = (...parts) => {
  if (!(process.env.BATTLEPLAN_CAPTURES ?? process.env.HOI4_CAPTURES)) throw new Error('Set BATTLEPLAN_CAPTURES to the folder holding the parity fixtures');
  return [(process.env.BATTLEPLAN_CAPTURES ?? process.env.HOI4_CAPTURES), ...parts].join('/');
};
export const fixture = {
  get recording() {
    return underCaptures('widgetsyear/library/GER 1943-10-24 to 1944-10-24.battleplan');
  },
  get bridge() { return (process.env.BATTLEPLAN_PARITY_BRIDGE ?? process.env.HOI4_PARITY_BRIDGE) || underCaptures('bridge-release/dashboard-bridge.exe'); },
  get cache() { return (process.env.BATTLEPLAN_PARITY_CACHE ?? process.env.HOI4_PARITY_CACHE) || underCaptures('parity-baselines'); },
  gameDir: (process.env.BATTLEPLAN_GAME_DIR ?? process.env.HOI4_GAME_DIR) || '', userDir: (process.env.BATTLEPLAN_USER_DIR ?? process.env.HOI4_USER_DIR) || '',
  nativeSeed: (process.env.BATTLEPLAN_PARITY_NATIVE_SEED ?? process.env.HOI4_PARITY_NATIVE_SEED) || process.env.LOCALAPPDATA,
  preparedSeed: (process.env.BATTLEPLAN_PARITY_PREPARED_SEED ?? process.env.HOI4_PARITY_PREPARED_SEED) || null,
  matrix: (process.env.BATTLEPLAN_PARITY_MATRIX ?? process.env.HOI4_PARITY_MATRIX) || null,
  hour: 60836486,
};

export const edgeLaunchOptions = {
  channel: 'msedge', headless: true,
  handleSIGINT: false, handleSIGTERM: false, handleSIGHUP: false,
  args: ['--disable-gpu-rasterization', '--disable-accelerated-2d-canvas',
    '--disable-skia-runtime-opts', '--disable-partial-raster', '--force-color-profile=srgb',
    '--disable-checker-imaging', '--run-all-compositor-stages-before-draw', '--disable-background-timer-throttling',
    '--disable-renderer-backgrounding', '--disable-backgrounding-occluded-windows'],
};

export async function launchOwnedBrowser(chromium) {
  if (!(process.env.BATTLEPLAN_OWNED_JOB ?? process.env.HOI4_OWNED_JOB)) throw Error('Owned Job required for Edge');
  const prior = new Set(process.listeners('exit'));
  const browser = await chromium.launch(edgeLaunchOptions);
  const added = process.listeners('exit').filter(listener => !prior.has(listener));
  if (added.length !== 1) throw Error('Unexpected Playwright exit ownership');
  process.removeListener('exit', added[0]);
  return browser;
}

export const contextOptions = {
  viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 1, colorScheme: 'dark', reducedMotion: 'reduce',
  locale: 'en-US', timezoneId: 'UTC',
};

export async function deterministicPage(page) {
  if (await page.evaluate(() => devicePixelRatio) !== 1) {
    throw new Error('The parity fixture requires DPR 1; use the installed Edge fixture');
  }
  await page.addInitScript(() => {
    localStorage.setItem('battleplan.appearance.v2', JSON.stringify({
      version: 2, scheme: 'dark', themes: { dark: 'carbon', light: 'carbon' }, custom: [],
      contrast: 100, glass: 100, radius: 8, motion: 'reduced', headerFlag: 'native',
      fonts: { interface: 'Inter Variable', display: 'Inter Variable', tables: 'Cascadia Code Variable', diagnostics: 'Cascadia Code Variable' },
      typography: { advanced: false, sizes: { interface: 16, display: 14, tables: 13, diagnostics: 12 }, smoothing: true, wordWrap: true },
      backdrop: { kind: 'solid', imageId: null, dim: 55, blur: 0 },
      charts: { motion: 'snappy', countryColours: 'raw' }, sidebarIcons: {},
    }));
  });
}
