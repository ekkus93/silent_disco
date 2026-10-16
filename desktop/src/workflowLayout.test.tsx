// jsdom does not lay out pages or evaluate media queries, so compact-window
// and reduced-motion behavior is verified as a contract between the real
// stylesheet and the real rendered DOM: every compact rule must target markup
// that exists, must not hide an action, and the reduced-motion override must
// be able to win over every motion declaration. Pixel-level verification at
// the minimum window size still needs a real webview.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

import { screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  coreSnapshot,
  hostSession,
  listener,
  networkSnapshot,
  pendingRequest,
} from "./test/hostWorkflowFixtures";
import { renderHostApp } from "./test/renderHostApp";

const mocks = vi.hoisted(() => ({
  ensureDesktopBridge: vi.fn(),
  subscribeDesktopNotifications: vi.fn(),
  getHostSessionState: vi.fn(),
  getHostNetworkState: vi.fn(),
  getLabModeAvailable: vi.fn(),
  getAppShutdownState: vi.fn(),
}));

vi.mock("./core/bridge", () => ({
  ensureDesktopBridge: mocks.ensureDesktopBridge,
  subscribeDesktopNotifications: mocks.subscribeDesktopNotifications,
}));
vi.mock("./core/client", async () => {
  const actual = await vi.importActual<typeof import("./core/client")>("./core/client");
  return {
    ...actual,
    getHostSessionState: mocks.getHostSessionState,
    getHostNetworkState: mocks.getHostNetworkState,
    getLabModeAvailable: mocks.getLabModeAvailable,
    getAppShutdownState: mocks.getAppShutdownState,
  };
});

const stylesheet = readFileSync("src/styles.css", "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

interface CssBlock {
  prelude: string;
  body: string;
}

/** Splits a CSS body into its direct child blocks (at-rules or style rules). */
function childBlocks(css: string): CssBlock[] {
  const blocks: CssBlock[] = [];
  let depth = 0;
  let preludeStart = 0;
  let bodyStart = 0;
  for (let index = 0; index < css.length; index += 1) {
    const character = css[index];
    if (character === "{") {
      if (depth === 0) bodyStart = index + 1;
      depth += 1;
    } else if (character === "}") {
      depth -= 1;
      if (depth === 0) {
        const prelude = css.slice(preludeStart, bodyStart - 1).trim();
        blocks.push({
          prelude: prelude.slice(prelude.lastIndexOf(";") + 1).trim(),
          body: css.slice(bodyStart, index),
        });
        preludeStart = index + 1;
      }
    } else if (character === ";" && depth === 0) {
      preludeStart = index + 1;
    }
  }
  return blocks;
}

function topLevelBlock(prelude: string): CssBlock {
  const block = childBlocks(stylesheet).find((candidate) => candidate.prelude === prelude);
  if (!block) throw new Error(`missing top-level block ${prelude}`);
  return block;
}

function declarations(body: string): Record<string, string> {
  return Object.fromEntries(
    body
      .split(";")
      .map((declaration) => declaration.split(":"))
      .filter((parts): parts is [string, string, ...string[]] => parts.length >= 2)
      .map(([property, ...value]) => [property.trim(), value.join(":").trim()]),
  );
}

function sourceFiles(directory: string): string[] {
  return readdirSync(directory).flatMap((entry) => {
    const path = join(directory, entry);
    if (statSync(path).isDirectory()) return entry === "generated" ? [] : sourceFiles(path);
    return /\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry) && !path.includes("/test/")
      ? [path]
      : [];
  });
}

const COMPACT_QUERY = "@media (min-width: 700px) and (max-height: 700px)";
const INTERACTIVE = "button, input, select, textarea, a[href], [tabindex]";

