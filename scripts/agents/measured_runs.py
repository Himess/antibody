#!/usr/bin/env python3
"""Measured agent runs (docs/agent-runs.md): an off-the-shelf agent (Claude Code) with only Antibody's
MCP server completes the practice program's loop, N times. The practice program is a tutorial whose bug
and proof are public: these runs measure completing the loop (sealing, verification, payment), not
finding bugs.

Two tasks (`--task`):
- `a`: the tutorial proof through `practice_try`. No built-in tools (`--tools ""`, `--strict-mcp-config`),
  so the agent acts only through the MCP server.
- `a2`: the agent writes the proof itself: `hunt_start` on the practice vending contract, the program's
  sources and rules in the workspace it creates, the bundle written there, `check_poc`, `hunt_submit`.
  The built-in tools are the file tools only (Read, Grep, Glob, Write), in the run's workspace.

Each run gets a fresh workspace and a fresh account name. Success is read from the chain, never from the
agent's words: the practice account's commit must end `Done:Paid`.

    python3 scripts/agents/measured_runs.py --runs 10 --network sandbox --rpc http://127.0.0.1:3030 \\
        --deployments target/demo/deployments.json --antibody target/release/antibody --out results/agent-runs/sandbox

With `--remote "ssh <host>"` the MCP server (and the on-chain checks) run on that host, next to the
chains (`antibody-services demo --serve`); the agent runs here and talks to the server over SSH stdio.
`--cast <dir>` also writes each run's output as an asciinema cast (v2), timed as the lines arrived.
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

PROMPT_A2 = """You have one MCP server, `antibody`, and file tools that work in this folder. Antibody pays \
for verified proofs that a bounty program's own invariants break. This deployment has a practice program: a \
tutorial whose bug is public, so anyone can practise writing a proof. Write the proof yourself and submit \
it once: get an account named `{name}` and the instance it may prove with hunt_start (vending `{vending}`), \
read the program's rules and sources in the workspace it creates, write the proof bundle there, check it \
with check_poc, then submit it with hunt_submit. Report the verdict, the commit's final state and the \
payout, with the transaction links. Use only the antibody tools and the file tools."""

# Each task: its prompt, and the built-in tools the agent has besides the MCP server.
TASKS = {"a": (PROMPT, ""), "a2": (PROMPT_A2, "Read,Grep,Glob,Write")}


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
    p = cli(a, "status", "--program", str(program), "--of", account, "--json")
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
    # Off the shelf, and nothing of this machine's own setup: only this MCP server and the task's built-in
    # tools (none for task a), and no user settings or hooks (the workspace has no project settings).
    prompt, tools = TASKS[a.task]
    allowed = ",".join(["mcp__antibody"] + [t for t in tools.split(",") if t])
    cmd = [a.claude, "-p", prompt.format(name=name, vending=practice["vending"]), "--mcp-config", str(ws / "mcp.json"), "--strict-mcp-config",
           "--tools", tools, "--allowedTools", allowed, "--setting-sources", "project", "--output-format", "stream-json", "--verbose",
           "--no-session-persistence"]
    if a.model:
        cmd += ["--model", a.model]
    t0 = time.time()
    # Each line with the second it arrived, for the cast (`--cast`).
    timed, err, code = [], "", -1
    proc = subprocess.Popen(cmd, cwd=ws, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8")
    try:
        deadline = t0 + a.timeout
        for line in proc.stdout:
            timed.append((round(time.time() - t0, 3), line.rstrip("\n")))
            if time.time() > deadline:
                proc.kill()
                err = "timeout"
                break
        code = proc.wait(timeout=60)
        err = err or proc.stderr.read()
    except subprocess.TimeoutExpired:
        proc.kill()
        err = "timeout"
    transcript = "\n".join(line for _, line in timed) + ("\n" if timed else "")
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
        "run": i, "task": a.task, "name": name, "account": account, "exit_code": code, "seconds": round(secs, 1),
        "success": "Done:Paid" in states, "states": states, "commits": [c.get("id") for c in chain.get("commits", [])], "paid_raw": str(paid),
        "verification_spent_yocto": st.get("verification_spent_yocto"), "bonds_locked": st.get("bonds_locked"),
        "bonds_lost": st.get("bonds_lost"), "bonds_returned": st.get("bonds_returned"), "agent_paid": st.get("paid"),
        "net_tokens": st.get("net_tokens"), "tool_calls": tools_called, "num_turns": final.get("num_turns"),
        "cost_usd": final.get("total_cost_usd"), "duration_ms": final.get("duration_ms"), "model": init.get("model"),
        "final_text": (final.get("result") or "")[:2000], "stderr_tail": err[-600:] if err else "",
    }
    return record, transcript, ws, timed


