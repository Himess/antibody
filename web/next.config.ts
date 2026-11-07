import type { NextConfig } from "next";

/**
 * Static security headers for every response. The Content-Security-Policy carries a fresh nonce per
 * request, so it is set in `proxy.ts`.
 */
const securityHeaders = [
  { key: "Strict-Transport-Security", value: "max-age=63072000; includeSubDomains; preload" },
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  { key: "X-Frame-Options", value: "DENY" },
  { key: "Cross-Origin-Opener-Policy", value: "same-origin-allow-popups" },
  {
    key: "Permissions-Policy",
    value: "camera=(), microphone=(), geolocation=(), payment=(), usb=(), browsing-topics=()",
  },
];

const nextConfig: NextConfig = {
  // A nonce-based CSP needs every page rendered per request (docs: "Content Security Policy"), which
  // rules out the static shell Cache Components prerenders. Data stays cached in the Data Cache.
  cacheComponents: false,
  partialPrefetching: false,
  poweredByHeader: false,
  reactStrictMode: true,
  // web/ is the app's root (the repository's root holds the Rust workspace).
  turbopack: { root: process.cwd() },
  async headers() {
    return [{ source: "/:path*", headers: securityHeaders }];
  },
};

export default nextConfig;
