"use client";

import Link from "next/link";
import { useEffect, useRef, useState, useSyncExternalStore, type ChangeEvent, type FormEvent, type ReactNode } from "react";
import { CopyButton } from "@/components/ui/CopyButton";
import { Icon } from "@/components/ui/Icon";
import { config } from "@/lib/config";
import {
  FIELD_ORDER,
  LIMITS,
  ROLES,
  chars,
  emptyValues,
  outcomeOf,
  payload,
  validate,
  type Errors,
  type Field,
  type Role,
  type TextField,
  type Values,
} from "./validate";
import styles from "./pilot.module.css";

type Phase = { kind: "editing" } | { kind: "sending" } | { kind: "sent"; id: string | null } | { kind: "failed"; title: string; detail: string; network: boolean };

const ENDPOINT = `${config.apiBase}/v1/interest`;

/** Where focus goes for a field: its control (for the role, the chosen radio or the first). */
const targetId = (f: Field, role: Role | "") => (f === "role" ? `pilot-role-${role || ROLES[0].value}` : `pilot-${f}`);

// Hydrated or not, without setting state in an effect: false on the server and while hydrating, then true.
const noSubscribe = () => () => {};

/**
 * The pilot and early-access form. It checks every field as the API does, then posts JSON straight to
 * the API's POST /v1/interest from the browser: no third-party service in between. Errors are tied to
 * their fields (aria-invalid, aria-describedby) and listed in a summary that a screen reader hears.
 */