def cast(timed, title):
    """An asciinema cast (v2) of a run's stream-json lines, each shown when it arrived: the agent's words,
    each tool call with its input, each tool's answer (shortened) and the result."""
    def short(v, n=600):
        s = v if isinstance(v, str) else json.dumps(v, ensure_ascii=False)
        return s if len(s) <= n else s[:n] + " …"
    out = [json.dumps({"version": 2, "width": 120, "height": 36, "title": title, "env": {"TERM": "xterm-256color"}})]
    for t, line in timed:
        try:
            e = json.loads(line)
        except json.JSONDecodeError:
            continue
        text = ""
        if e.get("type") == "assistant":
            for b in e.get("message", {}).get("content", []):
                if b.get("type") == "text" and b.get("text", "").strip():
                    text += f"\x1b[1magent\x1b[0m {b['text'].strip()}\r\n"
                elif b.get("type") == "tool_use":
                    text += f"\x1b[36m→ {b.get('name', '').removeprefix('mcp__antibody__')}\x1b[0m {short(b.get('input', {}), 300)}\r\n"
        elif e.get("type") == "user":
            for b in e.get("message", {}).get("content", []):
                if isinstance(b, dict) and b.get("type") == "tool_result":
                    c = b.get("content")
                    body = " ".join(x.get("text", "") for x in c if isinstance(x, dict)) if isinstance(c, list) else (c or "")
                    text += f"\x1b[2m← {short(body)}\x1b[0m\r\n"
        elif e.get("type") == "result":
            text += f"\x1b[1;32mresult\x1b[0m {short(e.get('result') or '', 1500)}\r\n"
        if text:
            out.append(json.dumps([t, "o", text.replace("\n", "\r\n").replace("\r\r\n", "\r\n")], ensure_ascii=False))
    return "\n".join(out) + "\n"


def near(yocto):
    """NEAR from yoctoNEAR; "-" when the statement does not record it (a vended account's runs are paid
    by its vending contract, not counted on the account)."""
    return "-" if yocto in (None, "") else f"{int(yocto) / 1e24:.4f}"


def usd(v):
    return "-" if v is None else f"{v:.3f}"


def tokens(raw, decimals=6):
    return f"{int(raw or 0) / 10**decimals:g}"


