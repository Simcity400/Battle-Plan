import { paintFields } from './DrawnPanel.tsx';
import type { PanelGeometry } from '../draw/panels/view.ts';

const GEOMETRY: PanelGeometry = { rowAttribute: 'data-project', extraFields: '.special-project-progress > span', meters: '.special-project-progress',
  barVariable: { host: '[data-project]', name: '--project-progress' } };
const STYLE = '.special-project-progress { position:relative; display:block; width:200px; height:6px; overflow:hidden; } .special-project-progress > span { position:absolute; inset:0; transform-origin:left; transform:scaleX(var(--project-progress)); }';

declare global { interface Window { drawnPanel: unknown } }

window.drawnPanel = {
  project(progress: number, texts: [string, string][]) {
    document.documentElement.dataset.motion = 'reduced';
    document.body.innerHTML = `<style>${STYLE}</style><div class="special-project-list"><div data-project="fast" class="special-project-now" style="--project-progress:${progress}">
      <span class="special-project-now-left"><strong data-paint="progress">${progress * 100}%</strong><small data-paint="iterations">1</small></span>
      <span class="special-project-progress" aria-hidden="true"><span></span><i></i></span></div></div>`;
    const root = document.body.querySelector('.special-project-list') as HTMLElement;
    paintFields(root, new Map(texts), GEOMETRY);
    const track = root.querySelector('.special-project-progress')!.getBoundingClientRect(), bar = root.querySelector('.special-project-progress > span')!.getBoundingClientRect();
    return { fill: Math.round(bar.width / track.width * 100) / 100, progress: root.querySelector('[data-paint="progress"]')!.textContent,
      iterations: root.querySelector('[data-paint="iterations"]')!.textContent };
  },
};
