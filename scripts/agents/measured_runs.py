#!/usr/bin/env python3
"""Measured agent runs (docs/agent-runs.md): an off-the-shelf agent (Claude Code) with only Antibody's
MCP server completes the practice program's loop with the tutorial proof, N times. The practice program
is a tutorial whose bug and proof are public: these runs measure completing the loop (sealing,
verification, payment), not finding bugs.

Each run gets a fresh workspace and no built-in tools (`--tools ""`, `--strict-mcp-config`), so the
agent can act only through the MCP server. Success is read from the chain, never from the agent's
words: the practice account's commit must end `Done:Paid`.

    python3 scripts/agents/measured_runs.py --runs 10 --network sandbox --rpc http://127.0.0.1:3030 \\
        --deployments target/demo/deployments.json --antibody target/release/antibody --out results/agent-runs/sandbox

With `--remote "ssh <host>"` the MCP server (and the on-chain checks) run on that host, next to the
chains (`antibody-services demo --serve`); the agent runs here and talks to the server over SSH stdio.
"""
import argparse
import datetime
import json
import os
import pathlib
import shlex
import shutil
import statistics
import subprocess
import tempfile
import time

PROMPT = """You have one MCP server, `antibody`. Antibody pays for verified proofs that a bounty program's \
own invariants break. This deployment has a practice program: a tutorial whose bug and proof are public, \
so anyone can practise the loop. Complete the loop once with the tutorial proof: get a practice account \
named `{name}`, have the proof sealed, committed, verified and settled, then report the verdict, the \
commit's final state and the payout, with the transaction links. Use only the antibody tools."""


def antibody(a, *args):
    """The antibody command line (the deployment's flags first)."""
    cmd = [a.antibody, "--network", a.network, "--deployments", a.deployments]
    if a.rpc:
        cmd += ["--rpc", a.rpc]
    return cmd + list(args)


def command(a, cmd, cwd):
    """`cmd` in `cwd`: here, or on the remote host over SSH."""
    if not a.remote:
        return cmd, cwd
    return shlex.split(a.remote) + [f"mkdir -p {shlex.quote(cwd)} && cd {shlex.quote(cwd)} && {shlex.join(cmd)}"], None


def cli(a, *args):
    """The antibody CLI's JSON output (view calls)."""
    cmd, cwd = command(a, antibody(a, *args), "/tmp/antibody-agent-checks" if a.remote else ".")
    out = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=120)
    if out.returncode != 0:
        raise RuntimeError(f"antibody {' '.join(args)}: {out.stderr.strip()[:400]}")
    return json.loads(out.stdout)


def label(state):
    if isinstance(state, str):
        return state
    if isinstance(state, dict) and state:
        k, v = next(iter(state.items()))
        return f"{k}:{next(iter(v))}" if isinstance(v, dict) and v else k
    return "-"


def on_chain(a, program, account):
    """The practice account's commit, read from near-antibody: its state, payout, run and the agent statement."""
    p = cli(a, "status", "--program", str(program), "--of", account)
    rows = [c for c in p.get("program", {}).get("commits", []) if c.get("hunter") == account]
    acc = p.get("account", {})
    return {"commits": rows, "states": [c.get("state") for c in rows], "statement": acc.get("agent") or {}, "account": acc}


