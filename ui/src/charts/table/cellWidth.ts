import type { ComponentType, ReactNode } from 'react';

const widths = new WeakMap<object, (props: never) => ReactNode>();

/** A composed cell declares the text and geometry its width depends on. */
export function defineCellWidth<P>(component: ComponentType<P>, render: (props: P) => ReactNode): void {
  widths.set(component, render);
}

export function renderedCellWidth(component: unknown, props: unknown): { content: ReactNode } | undefined {
  if ((typeof component !== 'function' && typeof component !== 'object') || component === null) return undefined;
  const render = widths.get(component);
  return render ? { content: render(props as never) } : undefined;
}
