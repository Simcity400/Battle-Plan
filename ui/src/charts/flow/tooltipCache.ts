import type { TooltipContent } from '../tooltip';

/** A still hover reuses its content until its hit or tooltip inputs change. */
export class FlowTooltipCache<Hit> {
  private id: string | null = null;
  private callback: ((hit: Hit) => TooltipContent | null) | undefined;
  private content: TooltipContent | null = null;
  private revision = 0;

  describe(id: string, hit: Hit, callback: ((hit: Hit) => TooltipContent | null) | undefined): { content: TooltipContent | null; key: string } {
    if (id !== this.id || callback !== this.callback) {
      this.id = id;
      this.callback = callback;
      this.content = callback?.(hit) ?? null;
      this.revision++;
    }
    return { content: this.content, key: String(this.revision) };
  }

  clear(): void { this.id = null; this.callback = undefined; this.content = null; }
}