def report(s):
    """The published table: one row per run, read from the chain, and the totals."""
    d, sym = s.get("token_decimals", 6), s.get("token_symbol", "tUSD")
    what = ("An off-the-shelf agent (Claude Code, no built-in tools) with only Antibody's MCP server completes the practice "
            "program's loop with the tutorial proof (a public, textbook bug), through `practice_try`."
            if s.get("task", "a") == "a" else
            "An off-the-shelf agent (Claude Code) with Antibody's MCP server and file tools in its workspace (Read, Grep, Glob, Write) "
            "writes the practice program's proof itself (a public, textbook bug): `hunt_start` on the practice vending contract, the "
            "bundle written from the workspace's sources and proof format, `check_poc`, `hunt_submit`.")
    lines = [f"# Measured agent runs: task {s.get('task', 'a')}, {s['network']}, {s.get('started_utc') or s['started']}", "",
             what + " This measures completing the loop: sealing, verification and payment. It is not bug finding. Success is read "
             "from near-antibody, not from the agent.", ""]
    if s.get("versions"):
        lines += [f"- Versions: {s['versions'].get('antibody', '-')} (the MCP server); Claude Code {s['versions'].get('claude', '-')}"]
    lines += [f"- Runs: {s['runs']}; paid within the agent's own call: {s['succeeded']} ({s['success_rate'] * 100:.0f}%)"]
    if s.get("paid_on_chain") is not None:
        lines += [f"- Paid on near-antibody when read again ({s.get('rechecked_at', '-')}): {s['paid_on_chain']} of {s['runs']} "
                  "(a payout the agent's call did not wait for, or did not live to see, settled by the services' keeper)"]
    lines += [f"- Wall time of a run paid within its call: median {s['seconds']['median']} s (min {s['seconds']['min']}, max {s['seconds']['max']})",
             f"- Paid (all runs): {tokens(s['paid_raw_total'], d)} {sym} within the calls"
             + (f", {tokens(s['paid_final_raw_total'], d)} {sym} when read again" if s.get("paid_final_raw_total") is not None else "")
             + f"; bonds lost: {tokens(s['bonds_lost_total'], d)} {sym}",
             f"- Agent cost (all runs, USD, as Claude Code reports it): {usd(s['cost_usd_total'])}; models: {', '.join(s['models']) or '-'}", "",
             f"| Run | Paid in its call | Commit | State at its end | State read again | Seconds | Tool calls | Verification (NEAR) | Bonds lost ({sym}) | Paid ({sym}, read again) | Net ({sym}, in its call) | Cost (USD) |",
             "|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for r in s["records"]:
        lines.append(f"| {r['run']} | {'yes' if r['success'] else 'no'} | {', '.join(str(c) for c in r.get('commits') or []) or '-'} | "
                     f"{', '.join(r['states']) or '-'} | {', '.join(r.get('final_states') or []) or '-'} | {r['seconds']} | "
                     f"{', '.join(t.removeprefix('mcp__antibody__') for t in r['tool_calls']) or '-'} | {near(r['verification_spent_yocto'])} | "
                     f"{tokens(r['bonds_lost'], d)} | {tokens(r.get('paid_final_raw', r['paid_raw']), d)} | {tokens(r['net_tokens'], d)} | {usd(r['cost_usd'])} |")
    for note in s.get("notes") or []:
        lines += ["", note]
    lines += ["", "Full transcripts: `logs/run-NN.jsonl` (Claude Code's stream-json, local paths replaced).", "",
              "The prompt, the same for every run:", "", "> " + s["prompt"].replace("\n", " "), ""]
    return "\n".join(lines)


def clean_paths(text, places, remote=None):
    """The transcript with local paths replaced by placeholders: each place, then the home directory, at
    every level of JSON escaping (a tool's answer is JSON inside the transcript's JSON, so a Windows path
    appears with its backslashes doubled, then doubled again), with or without Windows' `\\\\?\\` prefix, and
    the home directory as Claude Code names its project folders (`C--Users-<name>-…`)."""
    home = pathlib.Path.home()
    places = list(places) + [(str(home), "<home>")]
    for raw, mark in places:
        variants = {raw, raw.replace("\\", "/")}
        for v in list(variants):
            variants.add("\\\\?\\" + v)
        for v in sorted(variants, key=len, reverse=True):
            esc = v
            for _ in range(4):
                text = text.replace(esc, mark)
                esc = json.dumps(esc)[1:-1]
    slug = str(home).replace(":", "-").replace("\\", "-").replace("/", "-")
    text = text.replace(slug, "<home>")
    return text.replace(remote, "<remote>") if remote else text


def recheck(a, s):
    """Each run's commit read again from near-antibody (`final_states`): a payout the agent's call did not
    wait for is settled by the keeper later. Counts `paid_on_chain`."""
    for r in s["records"]:
        try:
            chain = on_chain(a, s["practice"]["program"], r["account"])
            r["final_states"] = [label(x) for x in chain.get("states", [])]
            r["paid_final_raw"] = str(int((chain.get("statement") or {}).get("paid") or 0))
        except Exception as e:  # noqa: BLE001 - the account may never have vended
            r["final_states"] = []
            r["recheck_error"] = str(e)[:200]
    s["paid_on_chain"] = sum(1 for r in s["records"] if "Done:Paid" in (r.get("final_states") or []))
    s["paid_final_raw_total"] = str(sum(int(r.get("paid_final_raw") or 0) for r in s["records"]))
    s["rechecked_at"] = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    return s


def versions(a):
    """The MCP server's and the agent's versions, as their binaries print them."""
    def v(cmd):
        try:
            return subprocess.run(cmd, capture_output=True, text=True, timeout=30).stdout.strip().splitlines()[0]
        except Exception:  # noqa: BLE001
            return "-"
    return {"antibody": v([a.antibody, "--version"]), "claude": v([a.claude, "--version"])}


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
    ap.add_argument("--task", choices=sorted(TASKS), default="a", help="a: practice_try; a2: the agent writes the proof (hunt tools)")
    ap.add_argument("--cast", help="also write each run's output as an asciinema cast (v2) in this directory")
    ap.add_argument("--settle-wait", type=int, default=300, help="seconds after the last run before each commit is read again")
    ap.add_argument("--note", action="append", default=[], help="a note printed in the report (the conditions of the runs)")
    ap.add_argument("--recheck", action="store_true", help="read the commits of the runs in --out again and rewrite its report; run nothing")
    a = ap.parse_args()
    if a.recheck:
        out = pathlib.Path(a.out)
        s = recheck(a, json.loads((out / "summary.json").read_text(encoding="utf-8")))
        s["notes"] = list(dict.fromkeys((s.get("notes") or []) + a.note))
        s.setdefault("versions", versions(a))
        (out / "summary.json").write_text(json.dumps(s, indent=2) + "\n", encoding="utf-8", newline="\n")
        (out / "REPORT.md").write_text(report(s), encoding="utf-8", newline="\n")
        print(f"paid on near-antibody: {s['paid_on_chain']} of {s['runs']} ({s['rechecked_at']})")
        return
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
    now = datetime.datetime.now(datetime.timezone.utc)
    stamp, started_utc = now.strftime("%m%d%H%M"), now.strftime("%Y-%m-%d %H:%M UTC")
    records = []
    for i in range(a.runs):
        r, transcript, ws, timed = one_run(a, i, practice, stamp)

        def clean(text):
            return clean_paths(text, [(str(ws), "<workspace>"), (str(pathlib.Path(a.antibody).resolve()), "<antibody>")], a.remote)
        (out / "logs" / f"run-{i:02d}.jsonl").write_text(clean(transcript), encoding="utf-8", newline="\n")
        if a.cast:
            pathlib.Path(a.cast).mkdir(parents=True, exist_ok=True)
            title = f"Antibody: task {a.task}, run {i} ({r['account']})"
            (pathlib.Path(a.cast) / f"task-{a.task}-run-{i:02d}.cast").write_text(clean(cast(timed, title)), encoding="utf-8", newline="\n")
        shutil.rmtree(ws, ignore_errors=True)
        records.append(r)
        print(f"run {i}: success={r['success']} states={r['states']} {r['seconds']} s, {len(r['tool_calls'])} tool calls, cost {r['cost_usd']}")
        if "Failed to authenticate" in r["final_text"]:
            raise SystemExit(f"the agent could not authenticate ({r['final_text'][:120]}): log in to Claude Code, then run again")
    ok = [r for r in records if r["success"]]
    secs = [r["seconds"] for r in ok]
    summary = {
        "task": a.task, "network": a.network, "practice": practice, "runs": len(records), "succeeded": len(ok),
        "token_decimals": dep.get("token_decimals", 6), "token_symbol": dep.get("token_symbol", "tUSD"),
        "success_rate": round(len(ok) / len(records), 3) if records else 0,
        "seconds": {"median": statistics.median(secs) if secs else None, "max": max(secs) if secs else None, "min": min(secs) if secs else None},
        "verification_spent_yocto_total": str(sum(int(r["verification_spent_yocto"] or 0) for r in records)),
        "paid_raw_total": str(sum(int(r["paid_raw"]) for r in records)),
        "bonds_lost_total": str(sum(int(r["bonds_lost"] or 0) for r in records)),
        "cost_usd_total": round(sum(r["cost_usd"] or 0 for r in records), 4),
        "models": sorted({r["model"] for r in records if r["model"]}),
        "prompt": TASKS[a.task][0], "tools": TASKS[a.task][1] or "none", "started": stamp, "records": records,
        "versions": versions(a), "notes": a.note, "started_utc": started_utc,
    }
    # The commits read again once the keeper has had time to settle what the agents' calls did not wait for.
    time.sleep(max(0, a.settle_wait))
    summary = recheck(a, summary)
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8", newline="\n")
    (out / "REPORT.md").write_text(report(summary), encoding="utf-8", newline="\n")
    print(json.dumps({k: v for k, v in summary.items() if k not in ("records", "prompt")}, indent=2))


if __name__ == "__main__":
    main()
