import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useChartTokens } from './useChartTokens';
import { fittedListCount, fittedListRemainder } from './fittedListData.ts';
import './fittedList.css';

interface FittedListProps<T> {
  items: readonly T[];
  className: string;
  moreClassName: string;
  itemKey: (item: T) => string;
  label: (item: T) => string;
  describe?: (item: T) => string;
  extraWidth: (item: T) => number;
  renderItem: (item: T) => ReactNode;
}

export default function FittedList<T>({ items, className, moreClassName, itemKey, label, describe = label, extraWidth, renderItem }: FittedListProps<T>) {
  const list = useRef<HTMLDivElement>(null);
  const probe = useRef<HTMLSpanElement>(null);
  const [shown, setShown] = useState(0);
  const { font } = useChartTokens();
  useLayoutEffect(() => {
    const node = list.current;
    const more = probe.current;
    if (!node || !more) return;
    const context = document.createElement('canvas').getContext('2d');
    if (!context) return;
    const measure = () => {
      const style = getComputedStyle(node);
      const moreStyle = getComputedStyle(more);
      const gap = Number.parseFloat(style.columnGap) || 0;
      const moreExtra = ['paddingLeft', 'paddingRight', 'borderLeftWidth', 'borderRightWidth']
        .reduce((sum, key) => sum + (Number.parseFloat(moreStyle[key as keyof CSSStyleDeclaration] as string) || 0), 0);
      const textWidth = (text: string, textFont: string) => { context.font = textFont; return context.measureText(text).width; };
      setShown(fittedListCount(items.length, node.clientWidth,
        index => textWidth(label(items[index]), style.font) + extraWidth(items[index]),
        hidden => textWidth(`+${hidden} more`, moreStyle.font) + moreExtra, gap));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    document.fonts?.addEventListener('loadingdone', measure);
    return () => { observer.disconnect(); document.fonts?.removeEventListener('loadingdone', measure); };
  }, [items, label, extraWidth, font]);
  const count = Math.min(shown, items.length);
  const remaining = useMemo(() => fittedListRemainder(items, count, describe), [items, count, describe]);
  return <div ref={list} className={className}>
    {items.slice(0, count).map(item => <span key={itemKey(item)} className="fitted-list-item">{renderItem(item)}</span>)}
    {count < items.length && <span className={moreClassName} tabIndex={0} data-tooltip={remaining}>+{items.length - count} more</span>}
    <span ref={probe} className={`${moreClassName} fitted-list-probe`} aria-hidden="true">+0 more</span>
  </div>;
}
