/** Writes a painter makes each frame, each only when it changes the node: an unchanged write still dirties style or
 * layout, and a page that lays out every frame drops frames however little it draws. */

export function setText(node: Node | null | undefined, text: string): void {
  if (node && node.textContent !== text) node.textContent = text;
}

export function setAttr(element: Element | null | undefined, name: string, value: string): void {
  if (element && element.getAttribute(name) !== value) element.setAttribute(name, value);
}

export function setHidden(element: HTMLElement | null | undefined, hidden: boolean): void {
  if (element && element.hidden !== hidden) element.hidden = hidden;
}

export function setStyle(element: HTMLElement | null | undefined, property: string, value: string): void {
  if (element && element.style.getPropertyValue(property) !== value) element.style.setProperty(property, value);
}
