#!/usr/bin/env python3
"""Writes the Acme Lend submissions (A-F and the project's known issue) as bundles for one deployment
of the fixture (DELIBERATELY VULNERABLE TEST FIXTURE), with C's report:

  python3 scripts/demo/acme_bundles.py <deployment.json> <out dir>

The deployment file is what script/DeployAcme.s.sol writes. The bundles are the ones
scripts/demo/acme_fixture.py builds the local fixture from, at that deployment's addresses (the Acme
contracts' CREATE2 addresses depend on the deployer).
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
from acme_fixture import submissions  # noqa: E402


def main(dep_file, out_dir):
    dep = json.load(open(dep_file))
    os.makedirs(out_dir, exist_ok=True)
    for agent, (_title, bundle, report) in submissions(dep).items():
        prefix = os.path.join(out_dir, agent)
        json.dump(bundle, open(prefix + ".bundle.json", "w"), indent=1)
        if report:
            open(prefix + ".report.md", "w", encoding="utf-8").write(report)
    print(f"bundles for {', '.join(submissions(dep))} in {out_dir}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
