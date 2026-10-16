import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

const stylesheet = readFileSync("src/styles.css", "utf8");
const colors = Object.fromEntries(
  [...stylesheet.matchAll(/--color-sd-([\w-]+):\s*(#[\da-fA-F]{6});/g)].map((match) => [
    match[1],
    match[2],
  ]),
);

function luminance(hex: string): number {
  const linearize = (channel: string) => {
    const value = Number.parseInt(channel, 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  const red = linearize(hex.slice(1, 3));
  const green = linearize(hex.slice(3, 5));
  const blue = linearize(hex.slice(5, 7));
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

function contrastRatio(foreground: string, background: string): number {
  const first = luminance(foreground);
  const second = luminance(background);
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

describe("shared workflow color tokens", () => {
  it.each([
    ["on-background", "background"],
    ["on-surface", "surface"],
    ["on-surface-variant", "surface"],
    ["on-surface-variant", "surface-variant"],
    ["on-primary", "primary"],
    ["on-primary-container", "primary-container"],
    ["on-secondary-container", "secondary-container"],
    ["on-error-container", "error-container"],
  ])("keeps %s readable on %s at WCAG AA text contrast", (foreground, background) => {
    const foregroundColor = colors[foreground];
    const backgroundColor = colors[background];
    if (!foregroundColor || !backgroundColor) {
      throw new Error(`missing color token pair ${foreground}/${background}`);
    }
    expect(contrastRatio(foregroundColor, backgroundColor)).toBeGreaterThanOrEqual(4.5);
  });
});
