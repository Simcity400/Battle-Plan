export type OnScreen = (element: Element, changed: (visible: boolean) => void) => () => void;

export const NEAR_SCREEN = '240px 0px';
export const DASHBOARD_SCROLL = '.dash-grid-scroll';

export const screenHolder = (element: Element) => element.closest('article.dash-widget, [popover]') ?? element;

export const observeOnScreen: OnScreen = (element, changed) => {
  if (typeof IntersectionObserver !== 'function') { changed(true); return () => {}; }
  element = screenHolder(element);
  const root = element.closest(DASHBOARD_SCROLL);
  const reach: IntersectionObserverInit | undefined = root ? { root, rootMargin: NEAR_SCREEN } : undefined;
  if (getComputedStyle(element).display !== 'contents') {
    const observer = new IntersectionObserver(entries => { for (const entry of entries) changed(entry.isIntersecting); }, reach);
    observer.observe(element);
    return () => observer.disconnect();
  }
  const seen = new Map<Element, boolean>();
  let told: boolean | null = null;
  const tell = () => {
    const visible = [...seen.values()].some(Boolean);
    if (visible !== told) { told = visible; changed(visible); }
  };
  const observer = new IntersectionObserver(entries => {
    for (const entry of entries) if (seen.has(entry.target)) seen.set(entry.target, entry.isIntersecting);
    tell();
  }, reach);
  const watch = () => {
    const children = new Set(element.children);
    let added = false;
    for (const child of [...seen.keys()]) if (!children.has(child)) { observer.unobserve(child); seen.delete(child); }
    for (const child of children) if (!seen.has(child)) { seen.set(child, false); observer.observe(child); added = true; }
    if (!added && told !== null) tell();
  };
  watch();
  const children = new MutationObserver(watch);
  children.observe(element, { childList: true });
  return () => { children.disconnect(); observer.disconnect(); };
};
