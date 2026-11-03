#!/usr/bin/env python3
"""Prints what a run recorded, at a dotted path of its results file: the values JUDGES.md takes from the
run itself (timings, refusals, counts) rather than from chain state.

    python3 scripts/docs/recorded.py one_moment.seconds_from_commit
    python3 scripts/docs/recorded.py 'rounds.*.summary' [--file target/demo/results.json]

The file is the run that is serving when there is one (target/demo/results.json), else the recorded run
JUDGES.md was generated from (docs/results/sandbox-2026-11-03.json). `*` maps over a list.
"""
import argparse
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
DEFAULTS = [ROOT / "target/demo/results.json", ROOT / "docs/results/sandbox-2026-11-03.json"]


def walk(value, parts, soft=False):
    """The value at `parts`; under `*`, an entry without the key gives null instead of an error."""
    if not parts:
        return value
    head, rest = parts[0], parts[1:]
    if head == "*":
        items = value if isinstance(value, list) else list(value.values())
        return [walk(v, rest, soft=True) for v in items]
    if isinstance(value, list):
        return walk(value[int(head)], rest, soft)
    if not isinstance(value, dict) or head not in value:
        if soft:
            return None
        sys.exit(f"no '{head}' here; keys: {', '.join(value) if isinstance(value, dict) else '(not an object)'}")
    return walk(value[head], rest, soft)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("path", help="a dotted path, e.g. one_moment.seconds_from_commit")
    ap.add_argument("--file", type=pathlib.Path)
    a = ap.parse_args()
    f = a.file or next((p for p in DEFAULTS if p.exists()), DEFAULTS[-1])
    data = json.loads(f.read_text(encoding="utf-8"))
    print(f"# {f.relative_to(ROOT) if f.is_relative_to(ROOT) else f}", file=sys.stderr)
    print(json.dumps(walk(data, a.path.split(".")), indent=2))


if __name__ == "__main__":
    main()
