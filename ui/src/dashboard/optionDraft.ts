import { setOption } from './options.ts';
import type { OptionField, OptionValue, WidgetOptions } from './options';

/** Compose several immediate option saves before the parent publishes new props. */
export class OptionDraft {
  private supplied: WidgetOptions | undefined;
  private current: WidgetOptions | undefined;

  constructor(options?: WidgetOptions) { this.supplied = this.current = options; }

  sync(options: WidgetOptions | undefined): void {
    if (options !== this.supplied) this.supplied = this.current = options;
  }

  update(fields: readonly OptionField[] | undefined, key: string, value: OptionValue): WidgetOptions | null {
    return this.updateAll(fields, { [key]: value });
  }

  /** Several options as one change: all are applied, or none if any is invalid. */
  updateAll(fields: readonly OptionField[] | undefined, values: Readonly<Record<string, OptionValue>>): WidgetOptions | null {
    let next: WidgetOptions | null | undefined = this.current;
    for (const [key, value] of Object.entries(values)) {
      next = setOption(fields, next ?? undefined, key, value);
      if (!next) return null;
    }
    if (next) this.current = next;
    return next ?? null;
  }
}
