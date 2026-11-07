#!/usr/bin/env node
// Refreshes the bundled copy of /v1/moments (web/data/moments.json): the site's fallback when the API
// cannot be reached, and the source of the landing's OG image. Usage: node scripts/sync-data.mjs [api base]
import { writeFileSync } from "node:fs";

const base = (process.argv[2] ?? process.env.NEXT_PUBLIC_API_BASE ?? "").replace(/\/+$/, "");
if (!base) {
  console.error("usage: node scripts/sync-data.mjs <api base>   (or set NEXT_PUBLIC_API_BASE)");
  process.exit(2);
}
const res = await fetch(`${base}/v1/moments`);
if (!res.ok) throw new Error(`GET /v1/moments: HTTP ${res.status}`);
const body = await res.json();
if (!Array.isArray(body.moments) || body.moments.length === 0) throw new Error("no moments in the answer");
writeFileSync(new URL("../data/moments.json", import.meta.url), JSON.stringify(body, null, 1) + "\n");
console.log(`data/moments.json: ${body.moments.length} run(s), latest ${body.latest}`);
