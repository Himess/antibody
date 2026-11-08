import { NextResponse, type NextRequest } from "next/server";
import { config as site } from "@/lib/config";
import { WALLET_CONNECT_SRC } from "@/lib/wallet/csp";

/**
 * A strict Content-Security-Policy with a fresh nonce per request (Next.js applies it to its own scripts).
 * The browser talks to our API and the NEAR and EVM RPCs it uses for checks and the try; on /account, when a
 * visitor picks a NEAR wallet, to that wallet's pinned connector (below). No trackers, no third-party fonts.
 */
const origin = (u: string) => {
  try {
    return new URL(u).origin;
  } catch {
    return "";
  }
};

const connect = Array.from(
  new Set(
    [
      site.apiBase,
      ...site.nearRpcs,
      ...site.nearArchivalRpcs,
      ...Object.values(site.evmRpcs).flat(),
      ...(process.env.NEXT_PUBLIC_CSP_CONNECT_EXTRA ?? "").split(","),
    ]
      .map((u) => origin(u.trim()))
      .filter(Boolean),
  ),
);

// The account page's NEAR wallet connection (lib/wallet/csp.ts): each pinned wallet's connector script, by
// its exact path, and what those scripts reach from their sandboxed iframes (srcdoc documents inherit this
// policy). In every page's policy because a client-side navigation to /account keeps the policy of the page
// first loaded. No script-src, frame-src or img-src change: the scripts run inline in the iframe with this
// request's nonce, a srcdoc iframe loads no URL, and wallet icons are inline.
connect.push(...WALLET_CONNECT_SRC.filter((s) => !connect.includes(s)));

export function proxy(request: NextRequest) {
  const nonce = btoa(crypto.randomUUID());
  const dev = process.env.NODE_ENV === "development";
  const csp = [
    "default-src 'self'",
    `script-src 'self' 'nonce-${nonce}' 'strict-dynamic'${dev ? " 'unsafe-eval'" : ""}`,
    // React and next/font set a few inline style attributes; styles cannot run code.
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob:",
    "font-src 'self'",
    `connect-src 'self' ${connect.join(" ")}${dev ? " ws: http://127.0.0.1:* http://localhost:*" : ""}`,
    "frame-src 'none'",
    "worker-src 'self' blob:",
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'self'",
    "frame-ancestors 'none'",
    "upgrade-insecure-requests",
  ].join("; ");

  const requestHeaders = new Headers(request.headers);
  requestHeaders.set("x-nonce", nonce);
  requestHeaders.set("Content-Security-Policy", csp);
  const response = NextResponse.next({ request: { headers: requestHeaders } });
  response.headers.set("Content-Security-Policy", csp);
  return response;
}

export const config = {
  matcher: [
    {
      source: "/((?!_next/static|_next/image|favicon.ico|icon.svg|apple-icon.png|robots.txt|sitemap.xml|fonts/).*)",
      missing: [
        { type: "header", key: "next-router-prefetch" },
        { type: "header", key: "purpose", value: "prefetch" },
      ],
    },
  ],
};
