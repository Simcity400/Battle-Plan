
export function setText(node: Node | null | undefined, text: string): void {
  if (node && node.textContent !== text) node.textContent = text;
}

export function setAttr(element: Element | null | undefined, name: string, value: string): void {
  if (element && element.getAttribute(name) !== value) element.setAttribute(name, value);
}

export function setStyle(element: HTMLElement | null | undefined, property: string, value: string): void {
  if (element && element.style.getPropertyValue(property) !== value) element.style.setProperty(property, value);
}