describe("reduced motion", () => {
  it("neutralizes animation, transition, and smooth scrolling at the top level", () => {
    const reduced = topLevelBlock("@media (prefers-reduced-motion: reduce)");
    const rules = childBlocks(reduced.body);
    expect(declarations(rules.find((rule) => rule.prelude === ":root")?.body ?? "")).toMatchObject({
      "scroll-behavior": "auto",
    });
    const universal = rules.find((rule) => rule.prelude.startsWith(":root *"));
    expect(universal?.prelude.replace(/\s+/g, " ")).toBe(
      ":root *, :root *::before, :root *::after",
    );
    expect(declarations(universal?.body ?? "")).toEqual({
      "animation-duration": "0.01ms",
      "animation-iteration-count": "1",
      "transition-duration": "0.01ms",
    });
  });

  it("keeps every other motion declaration layered so the unlayered override wins", () => {
    const motion = /\b(transition|animation)[\w-]*\s*:/;
    for (const block of childBlocks(stylesheet)) {
      if (block.prelude === "@media (prefers-reduced-motion: reduce)") continue;
      if (block.prelude.startsWith("@layer") || block.prelude.startsWith("@theme")) {
        expect(block.body).not.toMatch(/!important/);
        continue;
      }
      expect(block.body, `${block.prelude} must not declare unlayered motion`).not.toMatch(motion);
    }
  });

  it("has no script-driven motion that would bypass the CSS preference", () => {
    for (const file of sourceFiles("src")) {
      const source = readFileSync(file, "utf8");
      expect(source, file).not.toMatch(/behavior:\s*["']smooth["']/);
      expect(source, file).not.toMatch(/\.animate\(/);
      expect(source, file).not.toMatch(/requestAnimationFrame/);
    }
  });
});

describe("compact desktop window", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getHostNetworkState.mockResolvedValue(networkSnapshot());
    mocks.getLabModeAvailable.mockResolvedValue(false);
    mocks.getAppShutdownState.mockResolvedValue({ kind: "notRequested" });
  });

  async function renderLiveSession() {
    const harness = renderHostApp({
      initial: coreSnapshot({ hostLifecycle: "streaming" }),
      hostSession: hostSession({
        hostLifecycle: "streaming",
        transportState: "connected",
        playbackState: "playing",
        playbackPositionMs: "84000",
        pendingJoinRequests: [pendingRequest("jo", "Jo's Pixel")],
        connectedListeners: [listener("sam", "Sam's Phone")],
      }),
      ensureDesktopBridge: mocks.ensureDesktopBridge,
      subscribeDesktopNotifications: mocks.subscribeDesktopNotifications,
      getHostSessionState: mocks.getHostSessionState,
    });
    await screen.findByText("Sam's Phone");
    return harness.container;
  }

  it("targets only markup the live session actually renders", async () => {
    const container = await renderLiveSession();
    const compact = topLevelBlock(COMPACT_QUERY);
    const selectors = childBlocks(compact.body).flatMap((rule) => rule.prelude.split(","));
    expect(selectors.length).toBeGreaterThan(10);
    for (const selector of selectors) {
      expect(container.querySelector(selector.trim()), selector.trim()).not.toBeNull();
    }
  });

  it("hides only non-interactive hints and never a primary action", async () => {
    const container = await renderLiveSession();
    const hidden = childBlocks(topLevelBlock(COMPACT_QUERY).body).filter((rule) => {
      const { display } = declarations(rule.body);
      return display === "none";
    });
    expect(hidden.length).toBeGreaterThan(0);
    for (const rule of hidden) {
      for (const element of Array.from(container.querySelectorAll(rule.prelude))) {
        expect(element.matches(INTERACTIVE), rule.prelude).toBe(false);
        expect(element.querySelector(INTERACTIVE), rule.prelude).toBeNull();
      }
    }
  });

  it("keeps all live actions in the dashboard, un-clipped, with the page body scrolling", async () => {
    await renderLiveSession();
    const dashboard = screen.getByRole("region", { name: "Live playback and queue controls" });
    for (const name of ["Play", "Pause", "Stop", "Add songs", "Create invite"]) {
      expect(within(dashboard).getByRole("button", { name })).toBeInTheDocument();
    }
    expect(within(dashboard).getByText("1 listeners")).toBeInTheDocument();
    const rules = Object.fromEntries(
      childBlocks(topLevelBlock(COMPACT_QUERY).body).map((rule) => [
        rule.prelude,
        declarations(rule.body),
      ]),
    );
    // The dashboard is a two-column grid that never scrolls internally.
    expect(rules[".host-live-dashboard"]).toMatchObject({
      display: "grid",
      "max-height": "none",
      overflow: "visible",
      position: "static",
    });
    // The window itself never scrolls; only the destination body does.
    expect(rules[".desktop-app-shell"]).toMatchObject({ height: "100dvh", overflow: "hidden" });
    expect(rules[".desktop-app-panel-body"]).toMatchObject({ "overflow-y": "auto" });
  });
});