def one_run(a, i, practice, stamp):
    name = f"m{stamp}-{i:02d}"
    ws = pathlib.Path(tempfile.mkdtemp(prefix=f"antibody-agent-{i:02d}-"))
    # The MCP server's workspace (and its local state: the practice key): here, or on the remote host.
    server_ws = f"/tmp/antibody-agent-{stamp}-{i:02d}" if a.remote else str(ws)
    server, _ = command(a, antibody(a, "mcp", "--workspace", server_ws), server_ws)
    if not a.remote:
        server = [str(pathlib.Path(server[0]).resolve())] + server[1:]
    cfg = {"mcpServers": {"antibody": {"command": server[0], "args": server[1:]}}}
    (ws / "mcp.json").write_text(json.dumps(cfg), encoding="utf-8")
    cmd = [a.claude, "-p", PROMPT.format(name=name), "--mcp-config", str(ws / "mcp.json"), "--strict-mcp-config", "--tools", "",
           "--allowedTools", "mcp__antibody", "--output-format", "stream-json", "--verbose", "--no-session-persistence"]
    if a.model:
        cmd += ["--model", a.model]
    t0 = time.time()
    try:
        out = subprocess.run(cmd, cwd=ws, capture_output=True, text=True, timeout=a.timeout, encoding="utf-8")
        transcript, err, code = out.stdout, out.stderr, out.returncode
    except subprocess.TimeoutExpired as e:
        transcript, err, code = (e.stdout or b"").decode() if isinstance(e.stdout, bytes) else (e.stdout or ""), "timeout", -1
    secs = time.time() - t0
    events = []
    for line in transcript.splitlines():
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            pass
    final = next((e for e in reversed(events) if e.get("type") == "result"), {})
    init = next((e for e in events if e.get("type") == "system" and e.get("subtype") == "init"), {})
    tools_called = [b.get("name") for e in events if e.get("type") == "assistant"
                    for b in e.get("message", {}).get("content", []) if b.get("type") == "tool_use"]
    account = f"{name}.{practice['vending']}"
    chain = {}
    try:
        chain = on_chain(a, practice["program"], account)
    except Exception as e:  # the account may not exist if the agent never vended
        chain = {"error": str(e)}
    states = [label(s) for s in chain.get("states", [])]
    st = chain.get("statement", {})
    paid = int(st.get("paid") or 0)
    record = {
        "run": i, "name": name, "account": account, "exit_code": code, "seconds": round(secs, 1),
        "success": "Done:Paid" in states, "states": states, "paid_raw": str(paid),
        "verification_spent_yocto": st.get("verification_spent_yocto"), "bonds_locked": st.get("bonds_locked"),
        "bonds_lost": st.get("bonds_lost"), "bonds_returned": st.get("bonds_returned"), "agent_paid": st.get("paid"),
        "net_tokens": st.get("net_tokens"), "tool_calls": tools_called, "num_turns": final.get("num_turns"),
        "cost_usd": final.get("total_cost_usd"), "duration_ms": final.get("duration_ms"), "model": init.get("model"),
        "final_text": (final.get("result") or "")[:2000], "stderr_tail": err[-600:] if err else "",
    }
    return record, transcript, ws


def near(yocto):
    return f"{int(yocto or 0) / 1e24:.4f}"


def tokens(raw, decimals=6):
    return f"{int(raw or 0) / 10**decimals:g}"