export function PilotForm({ defaultRole }: { defaultRole?: Role }) {
  const hydrated = useSyncExternalStore(noSubscribe, () => true, () => false);
  const [values, setValues] = useState<Values>(() => emptyValues(defaultRole ?? ""));
  const [errors, setErrors] = useState<Errors>({});
  const [attempted, setAttempted] = useState(false);
  const [phase, setPhase] = useState<Phase>({ kind: "editing" });
  const focusNext = useRef<string | null>(null);

  // Move focus once the DOM shows what it should land on (an invalid field, or the confirmation).
  useEffect(() => {
    const id = focusNext.current;
    if (!id) return;
    focusNext.current = null;
    const el = document.getElementById(id);
    if (el) {
      el.focus();
      el.scrollIntoView({ block: "center", behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
    }
  });

  const sending = phase.kind === "sending";

  const update = <K extends keyof Values>(key: K, value: Values[K]) => {
    const next = { ...values, [key]: value };
    setValues(next);
    // After a first attempt, errors follow the answers as they are fixed.
    if (attempted) setErrors(validate(next));
    if (phase.kind === "failed") setPhase({ kind: "editing" });
  };

  async function onSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (sending) return;
    const found = validate(values);
    setAttempted(true);
    setErrors(found);
    const first = FIELD_ORDER.find((f) => found[f]);
    if (first) {
      setPhase({ kind: "editing" });
      focusNext.current = targetId(first, values.role);
      return;
    }
    setPhase({ kind: "sending" });
    let res: Response;
    try {
      res = await fetch(ENDPOINT, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload(values)),
        cache: "no-store",
        credentials: "omit",
        signal: AbortSignal.timeout(20_000),
      });
    } catch (err) {
      const timedOut = err instanceof DOMException && (err.name === "TimeoutError" || err.name === "AbortError");
      setPhase({
        kind: "failed",
        network: true,
        title: timedOut ? "The API did not answer within 20 seconds." : "Could not reach the API.",
        detail: "Check your connection and try again in a minute; your answers are still here. Nothing was stored.",
      });
      return;
    }
    let body: unknown = null;
    try {
      body = await res.json();
    } catch {
      body = null;
    }
    const out = outcomeOf(res.status, body);
    if (out.kind === "sent") {
      // Keep nothing personal in the page once it is stored.
      setValues(emptyValues(values.role));
      setErrors({});
      setAttempted(false);
      setPhase({ kind: "sent", id: out.id });
      focusNext.current = "pilot-sent";
      return;
    }
    if (out.kind === "field") {
      setErrors({ [out.field]: out.message });
      setPhase({ kind: "editing" });
      focusNext.current = targetId(out.field, values.role);
      return;
    }
    setPhase({ kind: "failed", network: false, title: out.title, detail: out.detail });
  }

  if (phase.kind === "sent") {
    return (
      <Sent
        id={phase.id}
        onAgain={() => {
          setPhase({ kind: "editing" });
          focusNext.current = targetId("role", values.role);
        }}
      />
    );
  }

  const listed = FIELD_ORDER.filter((f) => errors[f]);
  // The hints follow the role: a project's contracts and launch, or the person's own work.
  const forProject = values.role === "project";
  const text = (field: TextField) => ({
    value: values[field],
    error: errors[field],
    onChange: (e: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => update(field, e.target.value),
  });

  return (
    <form className={styles.form} onSubmit={onSubmit} noValidate method="post" aria-labelledby="pilot-form-title" aria-busy={sending}>
      <div className={styles.formHead}>
        <h2 id="pilot-form-title">Your request</h2>
        <p className="small muted">Every field is required unless it says optional.</p>
      </div>

      <div aria-live="polite">
        {attempted && listed.length ? (
          <div className={styles.summary} role="group" aria-labelledby="pilot-summary-title">
            <p id="pilot-summary-title" className={styles.summaryTitle}>
              <Icon name="alert" size={16} />
              {listed.length === 1 ? "One answer needs a change" : `${listed.length} answers need a change`}
            </p>
            <ul>
              {listed.map((f) => (
                <li key={f}>
                  <a
                    href={`#${targetId(f, values.role)}`}
                    onClick={(ev) => {
                      ev.preventDefault();
                      document.getElementById(targetId(f, values.role))?.focus();
                    }}
                  >
                    {errors[f]}
                  </a>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </div>

      <fieldset
        className={styles.fieldset}
        role="radiogroup"
        aria-labelledby="pilot-role-legend"
        aria-describedby={errors.role ? "pilot-role-error" : undefined}
        aria-invalid={errors.role ? true : undefined}
        aria-required="true"
      >
        <legend id="pilot-role-legend" className={styles.legend}>
          Who are you?
        </legend>
        <div className={styles.roles}>
          {ROLES.map((r) => (
            <label key={r.value} className={styles.role}>
              <input
                type="radio"
                id={`pilot-role-${r.value}`}
                name="role"
                value={r.value}
                checked={values.role === r.value}
                onChange={() => update("role", r.value)}
                required
              />
              <span>{r.label}</span>
            </label>
          ))}
        </div>
        {errors.role ? <ErrorText id="pilot-role-error">{errors.role}</ErrorText> : null}
      </fieldset>

      <div className={styles.pair}>
        <TextInput field="name" label="Name" hint="Your name or a handle." autoComplete="name" {...text("name")} />
        <TextInput field="contact" label="Contact" hint="Email, Telegram or X: where we reply." autoComplete="email" spellCheck={false} {...text("contact")} />
      </div>
      <div className={styles.pair}>
        <TextInput field="project" label="Project or site" hint="A name or a link." optional autoComplete="organization" {...text("project")} />
        <TextInput
          field="chains"
          label="Chains"
          hint={forProject ? "The chains your contracts run on." : "The chains you work on."}
          optional
          autoComplete="off"
          {...text("chains")}
        />
      </div>
      <TextInput
        field="timeline"
        label="Timeline"
        hint={forProject ? "For example, a launch or an upgrade date." : "When you would start."}
        optional
        autoComplete="off"
        {...text("timeline")}
      />
      <TextInput
        field="notes"
        label="Notes"
        hint="Anything that helps us reply: what you want to try, what you would need, questions."
        optional
        multiline
        {...text("notes")}
      />

      <div className={styles.consentBox} data-invalid={errors.consent ? "true" : undefined}>
        <label className={styles.consent} htmlFor="pilot-consent">
          <input
            type="checkbox"
            id="pilot-consent"
            name="consent"
            checked={values.consent}
            onChange={(e) => update("consent", e.target.checked)}
            aria-invalid={errors.consent ? true : undefined}
            aria-describedby={errors.consent ? "pilot-consent-error" : undefined}
            required
          />
          <span>Keep these details to reply to me about a pilot or early access. They are stored on our server only, never published.</span>
        </label>
        {errors.consent ? <ErrorText id="pilot-consent-error">{errors.consent}</ErrorText> : null}
      </div>

      {/* A field people never see: a bot that fills it is answered as accepted and kept nowhere. */}
      <div className="visually-hidden" aria-hidden="true">
        <label htmlFor="pilot-website">Website</label>
        <input type="text" id="pilot-website" name="website" tabIndex={-1} autoComplete="off" value={values.website} onChange={(e) => update("website", e.target.value)} />
      </div>

      {phase.kind === "failed" ? (
        <div className={`callout callout-warn ${styles.failure}`} role="alert">
          <Icon name="alert" size={16} />
          <div className="stack" style={{ ["--gap" as string]: "4px" }}>
            <p>{phase.title}</p>
            <p>{phase.detail}</p>
            {phase.network ? (
              <p className="small">
                If it keeps failing, the <Link href="/status">status page</Link> shows whether the API is up.
              </p>
            ) : null}
          </div>
        </div>
      ) : null}

      <div className={styles.actions}>
        <button type="submit" className="btn btn-primary" disabled={!hydrated} aria-disabled={sending || undefined}>
          {sending ? (
            <>
              <Icon name="refresh" size={14} /> Sending…
            </>
          ) : (
            <>
              Send request <Icon name="arrow" size={14} />
            </>
          )}
        </button>
        <span className={styles.endpoint}>
          Sent from your browser straight to the Antibody API (<span className="mono">POST /v1/interest</span>)
        </span>
      </div>
      <p className="visually-hidden" aria-live="polite">
        {sending ? "Sending your request." : ""}
      </p>
      <noscript>
        <p className="small">This form needs JavaScript: your browser sends the request straight to our API.</p>
      </noscript>
    </form>
  );
}

function ErrorText({ id, children }: { id: string; children: ReactNode }) {
  return (
    <p id={id} className={styles.error}>
      <Icon name="alert" size={14} />
      <span>{children}</span>
    </p>
  );
}

function TextInput({
  field,
  label,
  hint,
  optional,
  multiline,
  value,
  error,
  onChange,
  autoComplete,
  spellCheck,
}: {
  field: TextField;
  label: string;
  hint?: string;
  optional?: boolean;
  multiline?: boolean;
  value: string;
  error?: string;
  onChange: (e: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => void;
  autoComplete?: string;
  spellCheck?: boolean;
}) {
  const id = `pilot-${field}`;
  const max = LIMITS[field].max;
  const n = chars(value.trim());
  const describedBy = [hint ? `${id}-hint` : null, multiline ? `${id}-count` : null, error ? `${id}-error` : null].filter(Boolean).join(" ") || undefined;
  const common = {
    id,
    name: field,
    value,
    onChange,
    required: !optional,
    "aria-invalid": error ? true : undefined,
    "aria-describedby": describedBy,
  };
  return (
    <div className={styles.field}>
      <label htmlFor={id} className="label">
        {label}
        {optional ? <span className={styles.optional}> (optional)</span> : null}
      </label>
      {hint ? (
        <p id={`${id}-hint`} className={styles.hint}>
          {hint}
        </p>
      ) : null}
      {multiline ? (
        <textarea className={`textarea ${styles.control}`} rows={6} {...common} />
      ) : (
        <input className={`input ${styles.control}`} type="text" autoComplete={autoComplete} spellCheck={spellCheck} {...common} />
      )}
      {multiline ? (
        <p id={`${id}-count`} className={`${styles.count} ${n > max ? styles.countOver : ""}`}>
          <span className="num">{n}</span> of {max} characters
        </p>
      ) : null}
      {error ? <ErrorText id={`${id}-error`}>{error}</ErrorText> : null}
    </div>
  );
}

function Sent({ id, onAgain }: { id: string | null; onAgain: () => void }) {
  return (
    <div className={styles.sent}>
      <span className={styles.sentIcon} aria-hidden>
        <Icon name="check" size={20} strokeWidth={2.2} />
      </span>
      <h2 id="pilot-sent" tabIndex={-1}>
        Request received
      </h2>
      {id ? (
        <div className="stack" style={{ ["--gap" as string]: "6px" }}>
          <span className="small muted">Your reference</span>
          <span className={styles.ref}>
            <span>{id}</span>
            <CopyButton value={id} label="Copy the reference" />
          </span>
        </div>
      ) : null}
      <p>
        We reply to the contact you gave, about a pilot or early access.{id ? " Quote the reference if you write to us again." : ""}
      </p>
      <p className="small muted">Your details are stored on our server only, never published. Nothing of them is kept in this page.</p>
      <div className="row">
        <Link href="/projects" className="btn btn-sm">
          Open a program
        </Link>
        <Link href="/try" className="btn btn-sm">
          Try it, no account
        </Link>
        <button type="button" className="btn btn-sm btn-ghost" onClick={onAgain}>
          Send another request
        </button>
      </div>
    </div>
  );
}
