/** Loadables mounted in a real browser, whose loads the test settles itself. */
import { Component, StrictMode } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { loadable } from './loadable';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 4; i++) await frame(); }

type Props = { name: string };
type Pending = { resolve: (component: (props: Props) => ReactNode) => void; reject: (error: Error) => void };
const pending: Record<string, Pending[]> = { Shown: [], Flaky: [] };
/** How many times each kind rendered its fallback. */
const fallbacks: Record<string, number> = { Shown: 0, Flaky: 0 };
const make = (kind: string) => loadable<Props>(() => new Promise((resolve, reject) => { pending[kind].push({ resolve, reject }); }),
  ({ name }) => { fallbacks[kind]++; return <i>loading {name}</i>; });
const kinds: Record<string, ReturnType<typeof make>> = { Shown: make('Shown'), Flaky: make('Flaky') };

class Boundary extends Component<{ children: ReactNode }, { error: string | null }> {
  state: { error: string | null } = { error: null };
  static getDerivedStateFromError(error: Error) { return { error: error.message }; }
  render() { return this.state.error ? <b>failed {this.state.error}</b> : this.props.children; }
}

const host = document.body.appendChild(document.createElement('div'));
const root = createRoot(host);
let generation = 0;

export const fixture = {
  /** Mounts a fresh boundary holding one `kind` loadable per name, as a widget's Retry remounts it. */
  async mount(kind: string, names: string[]) {
    const Loaded = kinds[kind];
    generation++;
    root.render(<StrictMode><Boundary key={generation}>{names.map(name => <Loaded key={name} name={name} />)}</Boundary></StrictMode>);
    await settle();
  },
  loads: (kind: string) => pending[kind].length,
  fallbacks: (kind: string) => fallbacks[kind],
  async resolve(kind: string) { pending[kind].at(-1)!.resolve(({ name }) => <span>shown {name}</span>); await settle(); },
  async reject(kind: string) { pending[kind].at(-1)!.reject(new Error('synthetic chunk failure')); await settle(); },
  text: () => host.textContent,
};
(window as unknown as { loadableFixture: typeof fixture }).loadableFixture = fixture;
