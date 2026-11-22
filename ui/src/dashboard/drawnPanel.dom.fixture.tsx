import { paintFields } from './DrawnPanel.tsx';
import { panelViews } from '../draw/panels/views.ts';
import '../widgets/special-projects/specialProjects.css';

declare global { interface Window { drawnPanel: unknown } }

window.drawnPanel = {
  project(progress: number, texts: [string, string][]) {
    document.documentElement.dataset.motion = 'reduced';
    document.body.innerHTML = `<div class="special-project-list"><div data-project="fast" class="special-project-now" style="--project-progress:${progress}">
      <span class="special-project-now-left"><strong data-paint="progress">${progress * 100}%</strong><small data-paint="iterations">1</small></span>
      <span class="special-project-progress" aria-hidden="true"><span></span><i></i></span></div></div>`;
    const root = document.body.firstElementChild as HTMLElement;
    paintFields(root, new Map(texts), panelViews['special-projects']!.geometry!);
    const track = root.querySelector('.special-project-progress')!.getBoundingClientRect(), bar = root.querySelector('.special-project-progress > span')!.getBoundingClientRect();
    return { fill: Math.round(bar.width / track.width * 100) / 100, progress: root.querySelector('[data-paint="progress"]')!.textContent,
      iterations: root.querySelector('[data-paint="iterations"]')!.textContent };
  },
};
