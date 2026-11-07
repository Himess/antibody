#!/usr/bin/env node
// Accessibility scan (axe-core, WCAG 2.2 A/AA) of every page, light and dark.
// Usage: node scripts/axe.mjs <base url> [path ...]
import { chromium } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

const [base = "http://localhost:3000", ...paths] = process.argv.slice(2);
const PAGES = paths.length ? paths : ["/", "/moment", "/programs", "/programs/5", "/programs/6", "/programs/7", "/commits/13", "/commits/16", "/commits/19", "/runs/33", "/try", "/judges", "/trust", "/status", "/mainnet", "/agents", "/projects", "/pilot", "/account", "/moment/present"];
const browser = await chromium.launch();
let total = 0;
for (const scheme of ["light", "dark"]) {
  const ctx = await browser.newContext({ colorScheme: scheme, reducedMotion: "reduce", viewport: { width: 1366, height: 900 } });
  const page = await ctx.newPage();
  for (const p of PAGES) {
    await page.goto(base + p, { waitUntil: "networkidle", timeout: 60_000 }).catch(() => {});
    const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
    for (const v of r.violations) {
      total += v.nodes.length;
      console.log(`[${scheme}] ${p} ${v.id} (${v.impact}) x${v.nodes.length}: ${v.help}`);
      for (const n of v.nodes.slice(0, 3)) console.log(`      ${n.target.join(" ")} :: ${(n.failureSummary || "").split("\n").slice(1, 2).join(" ").slice(0, 160)}`);
    }
  }
  await ctx.close();
}
await browser.close();
console.log(`${total} violation node(s)`);
process.exit(total ? 1 : 0);
