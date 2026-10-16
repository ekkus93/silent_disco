import type { UserEvent } from "@testing-library/user-event";

import { accessibleName } from "./accessibilityOutline";

function describeFocus(element: Element): string {
  const role =
    element.getAttribute("role") ??
    (element instanceof HTMLButtonElement
      ? "button"
      : element instanceof HTMLSelectElement
        ? "combobox"
        : element instanceof HTMLInputElement && element.type === "checkbox"
          ? "checkbox"
          : element instanceof HTMLInputElement
            ? "textbox"
            : element.tagName.toLowerCase());
  return `${role} "${accessibleName(element)}"`;
}

/**
 * Presses Tab from the current focus until focus leaves the document or wraps,
 * returning each focused control as `role "accessible name"`. This is the
 * order a keyboard-only user actually reaches controls in jsdom, which follows
 * DOM order and skips disabled controls.
 */
export async function tabSequence(user: UserEvent, limit = 80): Promise<string[]> {
  const seen: string[] = [];
  const visited = new Set<Element>();
  for (let index = 0; index < limit; index += 1) {
    await user.tab();
    const active = document.activeElement;
    if (!active || active === document.body || visited.has(active)) break;
    visited.add(active);
    seen.push(describeFocus(active));
  }
  return seen;
}

export function focusedDescription(): string {
  const active = document.activeElement;
  return active && active !== document.body ? describeFocus(active) : "body";
}
