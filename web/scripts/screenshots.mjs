#!/usr/bin/env node
// Screenshots of every page in light, dark and mobile, for review and the report.
// Usage: node scripts/screenshots.mjs <base url> <out dir> [path ...]
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "@playwright/test";

const [base = "http://localhost:3000", out = "screenshots", ...only] = process.argv.slice(2);
const PAGES = only.length
  ? only
  : ["/", "/moment", "/programs", "/programs/5", "/programs/6", "/programs/7", "/commits/13", "/commits/15", "/commits/16", "/commits/17", "/commits/19", "/runs/33", "/try", "/judges", "/trust", "/status", "/mainnet", "/agents", "/projects", "/pilot", "/account", "/moment/present"];
const MODES = [
  { name: "light", viewport: { width: 1440, height: 900 }, colorScheme: "light", isMobile: false },
  { name: "dark", viewport: { width: 1440, height: 900 }, colorScheme: "dark", isMobile: false },
  { name: "mobile", viewport: { width: 390, height: 844 }, colorScheme: "light", isMobile: true, deviceScaleFactor: 2 },
];

mkdirSync(out, { recursive: true });
const browser = await chromium.launch();
for (const mode of MODES) {
  const ctx = await browser.newContext({
    viewport: mode.viewport,
    colorScheme: mode.colorScheme,
    isMobile: mode.isMobile,
    deviceScaleFactor: mode.deviceScaleFactor ?? 1,
    reducedMotion: "reduce", // the final state of every animation
  });
  const page = await ctx.newPage();
  for (const p of PAGES) {
    if (p === "/moment/present" && mode.isMobile) continue;
    const vp = p === "/moment/present" ? { width: 1920, height: 1080 } : mode.viewport;
    await page.setViewportSize(vp);
    const name = (p === "/" ? "home" : p.slice(1).replace(/\//g, "_")) + `.${mode.name}.png`;
    try {
      await page.goto(base + p, { waitUntil: "networkidle", timeout: 60_000 });
    } catch {
      await page.goto(base + p, { waitUntil: "load", timeout: 60_000 }).catch(() => {});
    }
    await page.waitForTimeout(800);
    await page.screenshot({ path: join(out, name), fullPage: p !== "/moment/present" });
    console.log(name);
  }
  await ctx.close();
}
await browser.close();
