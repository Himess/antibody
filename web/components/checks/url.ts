/** An external link only when it is one (http or https), never a script or a relative path. */
export function safeUrl(u: string | null | undefined): string | undefined {
  return typeof u === "string" && /^https?:\/\//.test(u) ? u : undefined;
}
