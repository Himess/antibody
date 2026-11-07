import type { MetadataRoute } from "next";
import { config } from "@/lib/config";

const PAGES = ["", "/moment", "/programs", "/programs/5", "/programs/6", "/programs/7", "/try", "/judges", "/trust", "/agents", "/projects", "/pilot", "/status", "/mainnet", "/account"];

export default function sitemap(): MetadataRoute.Sitemap {
  return PAGES.map((p) => ({ url: `${config.siteUrl}${p}`, changeFrequency: "daily", priority: p === "" ? 1 : 0.7 }));
}
