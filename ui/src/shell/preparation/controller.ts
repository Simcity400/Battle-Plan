import type { DashboardClient, DashboardSource } from '../../dashboard/client.ts';
import { newId } from '../ids.ts';
import { preparationActive, validatePreparationReply } from './contract.ts';
import type { PreparationAction, PreparationReply } from './contract.ts';
export interface PreparationState { source: DashboardSource | null; reply: PreparationReply | null; loading: boolean; pending: boolean; error: string | null }
/** One visible campaign, one pending request, no retained campaign histories. */
export class PreparationController {
  private state: PreparationState = { source: null, reply: null, loading: true, pending: false, error: null };
  private listeners = new Set<() => void>();
  private request: AbortController | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private active = false;
  private owned = false;
  private client: DashboardClient;
  constructor(client: DashboardClient) { this.client = client; }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(next: PreparationState) { this.state = next; this.listeners.forEach(listener => listener()); }
  async start(path: string, source: DashboardSource | null) {
    this.active = true;
    this.update({ source: null, reply: null, loading: true, pending: false, error: null });
    const opening = new AbortController(); this.request = opening;
    try {
      const opened = source ?? await this.client.openSource(path, opening.signal);
      if (!this.active || opening.signal.aborted) { if (!source) await this.client.releaseSource(opened.id); return; }
      this.owned = !source;
      this.update({ ...this.state, source: opened });
      await this.command('status');
    } catch (error) { if (this.active && !opening.signal.aborted) this.update({ ...this.state, loading: false, error: String(error) }); }
  }
  async command(action: PreparationAction) {
    const source = this.state.source;
    if (!this.active || !source || this.state.pending && action !== 'status') return;
    if (this.timer) clearTimeout(this.timer); this.timer = null;
    this.request?.abort();
    const request = new AbortController(); this.request = request;
    const query = { sourceId: source.id, revision: source.revision, requestId: newId() };
    this.update({ ...this.state, pending: action !== 'status', error: null });
    try {
      const method = action === 'status' ? this.client.preparationStatus : action === 'start' ? this.client.prepareSource : action === 'cancel' ? this.client.cancelPreparation : this.client.clearPreparation;
      if (!method) throw new Error('Preparation is unavailable in this server version.');
      const reply = validatePreparationReply(await method.call(this.client, query, request.signal), query);
      if (request.signal.aborted || !this.active) return;
      this.update({ ...this.state, reply, loading: false, pending: false });
      if (preparationActive(reply)) this.timer = setTimeout(() => { void this.command('status'); }, 500);
    } catch (error) {
      if (!request.signal.aborted && this.active) this.update({ ...this.state, loading: false, pending: false, error: String(error) });
    }
  }
  stop() {
    this.active = false; this.request?.abort(); this.request = null;
    if (this.timer) clearTimeout(this.timer); this.timer = null;
    const source = this.state.source;
    if (this.owned && source) void this.client.releaseSource(source.id).catch(() => {});
    this.owned = false;
  }
}
