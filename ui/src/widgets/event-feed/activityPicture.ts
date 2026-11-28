import { formatDay } from '../../dashboard/calendar.ts';
import type { HourRanges } from '../../families/holders.ts';
import { stripBinCoverage, type StripState } from './activityStrip.ts';

function child(parent: HTMLElement, index: number, tag: 'span' | 'i'): HTMLElement {
  return (parent.children.item(index) as HTMLElement | null) ?? parent.appendChild(parent.ownerDocument.createElement(tag));
}

function trim(parent: HTMLElement, length: number) {
  while (parent.children.length > length) parent.lastElementChild!.remove();
}

export function createActivityPicture(host: HTMLDivElement) {
  const bars = host.ownerDocument.createElement('div');
  bars.className = 'efs-bars';
  const window = host.ownerDocument.createElement('div');
  window.className = 'efs-win';
  window.setAttribute('aria-hidden', 'true');
  const axis = host.ownerDocument.createElement('div');
  axis.className = 'efs-axis';

  return (strip: StripState | null, groupGaps?: HourRanges, reason?: string | null) => {
    const visible = !!(strip || reason);
    const className = reason ? 'efs-strip is-unavailable' : strip ? 'efs-strip' : '';
    if (host.className !== className) host.className = className;
    if (host.hidden === visible) host.hidden = !visible;
    if (visible && !host.hasAttribute('aria-label')) host.setAttribute('aria-label', 'Event activity strip');
    if (!visible) host.removeAttribute('aria-label');
    if (reason) {
      host.setAttribute('role', 'status');
      if (host.textContent !== reason || host.childElementCount) host.textContent = reason;
      return;
    }
    host.removeAttribute('role');
    if (!strip) {
      if (host.firstChild) host.replaceChildren();
      return;
    }
    if (bars.parentNode !== host) host.replaceChildren(bars, window, axis);

    const totals = new Array<number>(strip.binCount).fill(0);
    for (const series of strip.series) {
      for (let i = 0; i < strip.binCount; i++) totals[i]! += series.counts[i] ?? 0;
    }
    const maxTotal = Math.max(1, ...totals);
    const { gapBins, partBins } = stripBinCoverage(strip.firstHour, strip.boundFirstHour,
      strip.lastHour, strip.binHours, strip.binCount, strip.missing, groupGaps);

    for (let i = 0; i < strip.binCount; i++) {
      const bar = child(bars, i, 'span');
      const missing = gapBins.has(i) && totals[i] === 0;
      const partly = gapBins.has(i) || partBins.has(i);
      const className = missing ? 'efs-gap' : partly ? 'efs-bar is-partial' : 'efs-bar';
      const title = missing ? 'Not recorded' : partly ? 'Partly not recorded' : '';
      if (bar.className !== className) bar.className = className;
      if (bar.title !== title) {
        if (title) bar.title = title;
        else bar.removeAttribute('title');
      }
      let segments = 0;
      for (const series of strip.series) {
        const count = series.counts[i] ?? 0;
        if (count <= 0) continue;
        const segment = child(bar, segments++, 'i');
        const className = `efs-seg is-${series.category}`;
        const height = `${Math.round(Math.max(1, (count / maxTotal) * 22) * 10) / 10}px`;
        if (segment.className !== className) segment.className = className;
        if (segment.style.height !== height) segment.style.height = height;
      }
      trim(bar, segments);
    }
    trim(bars, strip.binCount);

    const windowWidth = String(Math.max(6, Math.min(18, Math.round((3 / Math.max(1, strip.binCount)) * 100))));
    if (window.style.getPropertyValue('--efs-window-width') !== windowWidth) window.style.setProperty('--efs-window-width', windowWidth);
    const tickCount = Math.min(5, Math.max(2, Math.floor(strip.binCount / 10) + 1));
    for (let i = 0; i < tickCount; i++) {
      const tick = child(axis, i, 'span');
      const frac = i / (tickCount - 1);
      const hour = Math.round(strip.firstHour + frac * Math.max(0, strip.lastHour - strip.firstHour));
      const label = formatDay(hour).split(' ').slice(0, 2).join(' ');
      const className = `efs-tick${i === 0 ? ' is-first' : i === tickCount - 1 ? ' is-last' : ''}`;
      const position = String(frac);
      if (tick.className !== className) tick.className = className;
      if (tick.style.getPropertyValue('--efs-tick-position') !== position) tick.style.setProperty('--efs-tick-position', position);
      if (tick.textContent !== label) tick.textContent = label;
    }
    trim(axis, tickCount);
  };
}
