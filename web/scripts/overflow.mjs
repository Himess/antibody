#!/usr/bin/env node
// Finds what makes a page wider than a phone: every element whose right edge passes the viewport.
// Usage: node scripts/overflow.mjs <base url> [path ...]
import { chromium } from "@playwright/test";

const [base = "http://localhost:3000", ...paths] = process.argv.slice(2);
const PAGES = paths.length ? paths : ["/", "/moment", "/programs", "/programs/5", "/programs/6", "/programs/7", "/commits/13", "/commits/16", "/runs/33", "/try", "/judges", "/trust", "/status", "/mainnet", "/agents", "/projects", "/pilot", "/account"];
const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 360, height: 800 }, isMobile: true, reducedMotion: "reduce" });
const page = await ctx.newPage();
for (const p of PAGES) {
  await page.goto(base + p, { waitUntil: "networkidle", timeout: 60_000 }).catch(() => {});
  const r = await page.evaluate(() => {
    const vw = document.documentElement.clientWidth;
    const over = [];
    for (const el of document.querySelectorAll("body *")) {
      const b = el.getBoundingClientRect();
      if (b.width === 0 || b.right <= vw + 1) continue;
      // Skip what scrolls inside a scroll container.
      let s = el.parentElement;
      let clipped = false;
      while (s && s !== document.body) {
        const cs = getComputedStyle(s);
        if (/(auto|scroll|hidden|clip)/.test(cs.overflowX)) {
          clipped = true;
          break;
        }
        s = s.parentElement;
      }
      if (clipped) continue;
      over.push({ tag: el.tagName.toLowerCase(), cls: String(el.className).slice(0, 80), right: Math.round(b.right), text: (el.textContent || "").trim().slice(0, 60) });
    }
    return { vw, scroll: document.documentElement.scrollWidth, over: over.slice(0, 8) };
  });
  console.log(`${p}: viewport ${r.vw}, scroll width ${r.scroll}`);
  for (const o of r.over) console.log(`   ${o.tag}.${o.cls} right=${o.right} "${o.text}"`);
}
await browser.close();
