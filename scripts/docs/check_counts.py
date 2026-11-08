#!/usr/bin/env python3
"""CI gate: the counts the README states match the files they count (no network, standard library only).

- "<n> findings from attacking our own system": the rows of docs/attacks.md's self-attack log, SA-1 to SA-<n>
  in order with no gap.
- "<n> settlement properties": the numbered rows of docs/properties.md's table of properties.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]


def read(rel):
    return (ROOT / rel).read_text(encoding="utf-8")


def stated(readme, pattern, what):
    m = re.search(pattern, readme)
    if not m:
        raise SystemExit(f"README.md: no line states the {what} (pattern {pattern!r})")
    return int(m.group(1))


def main():
    readme = read("README.md")
    errors = []

    ids = [int(x) for x in re.findall(r"^\| SA-(\d+) \|", read("docs/attacks.md"), re.M)]
    if ids != list(range(1, len(ids) + 1)):
        errors.append(f"docs/attacks.md: the self-attack rows are not SA-1 to SA-{len(ids)} in order: {ids}")
    said = stated(readme, r"\*\*(\d+) findings from attacking our own system", "self-attack findings")
    if said != len(ids):
        errors.append(f"README.md says {said} self-attack findings; docs/attacks.md lists {len(ids)} (SA-1 to SA-{len(ids)})")

    props = read("docs/properties.md")
    if "\n## The properties" not in props:
        errors.append("docs/properties.md: no '## The properties' section")
    else:
        table = props.split("\n## The properties", 1)[1].split("\n## ", 1)[0]
        rows = [int(x) for x in re.findall(r"^\| (\d+) \| \*\*", table, re.M)]
        if rows != list(range(1, len(rows) + 1)):
            errors.append(f"docs/properties.md: the properties are not numbered 1 to {len(rows)} in order: {rows}")
        said = stated(readme, r"\*\*(\d+) settlement properties", "settlement properties")
        if said != len(rows):
            errors.append(f"README.md says {said} settlement properties; docs/properties.md lists {len(rows)}")

    if errors:
        print("\n".join(errors))
        sys.exit(1)
    print(f"OK: {len(ids)} self-attack findings and {len(rows)} settlement properties, as README.md says")


if __name__ == "__main__":
    main()
