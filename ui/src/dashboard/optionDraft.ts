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
    const next = setOption(fields, this.current, key, value);
    if (next) this.current = next;
    return next;
  }
}
