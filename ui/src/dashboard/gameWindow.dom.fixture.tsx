import { act } from 'react';
import { createRoot } from 'react-dom/client';
import GameWindowView from './GameWindowView';
import type { DashboardClient, GameWindowRender, GameWindowRequest } from './client';

Object.assign(window, { IS_REACT_ACT_ENVIRONMENT: true });

const root = createRoot(document.body.appendChild(document.createElement('div')));
const hours: number[] = [];
const pending: Array<{ resolve: (render: GameWindowRender) => void; reject: (error: Error) => void }> = [];
const client = {
  renderGameWindow(request: GameWindowRequest) {
    hours.push(request.hour);
    return new Promise<GameWindowRender>((resolve, reject) => pending.push({ resolve, reject }));
  },
} as unknown as DashboardClient;

const retryButton = () => [...document.querySelectorAll('button')].find(button => button.textContent === 'Retry');
const state = () => ({
  hours,
  inFlight: pending.length,
  image: document.querySelector<HTMLImageElement>('.game-window-canvas img')?.alt ?? null,
  error: document.querySelector('.widget-state-title')?.textContent ?? null,
  retry: Boolean(retryButton()),
});

Object.assign(window, { gameWindow: {
  async reset() {
    await act(() => root.render(null));
    hours.length = 0;
    pending.length = 0;
  },
  async open(hour: number, generation = 1) {
    await act(() => root.render(<GameWindowView client={client} generation={generation} active label="equipment designer"
      request={{ sourceId: 'fixture', revision: '1', branchId: '1', hour, kind: 'equipmentDesigner' }} />));
    return state();
  },
  async cancel() {
    await act(() => pending.shift()!.reject(new Error('Request cancelled')));
    return state();
  },
  async show(title: string) {
    await act(() => pending.shift()!.resolve({
      title, readHour: hours.at(-1)!, pngBase64: '', logicalSize: [32, 32], pixelSize: [32, 32], scale: 1,
      artFromCurrentInstall: false, artLabel: null, regions: [], warnings: [], text: [], placeholders: [],
    }));
    return state();
  },
  async retry() {
    await act(() => retryButton()!.click());
    return state();
  },
} });
