import { expect, test } from "@playwright/test";

/** Every page renders its title, the testnet notice, and no error; no third-party request leaves the page. */
const PAGES: { path: string; h1: RegExp }[] = [
  { path: "/", h1: /We don.t read reports/i },
  { path: "/moment", h1: /sealed proof/i },
  { path: "/programs", h1: /program/i },
  { path: "/programs/5", h1: /Acme Lend/i },
  { path: "/programs/6", h1: /Practice/i },
  { path: "/commits/13", h1: /13/ },
  { path: "/commits/16", h1: /16/ },
  { path: "/runs/33", h1: /33/ },
  { path: "/try", h1: /./ },
  { path: "/judges", h1: /./ },
  { path: "/trust", h1: /./ },
  { path: "/status", h1: /./ },
  { path: "/mainnet", h1: /mainnet/i },
  { path: "/agents", h1: /./ },
  { path: "/projects", h1: /./ },
  { path: "/pilot", h1: /./ },
  { path: "/account", h1: /./ },
];

for (const p of PAGES) {
  test(`page ${p.path}`, async ({ page, baseURL }) => {
    const errors: string[] = [];
    const foreign: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    page.on("console", (m) => {
      if (m.type() === "error" && !/Failed to load resource|EventSource|ERR_CONNECTION/i.test(m.text())) errors.push(m.text());
    });
    const site = new URL(baseURL ?? "http://127.0.0.1").host;
    page.on("request", (r) => {
      const u = new URL(r.url());
      if (u.protocol.startsWith("http") && u.host !== site && !/^(127\.0\.0\.1|localhost)/.test(u.host) && !/sslip\.io|fastnear|near\.org|publicnode/.test(u.host))
        foreign.push(u.host);
    });
    const res = await page.goto(p.path, { waitUntil: "domcontentloaded" });
    expect(res?.status(), "HTTP status").toBeLessThan(400);
    await expect(page.locator("h1").first()).toHaveText(p.h1);
    await expect(page.getByRole("note", { name: "Testnet notice" })).toBeVisible();
    // No horizontal page scroll (mobile-first).
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow, "horizontal overflow (px)").toBeLessThanOrEqual(1);
    expect(errors, "page errors").toEqual([]);
    expect(foreign, "third-party requests").toEqual([]);
  });
}

test("security headers", async ({ request }) => {
  const res = await request.get("/");
  const csp = res.headers()["content-security-policy"] ?? "";
  expect(csp).toContain("script-src 'self' 'nonce-");
  expect(csp).toContain("frame-ancestors 'none'");
  expect(res.headers()["strict-transport-security"]).toContain("max-age=");
  expect(res.headers()["x-content-type-options"]).toBe("nosniff");
});

test("the one moment links every step to its transaction", async ({ page }) => {
  await page.goto("/moment");
  const links = page.locator('a[href*="nearblocks.io/txns/"], a[href*="etherscan.io/tx/"], a[href*="basescan.org/tx/"]');
  expect(await links.count()).toBeGreaterThanOrEqual(7);
});

test("presentation mode is chromeless", async ({ page }) => {
  await page.goto("/moment/present");
  await expect(page.getByRole("note", { name: "Testnet notice" })).toHaveCount(0);
  await expect(page.locator("h1")).toContainText(/paused/i);
});
