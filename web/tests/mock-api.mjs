#!/usr/bin/env node
// A local stand-in for the Antibody API, serving the recorded testnet snapshots in tests/fixtures/api/ (CI and
// Playwright run the site against it, so a test never depends on testnet or spends anything).
// Usage: node tests/mock-api.mjs [port]
import { createServer } from "node:http";
import { existsSync, readFileSync } from "node:fs";

const port = Number(process.argv[2] ?? process.env.MOCK_API_PORT ?? 4010);
const dir = new URL("./fixtures/api/", import.meta.url);
// MOCK_T2=1: the scenario where near-t2 has records (fixtures/api-t2 overrides; test data only).
const t2 = process.env.MOCK_T2 === "1";
const t2dir = new URL("./fixtures/api-t2/", import.meta.url);
const load = (name) => {
  const over = new URL(`${name}.json`, t2dir);
  if (t2 && existsSync(over)) return JSON.parse(readFileSync(over, "utf8"));
  const u = new URL(`${name}.json`, dir);
  return existsSync(u) ? JSON.parse(readFileSync(u, "utf8")) : null;
};

function route(path) {
  const m = (re) => path.match(re);
  let x;
  if (path === "/health") return load("health");
  if (path === "/v1/status") return load("status");
  if (path === "/v1/stats") return load("stats");
  if (path === "/v1/programs") return load("programs");
  if ((x = m(/^\/v1\/programs\/(\d+)$/))) return load(`program-${x[1]}`);
  if ((x = m(/^\/v1\/programs\/(\d+)\/feed$/))) {
    const f = load(`feed-${x[1]}`);
    if (f) return f;
    const p = load(`program-${x[1]}`);
    return p ? { program_id: Number(x[1]), submissions: p.commits } : null;
  }
  if ((x = m(/^\/v1\/programs\/(\d+)\/reflex$/))) return load(`reflex-${x[1]}`) ?? { program_id: Number(x[1]), guardian_address: null, chains: [] };
  if ((x = m(/^\/v1\/commits\/(\d+)$/))) return load(`commit-${x[1]}`);
  if ((x = m(/^\/v1\/runs\/(\d+)$/))) return load(`run-${x[1]}`);
  if ((x = m(/^\/v1\/accounts\/([a-z0-9._-]+)$/))) {
    const a = x[1] === "practice.antibody.testnet" ? load("account-practice") : load("account-agent-a");
    return a ? { ...a, account: x[1] } : null;
  }
  if (path === "/v1/leaderboard") return load("leaderboard");
  if (path === "/v1/events") return load("events");
  if (path === "/v1/guardian") return load("guardian");
  if (path === "/v1/judge") return load("judge");
  if (path === "/v1/trust") return load("trust");
  if (path === "/v1/practice") return load("practice");
  if (path === "/v1/mainnet") return load("mainnet");
  if (path === "/v1/moments") return load("moments");
  if (path === "/v1/t2") return t2 ? load("t2") : { __status: 502, error: "this deployment names no near-t2 contract yet" };
  return undefined;
}

createServer((req, res) => {
  const url = new URL(req.url ?? "/", "http://localhost");
  const headers = { "access-control-allow-origin": "*", "content-type": "application/json" };
  if (url.pathname === "/v1/stream") {
    res.writeHead(200, { ...headers, "content-type": "text/event-stream", "cache-control": "no-cache" });
    res.write(": mock stream\n\n");
    const t = setInterval(() => res.write(": keep-alive\n\n"), 15000);
    req.on("close", () => clearInterval(t));
    return;
  }
  if (req.method === "OPTIONS") {
    res.writeHead(204, { ...headers, "access-control-allow-methods": "POST, OPTIONS", "access-control-allow-headers": "content-type" });
    return res.end();
  }
  if (req.method === "POST" && url.pathname === "/v1/interest") {
    res.writeHead(200, headers);
    return res.end(JSON.stringify({ ok: true, id: "mock00000000" }));
  }
  const body = route(url.pathname);
  if (body === undefined || body === null) {
    // The real API's answer for an id near-antibody does not know (the site turns it into a 404).
    const id = url.pathname.match(/^\/v1\/(commits|runs|programs)\/(\d+)/);
    const what = id ? { commits: "commit", runs: "run", programs: "program" }[id[1]] : null;
    res.writeHead(502, headers);
    return res.end(JSON.stringify({ error: what ? `${what} ${id[2]} does not exist on bounty.antibody.testnet` : `not in the fixtures: ${url.pathname}` }));
  }
  const status = body.__status ?? 200;
  delete body.__status;
  res.writeHead(status, headers);
  res.end(JSON.stringify(body));
}).listen(port, "127.0.0.1", () => console.log(`mock API on http://127.0.0.1:${port}${t2 ? " (near-t2 with records)" : ""}`));
