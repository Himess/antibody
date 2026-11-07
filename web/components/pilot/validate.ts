/**
 * The pilot form's fields and the checks the API applies to them (crates/services/src/web.rs,
 * `validate_interest`), mirrored so a person sees a problem before anything is sent. The API stays the
 * authority: it trims every text field, counts characters as Unicode scalar values and refuses control
 * characters (line breaks and tabs only in the notes). No imports: the page and the form both use it.
 */

export const ROLES = [
  { value: "project", label: "A project that wants a pilot" },
  { value: "hunter", label: "A hunter" },
  { value: "agent", label: "An AI agent's operator" },
  { value: "other", label: "Something else" },
] as const;

export type Role = (typeof ROLES)[number]["value"];

export const isRole = (v: unknown): v is Role => typeof v === "string" && ROLES.some((r) => r.value === v);

export type TextField = "name" | "contact" | "project" | "chains" | "timeline" | "notes";
export type Field = "role" | TextField | "consent";

export const LIMITS: Record<TextField, { min: number; max: number; multiline: boolean }> = {
  name: { min: 1, max: 100, multiline: false },
  contact: { min: 3, max: 200, multiline: false },
  project: { min: 0, max: 200, multiline: false },
  chains: { min: 0, max: 200, multiline: false },
  timeline: { min: 0, max: 100, multiline: false },
  notes: { min: 0, max: 2000, multiline: true },
};

export const FIELD_NAMES: Record<Field, string> = {
  role: "Who you are",
  name: "Name",
  contact: "Contact",
  project: "Project or site",
  chains: "Chains",
  timeline: "Timeline",
  notes: "Notes",
  consent: "Consent",
};

/** The order fields appear in, for the error summary and for where focus goes first. */
export const FIELD_ORDER: Field[] = ["role", "name", "contact", "project", "chains", "timeline", "notes", "consent"];

export type Values = Record<TextField, string> & { role: Role | ""; consent: boolean; website: string };

export const emptyValues = (role: Role | "" = ""): Values => ({
  role,
  name: "",
  contact: "",
  project: "",
  chains: "",
  timeline: "",
  notes: "",
  consent: false,
  website: "",
});

export type Errors = Partial<Record<Field, string>>;

/** Characters as the API counts them: Unicode scalar values (Rust's `chars()`), not UTF-16 units. */
export const chars = (s: string): number => Array.from(s).length;

/** Rust's `char::is_control`: the Cc category, U+0000 to U+001F and U+007F to U+009F. */
function hasControl(s: string, multiline: boolean): boolean {
  for (const ch of s) {
    const cp = ch.codePointAt(0) ?? 0;
    const control = cp <= 0x1f || (cp >= 0x7f && cp <= 0x9f);
    if (control && !(multiline && (cp === 0x0a || cp === 0x0d || cp === 0x09))) return true;
  }
  return false;
}

const CONSENT_MESSAGE = "Tick the box so we may keep these details to reply to you.";
const ROLE_MESSAGE = "Choose who you are.";

export function validate(v: Values): Errors {
  const e: Errors = {};
  if (!isRole(v.role)) e.role = ROLE_MESSAGE;
  for (const k of Object.keys(LIMITS) as TextField[]) {
    const { min, max, multiline } = LIMITS[k];
    const s = (v[k] ?? "").trim();
    const n = chars(s);
    if (n === 0 && min > 0) e[k] = `${FIELD_NAMES[k]} is required.`;
    else if (n < min) e[k] = `${FIELD_NAMES[k]}: at least ${min} characters.`;
    else if (n > max) e[k] = `${FIELD_NAMES[k]}: at most ${max} characters.`;
    else if (hasControl(s, multiline)) e[k] = multiline ? `${FIELD_NAMES[k]}: no control characters.` : `${FIELD_NAMES[k]}: one line, with no control characters.`;
  }
  if (v.consent !== true) e.consent = CONSENT_MESSAGE;
  return e;
}

/** The JSON the API takes: every field it reads, trimmed as it trims them, and the honeypot as typed. */
export function payload(v: Values) {
  return {
    role: v.role,
    name: v.name.trim(),
    contact: v.contact.trim(),
    project: v.project.trim(),
    chains: v.chains.trim(),
    timeline: v.timeline.trim(),
    notes: v.notes.trim(),
    consent: v.consent === true,
    website: v.website,
  };
}

/** A 422 answer names its field first ("contact: 3 to 200 characters"): the message for that field. */
export function apiFieldError(error: string): { field: Field; message: string } | null {
  const m = /^([a-z]+):\s*([\s\S]*)$/.exec(error.trim());
  if (!m) return null;
  const field = FIELD_ORDER.find((f) => f === m[1]);
  if (!field) return null;
  if (field === "consent") return { field, message: CONSENT_MESSAGE };
  if (field === "role") return { field, message: ROLE_MESSAGE };
  const rest = m[2].replace(/\.$/, "");
  return { field, message: `${FIELD_NAMES[field]}: ${rest || "refused"}.` };
}

export type Outcome =
  | { kind: "sent"; id: string | null }
  | { kind: "field"; field: Field; message: string }
  | { kind: "failed"; title: string; detail: string };

/**
 * What an answer from POST /v1/interest means for the person: stored (with its reference), a field to
 * fix, or a failure said plainly with what to do next.
 */
export function outcomeOf(status: number, body: unknown): Outcome {
  const b = (typeof body === "object" && body !== null ? body : {}) as { ok?: unknown; id?: unknown; error?: unknown };
  const error = typeof b.error === "string" ? b.error : "";
  if (status >= 200 && status < 300 && b.ok === true) return { kind: "sent", id: typeof b.id === "string" && b.id ? b.id : null };
  if (status === 422) {
    const f = apiFieldError(error);
    if (f) return { kind: "field", ...f };
    return { kind: "failed", title: "The API refused the request.", detail: error ? `It answered: ${error}.` : "Check your answers and send it again." };
  }
  if (status === 429) {
    return /second/i.test(error)
      ? { kind: "failed", title: "The API is busy.", detail: "Try again in a few seconds. Your answers are still here." }
      : { kind: "failed", title: "Too many requests from your network.", detail: "Try again in an hour. Your answers are still here." };
  }
  if (status === 403) {
    return { kind: "failed", title: "This form only accepts requests from this site.", detail: "If you opened a copy of this page somewhere else, send it from the site itself." };
  }
  if (status === 413) return { kind: "failed", title: "The request is too large.", detail: "The API takes at most 8 KB: shorten the notes and send it again." };
  if (status >= 500) {
    return { kind: "failed", title: "Not stored.", detail: `${error ? `The API answered: ${error}. ` : ""}Try again later; your answers are still here.` };
  }
  if (status >= 200 && status < 300) {
    return { kind: "failed", title: "The API did not confirm it stored the request.", detail: "Send it again in a minute; your answers are still here." };
  }
  return { kind: "failed", title: `The API answered HTTP ${status}.`, detail: error ? `${error}. Try again in a minute.` : "Try again in a minute; your answers are still here." };
}
