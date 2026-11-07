# Antibody: the site

The public site of Antibody (Next.js App Router, TypeScript strict). It reads the API (`antibody-services api`,
[../docs/api.md](../docs/api.md)) on the server with short revalidation, follows the live event stream in the browser,
and talks to NEAR and the EVM chains directly from the browser for the checks and the practice try: keys, seals and
signatures are made in the visitor's tab.

## Run it

```bash
npm ci
node tests/mock-api.mjs &                              # the recorded testnet snapshots (tests/fixtures/api)
NEXT_PUBLIC_API_BASE=http://127.0.0.1:4010 npm run dev # http://localhost:3000
```

Against the live API, set `NEXT_PUBLIC_API_BASE` to its HTTPS URL. Every variable is in [.env.example](.env.example);
none is a secret (the practice vend key is public by design and is derived in the browser from its published seed).

## Check it

| Command | What it checks |
|---|---|
| `npm test` | The browser try's envelope, seal (ABSEAL1), commitment and signed NEAR transactions against `../golden/web-vectors.json`, which the Rust code writes (`crates/cli/tests/web_vectors.rs`); the try's state machine on a simulated chain; the wallet's transaction builders |
| `npx playwright test` | Every page in light, dark and mobile against a production build and the mock API: renders, the testnet notice, no horizontal scroll, no third-party request, the security headers |
| `MOCK_T2=1 node tests/mock-api.mjs` | The API as it will answer once near-t2 has records on testnet (tests/fixtures/api-t2, test data): T2 lights up with no code change |
| `node scripts/axe.mjs <url>` | WCAG 2.2 AA (axe-core), light and dark |
| `node scripts/overflow.mjs <url>` | What, if anything, makes a page wider than a 360 px phone |
| `node scripts/screenshots.mjs <url> <dir>` | Screenshots of every page, light, dark and mobile |
| `npm run sync-data` | Refreshes `data/moments.json`, the bundled copy of `/v1/moments` (the fallback and the OG image's source) |

`data/judges.json` is written by `../scripts/docs/judges.py` from the same results file as `../JUDGES.md`, so the
judges' page and JUDGES.md cannot drift.

## Layout

| Path | Contents |
|---|---|
| `app/(site)/` | The pages, under the layout with the testnet notice, the header and the footer |
| `app/(bare)/moment/present` | The one moment in presentation mode (16:9, no chrome) |
| `components/ui/` | The design system's parts: hashes, pills, tiers, the verdict stamp, the sealed block, live checks |
| `components/moment/` | The one-moment timeline (the signature element) |
| `lib/api/` | The API client and its types |
| `lib/practice/` | The browser try: sealing, the submission envelope, NEAR transactions, the flow |
| `lib/near/` | The browser's NEAR RPC client (public endpoints, failover, backoff) |
| `lib/wallet/` | The account page's NEAR wallet connection and the transactions it may ask for |
| `proxy.ts` | A strict Content-Security-Policy with a nonce per request |
