// A deterministic, reviewable outline of what a screen exposes to keyboard and
// assistive-technology users: landmarks/regions, headings, controls with their
// accessible names and states, live regions, lists, and visible text.
//
// Golden tests snapshot this outline instead of raw HTML so a reviewer sees
// workflow structure and copy, not Tailwind class churn. It is intentionally
// a simplified accessibility tree; it does not replace a real screen reader.

const STRUCTURAL_ROLES: Record<string, string> = {
  NAV: "navigation",
  FORM: "form",
  FIELDSET: "group",
  ASIDE: "complementary",
  MAIN: "main",
  OL: "list",
  UL: "list",
  LI: "listitem",
  DL: "definitions",
};

function collapse(text: string | null | undefined): string {
  return (text ?? "").replace(/\s+/g, " ").trim();
}

function textOf(element: Element): string {
  let text = "";
  for (const node of Array.from(element.childNodes)) {
    if (node.nodeType === Node.TEXT_NODE) {
      text += node.textContent ?? "";
    } else if (node instanceof Element && node.getAttribute("aria-hidden") !== "true") {
      if (node instanceof HTMLImageElement) {
        text += ` ${node.alt} `;
      } else {
        text += ` ${textOf(node)} `;
      }
    }
  }
  return collapse(text);
}

function labelledByName(element: Element): string | null {
  const ids = element.getAttribute("aria-labelledby");
  if (!ids) return null;
  const owner = element.ownerDocument;
  return collapse(
    ids
      .split(/\s+/)
      .map((id) => {
        const target = owner.getElementById(id);
        return target ? textOf(target) : "";
      })
      .join(" "),
  );
}

export function accessibleName(element: Element): string {
  const label = element.getAttribute("aria-label");
  if (label) return collapse(label);
  const labelledBy = labelledByName(element);
  if (labelledBy) return labelledBy;
  if (
    element instanceof HTMLInputElement ||
    element instanceof HTMLSelectElement ||
    element instanceof HTMLTextAreaElement
  ) {
    const explicit = element.id
      ? element.ownerDocument.querySelector(`label[for="${element.id}"]`)
      : null;
    const wrapping = element.closest("label");
    const owner = explicit ?? wrapping;
    return owner ? textOf(owner) : "";
  }
  if (element instanceof HTMLFieldSetElement) {
    const legend = element.querySelector(":scope > legend");
    return legend ? textOf(legend) : "";
  }
  return textOf(element);
}

function flags(element: Element): string {
  const values: string[] = [];
  if ((element as HTMLButtonElement).disabled || element.getAttribute("aria-disabled") === "true") {
    values.push("disabled");
  }
  if (element instanceof HTMLInputElement && element.type === "checkbox" && element.checked) {
    values.push("checked");
  }
  for (const attribute of ["aria-current", "aria-selected", "aria-pressed", "aria-busy"]) {
    const value = element.getAttribute(attribute);
    if (value !== null && value !== "false") {
      values.push(`${attribute.replace("aria-", "")}=${value}`);
    }
  }
  const live = element.getAttribute("aria-live");
  if (live) values.push(`live=${live}`);
  return values.length > 0 ? ` [${values.join(", ")}]` : "";
}

function controlLine(element: Element): string | null {
  if (element instanceof HTMLButtonElement) {
    return `button "${accessibleName(element)}"`;
  }
  if (element instanceof HTMLInputElement) {
    if (element.type === "checkbox") return `checkbox "${accessibleName(element)}"`;
    return `textbox "${accessibleName(element)}" = "${element.value}"`;
  }
  if (element instanceof HTMLSelectElement) {
    const selected = element.selectedOptions[0];
    return `combobox "${accessibleName(element)}" = "${collapse(selected?.textContent)}"`;
  }
  if (element instanceof HTMLProgressElement) {
    return `progressbar "${accessibleName(element)}" ${element.value}/${element.max}`;
  }
  if (element instanceof HTMLImageElement) {
    return `img "${element.alt}"`;
  }
  if (/^H[1-6]$/.test(element.tagName)) {
    return `heading${element.tagName.slice(1)} "${textOf(element)}"`;
  }
  return null;
}

function containerLine(element: Element): string | null {
  const role = element.getAttribute("role");
  if (role === "status" || role === "alert" || role === "tablist" || role === "tabpanel") {
    // Live regions are named only by an explicit label; their content follows.
    const name =
      role === "tablist" || role === "tabpanel"
        ? accessibleName(element)
        : (element.getAttribute("aria-label") ?? labelledByName(element));
    return name ? `${role} "${collapse(name)}"` : role;
  }
  if (role === "tab") return null;
  if (element.tagName === "SECTION") {
    const name = element.getAttribute("aria-label") ?? (labelledByName(element) || null);
    return name ? `region "${collapse(name)}"` : null;
  }
  const structural = STRUCTURAL_ROLES[element.tagName];
  if (structural) {
    const name = element.getAttribute("aria-label") ?? labelledByName(element);
    return name ? `${structural} "${collapse(name)}"` : structural;
  }
  if (element.getAttribute("aria-live")) return "live-region";
  return null;
}

function directText(element: Element): string {
  return collapse(
    Array.from(element.childNodes)
      .filter((node) => node.nodeType === Node.TEXT_NODE)
      .map((node) => node.textContent ?? "")
      .join(""),
  );
}

function walk(element: Element, depth: number, lines: string[]): void {
  if (element.getAttribute("aria-hidden") === "true" || element.tagName === "OPTION") return;
  if (element.tagName === "LEGEND") return;
  const indent = "  ".repeat(depth);
  if (element.getAttribute("role") === "tab") {
    lines.push(`${indent}tab "${accessibleName(element)}"${flags(element)}`);
    return;
  }
  const control = controlLine(element);
  if (control) {
    lines.push(`${indent}${control}${flags(element)}`);
    return;
  }
  const container = containerLine(element);
  let childDepth = depth;
  if (container) {
    lines.push(`${indent}${container}${flags(element)}`);
    childDepth = depth + 1;
  }
  const text = directText(element);
  const labelOnlyForControl =
    element instanceof HTMLLabelElement && element.querySelector("input, select, textarea");
  if (text && !labelOnlyForControl && !(element instanceof HTMLLabelElement && element.htmlFor)) {
    const annotations = container ? "" : flags(element);
    lines.push(`${"  ".repeat(childDepth)}"${text}"${annotations}`);
  }
  for (const child of Array.from(element.children)) {
    walk(child, childDepth, lines);
  }
}

export function accessibilityOutline(root: Element): string {
  const lines: string[] = [];
  for (const child of Array.from(root.children)) {
    walk(child, 0, lines);
  }
  return `\n${lines.join("\n")}\n`;
}
