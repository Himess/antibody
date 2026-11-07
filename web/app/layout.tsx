import type { Metadata, Viewport } from "next";
import { Instrument_Sans, JetBrains_Mono } from "next/font/google";
import { headers } from "next/headers";
import { themeScript } from "@/components/site/ThemeToggle";
import { config } from "@/lib/config";
import "./globals.css";

// Self-hosted at build time by next/font: no request to a font service from the browser.
const sans = Instrument_Sans({ subsets: ["latin"], variable: "--font-instrument", display: "swap" });
const mono = JetBrains_Mono({ subsets: ["latin"], variable: "--font-jetbrains", display: "swap" });

const description =
  "Bug bounties that pay on proof. Hunters submit a sealed exploit instead of a report; Antibody re-runs it in secure hardware, NEAR pays the bounty automatically if it breaks the project's rules, and a critical bug pauses the protocol on every chain.";

export const metadata: Metadata = {
  metadataBase: new URL(config.siteUrl),
  title: { default: "Antibody: bug bounties that pay on proof", template: "%s · Antibody" },
  description,
  applicationName: "Antibody",
  openGraph: {
    type: "website",
    siteName: "Antibody",
    title: "Antibody: we don't read reports. We run them.",
    description,
  },
  twitter: { card: "summary_large_image", title: "Antibody: we don't read reports. We run them.", description },
  robots: { index: true, follow: true },
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#f4f5f2" },
    { media: "(prefers-color-scheme: dark)", color: "#0a0b0c" },
  ],
};

export default async function RootLayout({ children }: { children: React.ReactNode }) {
  const nonce = (await headers()).get("x-nonce") ?? undefined;
  return (
    <html lang="en" className={`${sans.variable} ${mono.variable}`} suppressHydrationWarning>
      <head>
        <script nonce={nonce} dangerouslySetInnerHTML={{ __html: themeScript }} />
      </head>
      <body>
        <a href="#main" className="skip-link">
          Skip to content
        </a>
        {children}
      </body>
    </html>
  );
}