def report(s):
    """The published table: one row per run, read from the chain, and the totals."""
    lines = [f"# Measured agent runs: {s['network']}, {s['started']}", "",
             "An off-the-shelf agent (Claude Code, no built-in tools) with only Antibody's MCP server completes the practice "
             "program's loop with the tutorial proof (a public, textbook bug). This measures completing the loop: sealing, "
             "verification and payment. It is not bug finding. Success is read from near-antibody, not from the agent.", "",
             f"- Runs: {s['runs']}; succeeded: {s['succeeded']} ({s['success_rate'] * 100:.0f}%)",
             f"- Wall time of a successful run: median {s['seconds']['median']} s (min {s['seconds']['min']}, max {s['seconds']['max']})",
             f"- Verification spent (all runs, NEAR, paid by the vending contract): {near(s['verification_spent_yocto_total'])}",
             f"- Paid (all runs): {tokens(s['paid_raw_total'])}; bonds lost: {tokens(s['bonds_lost_total'])}",
             f"- Agent cost (all runs, USD, as Claude Code reports it): {s['cost_usd_total']}; models: {', '.join(s['models']) or '-'}", "",
             "| Run | Success | Final state | Seconds | Tool calls | Verification (NEAR) | Bonds lost | Paid | Net tokens | Cost (USD) |",
             "|---|---|---|---|---|---|---|---|---|---|"]
    for r in s["records"]:
        lines.append(f"| {r['run']} | {'yes' if r['success'] else 'no'} | {', '.join(r['states']) or '-'} | {r['seconds']} | "
                     f"{', '.join(t.removeprefix('mcp__antibody__') for t in r['tool_calls']) or '-'} | {near(r['verification_spent_yocto'])} | "
                     f"{tokens(r['bonds_lost'])} | {tokens(r['paid_raw'])} | {tokens(r['net_tokens'])} | {r['cost_usd']} |")
    lines += ["", "Full transcripts: `logs/run-NN.jsonl` (Claude Code's stream-json, local paths replaced).", "",
              "The prompt, the same for every run:", "", "> " + s["prompt"].replace("\n", " "), ""]
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=10)
    ap.add_argument("--network", default="testnet")
    ap.add_argument("--rpc")
    ap.add_argument("--deployments", required=True)
    ap.add_argument("--antibody", required=True)
    ap.add_argument("--claude", default=shutil.which("claude") or "claude")
    ap.add_argument("--model")
    ap.add_argument("--timeout", type=int, default=900)
    ap.add_argument("--out", required=True)
    ap.add_argument("--remote", help='run the MCP server and checks on a host, e.g. "ssh -i key root@host"')
    a = ap.parse_args()
    if a.remote:
        cmd, _ = command(a, ["cat", a.deployments], "/tmp")
        dep = json.loads(subprocess.run(cmd, capture_output=True, text=True, timeout=60, check=True).stdout)
    else:
        dep = json.loads(pathlib.Path(a.deployments).read_text(encoding="utf-8"))
    practice = dep.get("practice") or {}
    if not practice:
        raise SystemExit("the deployments file names no practice program")
    out = pathlib.Path(a.out)
    (out / "logs").mkdir(parents=True, exist_ok=True)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%m%d%H%M")
    records = []
    for i in range(a.runs):
        r, transcript, ws = one_run(a, i, practice, stamp)
        # The transcript as the agent produced it; local paths are replaced by placeholders.
        clean = transcript.replace(str(ws), "<workspace>").replace(str(pathlib.Path(a.antibody).resolve()), "<antibody>")
        if a.remote:
            clean = clean.replace(a.remote, "<remote>")
        clean = clean.replace(str(pathlib.Path.home()), "<home>").replace(json.dumps(str(pathlib.Path.home()))[1:-1], "<home>")
        (out / "logs" / f"run-{i:02d}.jsonl").write_text(clean, encoding="utf-8", newline="\n")
        shutil.rmtree(ws, ignore_errors=True)
        records.append(r)
        print(f"run {i}: success={r['success']} states={r['states']} {r['seconds']} s, {len(r['tool_calls'])} tool calls, cost {r['cost_usd']}")
    ok = [r for r in records if r["success"]]
    secs = [r["seconds"] for r in ok]
    summary = {
        "network": a.network, "practice": practice, "runs": len(records), "succeeded": len(ok),
        "success_rate": round(len(ok) / len(records), 3) if records else 0,
        "seconds": {"median": statistics.median(secs) if secs else None, "max": max(secs) if secs else None, "min": min(secs) if secs else None},
        "verification_spent_yocto_total": str(sum(int(r["verification_spent_yocto"] or 0) for r in records)),
        "paid_raw_total": str(sum(int(r["paid_raw"]) for r in records)),
        "bonds_lost_total": str(sum(int(r["bonds_lost"] or 0) for r in records)),
        "cost_usd_total": round(sum(r["cost_usd"] or 0 for r in records), 4),
        "models": sorted({r["model"] for r in records if r["model"]}),
        "prompt": PROMPT, "started": stamp, "records": records,
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8", newline="\n")
    (out / "REPORT.md").write_text(report(summary), encoding="utf-8", newline="\n")
    print(json.dumps({k: v for k, v in summary.items() if k not in ("records", "prompt")}, indent=2))


if __name__ == "__main__":
    main()
