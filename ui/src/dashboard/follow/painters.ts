import { CELL, cellNumber, cellState, cellValue } from './message.ts';
import type { FollowedAnswer } from './message.ts';
import { barGlides } from '../../engine/glide.ts';

export class TextCell {
  text: string;
  title: string;
  private readonly node: { data: string };
  private readonly host: { title: string };

  constructor(node: { data: string }, host: { title: string }) {
    this.node = node;
    this.host = host;
    this.text = node.data;
    this.title = host.title;
  }

  put(text: string, title: string): void {
    if (this.text !== text) { this.node.data = text; this.text = text; }
    if (this.title !== title) { this.host.title = title; this.title = title; }
  }
}

export function textCell(element: HTMLElement): TextCell {
  const only = element.firstChild;
  const node = only && only.nodeType === 3 && !only.nextSibling ? only as Text : document.createTextNode(element.textContent ?? '');
  if (node !== only) element.replaceChildren(node);
  return new TextCell(node, element);
}

export class BarCell {
  width: string;
  private readonly style: { width: string; transform?: string };
  private readonly scaled: boolean;
  private readonly glide: () => number;

  constructor(style: { width: string; transform?: string }, scaled = false, glide: () => number = () => 0) {
    this.style = style;
    this.scaled = scaled;
    this.glide = glide;
    this.width = scaled ? '' : style.width;
  }

  put(width: string): void {
    if (this.width === width) return;
    this.width = width;
    if (this.scaled) barGlides.to(this.style, (parseFloat(width) || 0) / 100, this.glide()); else this.style.width = width;
  }
}

export function rowAmount(answer: FollowedAnswer | undefined, row: number, scale: number): number | null | undefined {
  if (!answer) return undefined;
  if (answer.kind === 'notRecorded' || row >= answer.rows || cellState(answer, row * answer.columns) !== CELL.value) return null;
  const value = cellNumber(answer, row * answer.columns);
  return (value ?? Number(cellValue(answer, row * answer.columns))) / 10 ** scale;
}

