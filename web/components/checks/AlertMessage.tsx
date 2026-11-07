import { Fragment } from "react";
import { Time } from "@/components/ui/Time";
import { Md } from "./Md";

const ISO_TIME = /(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z)/;

/** An RFC 3339 time as Unix seconds (null when it is not one). */
function unixOf(iso: string): number | null {
  const ms = Date.parse(iso);
  return Number.isNaN(ms) ? null : Math.floor(ms / 1000);
}

/**
 * An alert's message as the page writes its own text: "2 permits", not "2 permit(s)", and each RFC 3339
 * time in UTC with its relative time. Code spans stay code. The status page and the try's "Before you
 * start" both show `/v1/status` alerts through it, so the same alert reads the same on each.
 */
export function AlertMessage({ text }: { text: string }) {
  const plural = text.replace(/\b(\d+)([^()\d]*?)\(s\)/g, (_, n: string, words: string) => `${n}${words}${n === "1" ? "" : "s"}`);
  return (
    <>
      {plural.split(ISO_TIME).map((part, i) =>
        i % 2 ? (
          <Time key={i} t={unixOf(part)} />
        ) : (
          <Fragment key={i}>
            <Md text={part} />
          </Fragment>
        ),
      )}
    </>
  );
}
