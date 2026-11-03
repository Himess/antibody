// -*- mode: Bluespec; -*-
/**
 * Antibody settlement: a small-scope model of one program.
 *
 * It models the rules near-antibody enforces (docs/spec.md):
 * - escrow and bond accounting;
 * - commit order and the allocation cursor;
 * - keys, duplicates and the one-way appeal;
 * - the T0/T1 gate, with windows and an instant budget per window;
 * - challenges settled by re-execution;
 * - rounds with immune memory;
 * - sealed known issues and the gap lane;
 * - pause and release permits, disclosure, and the exit after the deadline.
 * The twelve properties of docs/properties.md are the invariants at the end.
 *
 * Adversary.
 * - A T0 verdict may report anything (a lying or broken operator).
 * - A T1 verdict reports what the PoC really does (TDX and the approved images are the trust root), and
 *   so does a re-run whose quote is verified.
 * - What a PoC really does (`truth`) is chosen when it is revealed. What it does after a proposed fix
 *   follows from which keys the fix really fixes, also chosen nondeterministically.
 */
module antibody {

  // ================================================================ scope
  pure val NC = 4                                    // commits
  pure val CIDS = 0.to(NC - 1)
  pure val MAXR = 2                                  // rounds 0, 1 and 2
  pure val ROUNDS = 0.to(MAXR)
  pure val KEYS = Set("A", "B", "C")                 // root-cause keys: (invariant slot, group)
  pure val SLOT = Map("A" -> 0, "B" -> 1, "C" -> 1)
  pure val GAPSLOT = 2                               // the invariant a round may add (gap lane)
  // Slot 0 is Critical (A), slot 1 High (B and C share it, so they share its budget). Amounts follow the
  // slot's severity, as in near-antibody; they are in token units, times in ticks of 100 s, so the trace
  // replay (crates/rules/tests/quint_replay.rs) runs the same numbers.
  pure val AMT = Map("A" -> 200, "B" -> 400, "C" -> 400)
  pure val CRIT_HIGH = Set("A", "B", "C")            // must be fixed before a release
  pure val PAUSE_KEYS = Set("A")                     // pause policy: Critical slot 0 only
  pure val BUDGET = 500                              // per invariant per round
  pure val ESCROW0 = 500
  pure val TOPUP = 200
  pure val BOND = 100
  pure val CBOND = 100
  pure val FEE_BPS = 1000                            // protocol fee, basis points of a payment
  pure val GAPAMT = 100                              // payout of a promoted gap claim
  pure val CAP = 300                                 // instant cap per payout (B and C are above it)
  pure val IBUDGET = 300                             // instant budget per instant window
  pure val IWIN = 4                                  // instant window length
  pure val REVEAL = 2                                // reveal window after a commit
  pure val WIN = 2                                   // challenge window after a verdict
  pure val RESOLVE = 2                               // time a challenge has to reach a T1 result
  pure val INFRA = 3                                 // retry window after an infrastructure failure
  pure val ROUND_MIN = 3                             // minimum round duration
  pure val RERUN_WIN = 1                             // re-runs happen within this after a proposal
  pure val DISCLOSE_DELAY = 6                        // maximum delay before a paid key is disclosed
  pure val DEADLINE = 10
  pure val EXIT_GRACE = 2
  pure val MAX_KNOWN = 1
  pure val KNOWN_BPS = 5000                          // known issues cover at most this share of the escrow
  pure val KNOWN_SHARE = ESCROW0 * KNOWN_BPS / 10000
  pure val TMAX = DEADLINE + EXIT_GRACE + 1
  pure val HUNTERS = Set("h1", "h2")
  pure val CHALLENGERS = Set("project", "hunter", "third")

  // "VA", "VB", "VC": violation of key A, B, C. "Gap": no invariant broken, harm above the threshold on
  // a declared protected account. "Infra": the run failed for infrastructure reasons (facts only).
  pure val VIOL = Set("VA", "VB", "VC")
  pure val FACTS = VIOL.union(Set("NoV", "Inv", "Unev", "Gap", "Mis", "Infra"))
  pure def keyOf(o: str): str = if (o == "VA") "A" else if (o == "VB") "B" else "C"
  // What a PoC really does: a violation through one of two distinct bugs per key ("VA1", "VA2", ...),
  // or another outcome. Facts report only the key, so two distinct bugs can share a key.
  pure val BUGS = Set("VA1", "VA2", "VB1", "VB2", "VC1", "VC2")
  pure val TRUTHS = BUGS.union(Set("NoV", "Inv", "Unev", "Gap", "Mis"))
  pure def factsOf(t: str): str =
    if (t == "VA1" or t == "VA2") "VA" else if (t == "VB1" or t == "VB2") "VB"
    else if (t == "VC1" or t == "VC2") "VC" else t
  pure def mn(a: int, b: int): int = if (a < b) a else b

  // ================================================================ state
  type Commit = {
    st: str,        // "free" | "committed" | "verifying" | "verdict" | "done"
    hunter: str,
    round: int,     // commits are bound to their round
    tc: int,        // commit time
    tr: int,        // start of the current retry window
    truth: str,     // what the PoC really does at its round's code (a bug id or an outcome)
    facts: str,
    tier: str,      // "T0" | "T1"
    tv: int,        // verdict time
    second: str,    // "none" | "agree": an independent run agreed (needed above the cap)
    chal: str,      // "none" | "open"
    tch: int,
    chalBy: str,
    alloc: str,     // "none" | "holder" | "dup" | "promoted" | "gap"
    of: int,        // for a duplicate: the holder it duplicates
    amount: int,    // claim amount
    res: int,       // reserved for it now
    debt: int,      // owed but not funded (shortfall queue)
    end: str,       // "" or the terminal outcome
    rerun: str,     // "none" | "fixed" | "breaks": the T1 re-run at the proposed fix
  }

  pure val FREE: Commit = {
    st: "free", hunter: "", round: 0, tc: 0, tr: 0, truth: "", facts: "", tier: "T0", tv: 0,
    second: "none", chal: "none", tch: 0, chalBy: "", alloc: "none", of: -1, amount: 0, res: 0,
    debt: 0, end: "", rerun: "none",
  }

  type St = {
    now: int, phase: str, round: int, roundStart: int, misconfigured: bool,
    proposed: bool, fix: Set[str], addsInv: bool, canaries: bool,   // fix: the bugs the round really fixes
    proposedAt: int,
    cs: int -> Commit, n: int, cursor: int,
    holder: (int, str) -> int,          // (round, key) -> commit, -1 when free
    known: Set[str], slots: Set[int],    // known: the bugs declared as known issues
    spent: (int, int) -> int,           // (round, slot) -> amount charged to its budget
    funded: int, available: int, reserved: int, paid: int, fees: int, returned: int, topups: int,
    bondIn: int, bondLocked: int, bondRefunded: int, bondForfeited: int,
    cbondIn: int, cbondLocked: int, cbondToHunter: int, cbondToEscrow: int, cbondRefunded: int,
    credits: int, withdrawn: int,
    iEpoch: int, iSpent: int,
    tickets: int -> int, permits: int,
    // Disclosure is per (round, key): a round's proofs of a key open with that round's key.
    firstClaim: (int, str) -> int, fixedKeys: Set[(int, str)], disclosed: Set[(int, str)], wrapped: Set[(int, str)],
    pins: int, pinsAtOpen: int,
    // ghost state for the properties
    prevPaid: int, prevSlots: Set[int], prevCond: Set[(int, str)], knownAfterOpen: bool,
    payLog: Set[(int, str, int)],       // (round, key, commit) of every payment for a key
    instLog: Set[(int, int, int, str)], // (instant window, commit, amount, tier) of every instant payment
    ticketLog: Set[int], permitLog: Set[(int, bool)],
    promoteOk: bool,
  }

  var s: St

  // ================================================================ derived
  def aboveCap(x: Commit): bool = VIOL.contains(x.facts) and AMT.get(keyOf(x.facts)) > CAP

  // The settlement gate. A verdict is final when no challenge is open and either it is T1 below the
  // cap, or its challenge window has passed (above the cap also with an agreeing independent run).
  def isFinal(x: Commit): bool = and {
    x.st == "verdict", x.facts != "Infra", x.chal == "none",
    or {
      x.tier == "T1" and not(aboveCap(x)),
      s.now > x.tv + WIN and (not(aboveCap(x)) or x.second == "agree"),
    }
  }

  def knownKeys: Set[str] = s.known.map(b => keyOf(factsOf(b)))

  def earlierDebt(c: int): bool = CIDS.exists(d => d < c and s.cs.get(d).debt > 0)

  def discloseCond(rk: (int, str)): bool = or {
    s.fixedKeys.contains(rk),
    s.wrapped.contains(rk) and s.firstClaim.get(rk) >= 0 and s.now >= s.firstClaim.get(rk) + DISCLOSE_DELAY,
    s.phase == "closed" and s.firstClaim.get(rk) >= 0,
  }

  def exitEnabled: bool = s.phase != "closed" and (s.now >= DEADLINE + EXIT_GRACE or s.misconfigured)
  // The exit waits until every commit is allocated or closed, so it voids nothing that could be paid.
  def canExit: bool = exitEnabled and (s.misconfigured or s.cursor >= s.n)

  // What a re-run at the proposed fix shows (a quote is verified, so it is the truth).
  def afterFix(x: Commit): str =
    if (BUGS.contains(x.truth)) (if (s.fix.contains(x.truth)) "fixed" else "breaks")
    else if (x.truth == "Gap") (if (s.addsInv) "breaks" else "fixed")
    else "fixed"

  // Whether commit c holds a key in round r: as first finding, carried or promoted.
  def holdsIn(c: int, r: int): bool = KEYS.exists(k => s.holder.get((r, k)) == c)
  def everHeld(c: int): bool = ROUNDS.exists(r => r <= s.round and holdsIn(c, r))

  // Release gate (every re-run is at the proposed fix: results reset when a round opens):
  // - every paid PoC, from any round, is re-run and fixed (immune memory);
  // - every Critical or High PoC of the round, duplicates included, was re-run and is fixed: a
  //   duplicate that still breaks is an open Critical even when the first finding was fixed;
  // - no gap claim still breaks;
  // - the canaries pass, and every Critical or High key ever held is fixed.
  def critHigh(x: Commit): bool = VIOL.contains(x.facts) and CRIT_HIGH.contains(keyOf(x.facts))
  def releaseGate: bool = and {
    s.canaries,
    CIDS.forall(c => c >= s.n or s.cs.get(c).end != "paid" or s.cs.get(c).rerun == "fixed"),
    // every Critical or High PoC of the round, first finding or duplicate, was re-run and is fixed
    CIDS.forall(c => c >= s.n or s.cs.get(c).round != s.round or not(critHigh(s.cs.get(c)))
                       or s.cs.get(c).alloc == "none" or s.cs.get(c).rerun == "fixed"),
    // and no gap claim still harms a protected account
    CIDS.forall(c => c >= s.n or s.cs.get(c).round != s.round or s.cs.get(c).alloc != "gap"
                       or s.cs.get(c).rerun != "breaks"),
    CRIT_HIGH.forall(k => ROUNDS.forall(r => r > s.round or s.holder.get((r, k)) == -1
                       or s.cs.get(s.holder.get((r, k))).rerun == "fixed")),
  }


  // ================================================================ guards (shared by actions and tests)
  def canCommit: bool = s.phase == "open" and not(s.misconfigured) and s.now < DEADLINE and s.n < NC

  def canDeclareKnown(b: str): bool = and {
    s.phase == "setup", not(s.known.contains(b)), s.known.size() < MAX_KNOWN,
    s.known.union(Set(b)).fold(0, (a, j) => a + AMT.get(keyOf(factsOf(j)))) <= KNOWN_SHARE,
  }

  def canAllocate: bool = s.cursor < s.n and (s.cs.get(s.cursor).st == "done" or isFinal(s.cs.get(s.cursor)))

  def instantOk(x: Commit): bool = {
    val ep = s.now / IWIN
    val spentNow = if (ep == s.iEpoch) s.iSpent else 0
    x.tier == "T1" and x.amount <= CAP and spentNow + x.amount <= IBUDGET
  }

  def canPay(c: int): bool = {
    val x = s.cs.get(c)
    and {
      x.st == "verdict", x.alloc == "holder", x.end == "", x.debt == 0, x.res == x.amount,
      instantOk(x) or s.now > x.tv + WIN,
    }
  }

  def canTicket(c: int): bool = {
    val x = s.cs.get(c)
    and {
      s.phase == "open", not(s.misconfigured), x.alloc == "holder", x.round == s.round, x.tier == "T1",
      PAUSE_KEYS.contains(keyOf(x.facts)), s.tickets.get(x.round) < 1,
    }
  }

  // ================================================================ helpers
  def withC(c: int, x: Commit): int -> Commit = s.cs.set(c, x)

  // ================================================================ actions
  action init = all {
    s' = {
      now: 0, phase: "setup", round: 0, roundStart: 0, misconfigured: false,
      proposed: false, fix: Set(), addsInv: false, canaries: true, proposedAt: 0,
      cs: CIDS.mapBy(_ => FREE), n: 0, cursor: 0,
      holder: tuples(ROUNDS, KEYS).mapBy(_ => -1),
      known: Set(), slots: Set(0, 1),
      spent: tuples(ROUNDS, Set(0, 1, GAPSLOT)).mapBy(_ => 0),
      funded: ESCROW0, available: ESCROW0, reserved: 0, paid: 0, fees: 0, returned: 0, topups: 0,
      bondIn: 0, bondLocked: 0, bondRefunded: 0, bondForfeited: 0,
      cbondIn: 0, cbondLocked: 0, cbondToHunter: 0, cbondToEscrow: 0, cbondRefunded: 0,
      credits: 0, withdrawn: 0,
      iEpoch: 0, iSpent: 0,
      tickets: ROUNDS.mapBy(_ => 0), permits: 0,
      firstClaim: tuples(ROUNDS, KEYS).mapBy(_ => -1), fixedKeys: Set(), disclosed: Set(), wrapped: Set(),
      pins: 0, pinsAtOpen: 0,
      prevPaid: 0, prevSlots: Set(0, 1), prevCond: Set(), knownAfterOpen: false,
      payLog: Set(), instLog: Set(), ticketLog: Set(), permitLog: Set(),
      promoteOk: true,
    }
  }

  // Every action records the previous paid total and invariant set, for property 5.
  def ghost(t: St): St = { ...t, prevPaid: s.paid, prevSlots: s.slots, prevCond: tuples(ROUNDS, KEYS).filter(rk => discloseCond(rk)) }

  action Tick = all { s.now < TMAX, s' = ghost({ ...s, now: s.now + 1 }) }

  // ---- setup: pins and sealed known issues, then open
  action OwnerSetPins(v: int): bool = all {
    // The owner may try at any time; only a program in setup changes.
    s' = ghost(if (s.phase == "setup") { ...s, pins: v } else s),
  }

  action DeclareKnown(b: str): bool = {
    all {
      canDeclareKnown(b),
      s' = ghost({ ...s, known: s.known.union(Set(b)), knownAfterOpen: s.knownAfterOpen or s.phase != "setup" }),
    }
  }

  action Open = all {
    s.phase == "setup",
    s' = ghost({ ...s, phase: "open", pinsAtOpen: s.pins, roundStart: s.now }),
  }

  // ---- hunters
  action MakeCommit(h: str): bool = all {
    canCommit,
    s' = ghost({ ...s,
      cs: withC(s.n, { ...FREE, st: "committed", hunter: h, round: s.round, tc: s.now }),
      n: s.n + 1, bondIn: s.bondIn + BOND, bondLocked: s.bondLocked + BOND,
    }),
  }

  action Reveal(c: int, t: str): bool = {
    val x = s.cs.get(c)
    all {
      x.st == "committed", s.now <= x.tc + REVEAL,
      s' = ghost({ ...s, cs: withC(c, { ...x, st: "verifying", truth: t, tr: s.now }) }),
    }
  }

  // An unrevealed commit forfeits its bond to the program's escrow.
  action Expire(c: int): bool = {
    val x = s.cs.get(c)
    all {
      x.st == "committed", s.now > x.tc + REVEAL,
      s' = ghost({ ...s,
        cs: withC(c, { ...x, st: "done", end: "expired" }),
        bondLocked: s.bondLocked - BOND, bondForfeited: s.bondForfeited + BOND,
        funded: s.funded + BOND, available: s.available + BOND,
      }),
    }
  }

  // ---- verifier runs
  action FactsT0(c: int, f: str): bool = {
    val x = s.cs.get(c)
    all {
      x.st == "verifying", s.now <= x.tr + INFRA,
      s' = ghost({ ...s, cs: withC(c, { ...x, st: "verdict", facts: f, tier: "T0", tv: s.now, second: "none" }) }),
    }
  }

  action FactsT1(c: int): bool = {
    val x = s.cs.get(c)
    all {
      x.st == "verifying", s.now <= x.tr + INFRA,
      s' = ghost({ ...s, cs: withC(c, { ...x, st: "verdict", facts: factsOf(x.truth), tier: "T1", tv: s.now, second: "none" }) }),
    }
  }

  // A quote for the same run is relayed later. A quote only ever confirms the true output.
  action ConfirmT1(c: int): bool = {
    val x = s.cs.get(c)
    all {
      x.st == "verdict", x.tier == "T0", x.facts == factsOf(x.truth), x.facts != "Infra", x.chal == "none",
      s' = ghost({ ...s, cs: withC(c, { ...x, tier: "T1" }) }),
    }
  }

  // Infrastructure failures are retried with priority kept; after the retry window the commit is void
  // and its bond refunded. A run that never answers ends the same way.
  action Retry(c: int): bool = {
    val x = s.cs.get(c)
    all {
      x.st == "verdict", x.facts == "Infra", s.now <= x.tr + INFRA,
      s' = ghost({ ...s, cs: withC(c, { ...x, st: "verifying" }) }),
    }
  }

  action InfraTimeout(c: int): bool = {
    val x = s.cs.get(c)
    all {
      x.st == "verifying" or (x.st == "verdict" and x.facts == "Infra"),
      x.chal == "none", s.now > x.tr + INFRA,
      s' = ghost({ ...s,
        cs: withC(c, { ...x, st: "done", end: "void" }),
        bondLocked: s.bondLocked - BOND, bondRefunded: s.bondRefunded + BOND, credits: s.credits + BOND,
      }),
    }
  }

  // Above the cap a second, independent run must agree; a disagreement voids the verdict (retry).
  action SecondRun(c: int, f: str): bool = {
    val x = s.cs.get(c)
    all {
      x.st == "verdict", aboveCap(x), x.second == "none", x.chal == "none", x.alloc == "none",
      s' = ghost({ ...s, cs: withC(c,
        if (f == x.facts) { ...x, second: "agree" }
        else if (f == "Infra") x
        else { ...x, st: "verifying", facts: "", tr: s.now }) }),
    }
  }

  // ---- challenges: settled by re-execution, never by a person
  action Challenge(c: int, who: str): bool = {
    val x = s.cs.get(c)
    all {
      x.st == "verdict", x.facts != "Infra", x.chal == "none", x.alloc == "none",
      not(isFinal(x)), s.now <= x.tv + WIN,
      s' = ghost({ ...s,
        cs: withC(c, { ...x, chal: "open", tch: s.now, chalBy: who }),
        cbondIn: s.cbondIn + CBOND, cbondLocked: s.cbondLocked + CBOND,
      }),
    }
  }

  // A run of the same input whose quote is verified: equal facts uphold the verdict (the challenger's
  // bond goes to the other side), different facts replace it (the bond is refunded).
  action ResolveT1(c: int): bool = {
    val x = s.cs.get(c)
    val upheld = factsOf(x.truth) == x.facts
    val toEscrow = upheld and x.chalBy == "hunter"
    val toHunter = upheld and x.chalBy != "hunter"
    all {
      x.st == "verdict", x.chal == "open",
      s' = ghost({ ...s,
        cs: withC(c,
          if (upheld) { ...x, chal: "none", tier: "T1", second: "agree" }
          else { ...x, chal: "none", facts: factsOf(x.truth), tier: "T1", tv: s.now, second: "none" }),
        cbondLocked: s.cbondLocked - CBOND,
        cbondToEscrow: s.cbondToEscrow + (if (toEscrow) CBOND else 0),
        cbondToHunter: s.cbondToHunter + (if (toHunter) CBOND else 0),
        cbondRefunded: s.cbondRefunded + (if (upheld) 0 else CBOND),
        funded: s.funded + (if (toEscrow) CBOND else 0),
        available: s.available + (if (toEscrow) CBOND else 0),
        credits: s.credits + (if (toEscrow) 0 else CBOND),
      }),
    }
  }

  // No T1 result in time: nobody wins. The verdict is void and the commit retries with its priority.
  action ChallengeTimeout(c: int): bool = {
    val x = s.cs.get(c)
    all {
      x.st == "verdict", x.chal == "open", s.now > x.tch + RESOLVE,
      s' = ghost({ ...s,
        cs: withC(c, { ...x, st: "verifying", facts: "", chal: "none", tr: s.now }),
        cbondLocked: s.cbondLocked - CBOND, cbondRefunded: s.cbondRefunded + CBOND, credits: s.credits + CBOND,
      }),
    }
  }

  // ---- allocation, strictly in commit order over final verdicts
  action Allocate = {
    val c = s.cursor
    val x = s.cs.get(c)
    val r = x.round
    val k = keyOf(x.facts)
    val sl = SLOT.get(k)
    val room = BUDGET - s.spent.get((r, sl))
    val amt = mn(AMT.get(k), room)
    val blocked = earlierDebt(c)
    all {
      canAllocate,
      if (x.st == "done")
        s' = ghost({ ...s, cursor: c + 1 })
      else if (VIOL.contains(x.facts) and knownKeys.contains(k))
        // A proof of a known issue: bond refunded, nothing paid.
        s' = ghost({ ...s, cursor: c + 1,
          cs: withC(c, { ...x, st: "done", end: "known" }),
          bondLocked: s.bondLocked - BOND, bondRefunded: s.bondRefunded + BOND, credits: s.credits + BOND })
      else if (VIOL.contains(x.facts) and s.holder.get((r, k)) == -1) {
        // The first final proof of a key holds it: reserve its payout, or queue the shortfall as debt.
        val resv = if (blocked) 0 else mn(amt, s.available)
        s' = ghost({ ...s, cursor: c + 1,
          cs: withC(c, { ...x, alloc: "holder", amount: amt, res: resv, debt: amt - resv,
                         st: if (amt == 0) "done" else "verdict", end: if (amt == 0) "exhausted" else "" }),
          holder: s.holder.set((r, k), c),
          spent: s.spent.set((r, sl), s.spent.get((r, sl)) + amt),
          available: s.available - resv, reserved: s.reserved + resv,
          bondLocked: s.bondLocked - BOND, bondRefunded: s.bondRefunded + BOND, credits: s.credits + BOND,
          firstClaim: if (s.firstClaim.get((r, k)) == -1) s.firstClaim.set((r, k), s.now) else s.firstClaim })
      } else if (VIOL.contains(x.facts)) {
        // A later proof of a held key: a duplicate. Nothing paid; the key's first duplicate takes the
        // one promotion reservation (only one duplicate per key can be promoted), in commit order, so a
        // promotion pays at the original priority and copies cannot lock the escrow.
        val dupHeld = CIDS.exists(d => d < s.n and s.cs.get(d).round == r and s.cs.get(d).alloc == "dup"
                                       and s.cs.get(d).end == "" and keyOf(s.cs.get(d).facts) == k and s.cs.get(d).res > 0)
        val resv = if (blocked or dupHeld) 0 else mn(AMT.get(k), s.available)
        s' = ghost({ ...s, cursor: c + 1,
          cs: withC(c, { ...x, alloc: "dup", of: s.holder.get((r, k)), amount: AMT.get(k), res: resv }),
          available: s.available - resv, reserved: s.reserved + resv,
          bondLocked: s.bondLocked - BOND, bondRefunded: s.bondRefunded + BOND, credits: s.credits + BOND })
      } else if (x.facts == "Gap") {
        // Gap lane: harm on a protected account but no invariant broken. Bond refunded, sealed claim. A
        // claim is keyed only once a round adds its rule, so one gap reservation per round.
        val gapHeld = CIDS.exists(d => d < s.n and s.cs.get(d).round == r and s.cs.get(d).alloc == "gap"
                                       and s.cs.get(d).end == "" and s.cs.get(d).res > 0)
        val resv = if (blocked or gapHeld) 0 else mn(GAPAMT, s.available)
        s' = ghost({ ...s, cursor: c + 1,
          cs: withC(c, { ...x, alloc: "gap", amount: GAPAMT, res: resv }),
          available: s.available - resv, reserved: s.reserved + resv,
          bondLocked: s.bondLocked - BOND, bondRefunded: s.bondRefunded + BOND, credits: s.credits + BOND })
      } else if (x.facts == "NoV" or x.facts == "Inv")
        // Final rejection: the bond goes to the program's escrow.
        s' = ghost({ ...s, cursor: c + 1,
          cs: withC(c, { ...x, st: "done", end: "forfeit" }),
          bondLocked: s.bondLocked - BOND, bondForfeited: s.bondForfeited + BOND,
          funded: s.funded + BOND, available: s.available + BOND })
      else
        // Unevaluable (default policy: refund) or ProgramMisconfigured (refund, program closes).
        s' = ghost({ ...s, cursor: c + 1, misconfigured: s.misconfigured or x.facts == "Mis",
          cs: withC(c, { ...x, st: "done", end: "refund" }),
          bondLocked: s.bondLocked - BOND, bondRefunded: s.bondRefunded + BOND, credits: s.credits + BOND }),
    }
  }

  // ---- payments
  action Pay(c: int): bool = {
    val x = s.cs.get(c)
    val ep = s.now / IWIN
    val spentNow = if (ep == s.iEpoch) s.iSpent else 0
    val windowOk = s.now > x.tv + WIN
    val useInstant = instantOk(x) and not(windowOk)
    val fee = mn(x.amount * FEE_BPS / 10000, s.available)
    all {
      canPay(c),
      s' = ghost({ ...s,
        cs: withC(c, { ...x, st: "done", end: "paid", res: 0 }),
        reserved: s.reserved - x.amount, paid: s.paid + x.amount,
        available: s.available - fee, fees: s.fees + fee, credits: s.credits + x.amount + fee,
        iEpoch: if (useInstant) ep else s.iEpoch,
        iSpent: if (useInstant) spentNow + x.amount else s.iSpent,
        payLog: s.payLog.union(Set((x.round, keyOf(x.facts), c))),
        instLog: if (useInstant) s.instLog.union(Set((ep, c, x.amount, x.tier))) else s.instLog,
        wrapped: s.wrapped.union(Set((x.round, keyOf(x.facts)))),
      }),
    }
  }

  // A promoted duplicate or gap claim pays once its claim is funded.
  action PayPromoted(c: int): bool = {
    val x = s.cs.get(c)
    val fee = mn(x.amount * FEE_BPS / 10000, s.available)
    all {
      x.alloc == "promoted", x.end == "", x.debt == 0, x.res == x.amount,
      s' = ghost({ ...s,
        cs: withC(c, { ...x, st: "done", end: "paid", res: 0 }),
        reserved: s.reserved - x.amount, paid: s.paid + x.amount,
        available: s.available - fee, fees: s.fees + fee, credits: s.credits + x.amount + fee,
        wrapped: if (VIOL.contains(x.facts)) s.wrapped.union(Set((x.round, keyOf(x.facts)))) else s.wrapped,
      }),
    }
  }

  // The shortfall queue: what becomes available funds debts in commit order. Never a pro-rata cut.
  action SettleDebt(c: int): bool = {
    val x = s.cs.get(c)
    val m = mn(x.debt, s.available)
    all {
      x.debt > 0, m > 0, not(earlierDebt(c)),
      s' = ghost({ ...s,
        cs: withC(c, { ...x, res: x.res + m, debt: x.debt - m }),
        available: s.available - m, reserved: s.reserved + m,
      }),
    }
  }

  action TopUp = all {
    s.phase == "open", not(s.misconfigured), s.topups < 2 * TOPUP,
    s' = ghost({ ...s, funded: s.funded + TOPUP, available: s.available + TOPUP, topups: s.topups + TOPUP }),
  }

  action Withdraw(v: int): bool = all {
    v > 0, v <= s.credits,
    s' = ghost({ ...s, credits: s.credits - v, withdrawn: s.withdrawn + v }),
  }

  // ---- pause permits: T1-confirmed holders of keys in the pause policy, at most one per round
  action PauseTicket(c: int): bool = {
    val x = s.cs.get(c)
    all {
      canTicket(c),
      s' = ghost({ ...s, tickets: s.tickets.set(x.round, s.tickets.get(x.round) + 1),
                         ticketLog: s.ticketLog.union(Set(c)) }),
    }
  }

  // ---- rounds, immune memory and the one-way appeal
  action ProposeRound(f: Set[str], addsInv: bool, canaries: bool): bool = all {
    s.phase == "open", not(s.misconfigured), not(s.proposed), s.round < MAXR, s.now < DEADLINE,
    addsInv implies not(s.slots.contains(GAPSLOT)),   // a round only adds new invariants
    s' = ghost({ ...s, proposed: true, fix: f, addsInv: addsInv, canaries: canaries, proposedAt: s.now }),
  }

  // Immune memory and appeals: permissionless re-runs, at the requester's cost, at the proposed fix:
  // the round's first findings, duplicates and gap claims, every paid PoC, and every commit that ever
  // held a key. A re-run counts for its own proposal only.
  action Rerun(c: int): bool = {
    val x = s.cs.get(c)
    all {
      s.phase == "open", not(s.misconfigured), s.proposed, s.now <= s.proposedAt + RERUN_WIN, x.rerun == "none",
      or {
        x.round == s.round and (x.alloc == "holder" or ((x.alloc == "dup" or x.alloc == "gap") and x.end == "")),
        x.end == "paid",
        everHeld(c),
      },
      s' = ghost({ ...s, cs: withC(c, { ...x, rerun: afterFix(x) }) }),
    }
  }

  action OpenRound = {
    val r = s.round
    val r1 = r + 1
    // A held key is released when its holder's re-run shows it fixed, or when nobody re-ran it.
    val released = KEYS.filter(k => s.holder.get((r, k)) != -1 and s.cs.get(s.holder.get((r, k))).rerun != "breaks")
    val carried = KEYS.filter(k => s.holder.get((r, k)) != -1 and s.cs.get(s.holder.get((r, k))).rerun == "breaks")
    val fixedNow = KEYS.filter(k => s.holder.get((r, k)) != -1 and s.cs.get(s.holder.get((r, k))).rerun == "fixed")
    // The earliest duplicate of a released key that still breaks it is promoted.
    def promoteOf(k: str): int =
      CIDS.filter(d => d < s.n and s.cs.get(d).round == r and s.cs.get(d).alloc == "dup" and s.cs.get(d).end == ""
                       and keyOf(s.cs.get(d).facts) == k and s.cs.get(d).rerun == "breaks")
          .fold(-1, (a, d) => if (a == -1 or d < a) d else a)
    val promoted = released.filter(k => promoteOf(k) != -1).map(k => promoteOf(k))
    // The earliest gap claim that breaks the added rule is promoted; later copies of it are its duplicates.
    val gapBreaking = if (s.addsInv)
        CIDS.filter(d => d < s.n and s.cs.get(d).round == r and s.cs.get(d).alloc == "gap" and s.cs.get(d).end == ""
                         and s.cs.get(d).rerun == "breaks")
      else Set()
    val gapFirst = gapBreaking.fold(-1, (a, d) => if (a == -1 or d < a) d else a)
    val gapPromoted = gapBreaking.filter(d => d == gapFirst)
    val gate = releaseGate
    // Promotions share their slot's budget in the new round, in commit order, as the engine allocates
    // them: a second promotion into a slot gets what the first left.
    val promoAmt = range(0, NC).foldl(Map(), (acc, d) =>
      if (promoted.contains(d)) {
        val sl = SLOT.get(keyOf(s.cs.get(d).facts))
        val used = acc.keys().filter(e => SLOT.get(keyOf(s.cs.get(e).facts)) == sl).fold(0, (a, e) => a + acc.get(e))
        val room = BUDGET - s.spent.get((r1, sl)) - used
        acc.put(d, mn(s.cs.get(d).amount, if (room > 0) room else 0))
      } else acc)
    // New state of every commit of round r that is still waiting on the round.
    def nextC(d: int): Commit = {
      val x = s.cs.get(d)
      if (promoted.contains(d)) {
        val amt = promoAmt.get(d)
        val use = mn(x.res, amt)
        { ...x, alloc: "promoted", amount: amt, res: use, debt: amt - use }
      } else if (gapPromoted.contains(d)) {
        { ...x, alloc: "promoted", amount: GAPAMT, res: mn(x.res, GAPAMT), debt: GAPAMT - mn(x.res, GAPAMT) }
      } else if (gapBreaking.contains(d)) {
        { ...x, st: "done", end: "dupconf", res: 0 }
      } else if (x.round == r and (x.alloc == "dup" or x.alloc == "gap") and x.end == "")
        { ...x, st: "done", end: if (x.alloc == "dup") "dupconf" else "gapopen", res: 0 }
      else x
    }
    // Reservations released by confirmed duplicates, gap claims left open, and promotions above amount.
    val freed = CIDS.fold(0, (a, d) => a + s.cs.get(d).res - nextC(d).res)
    val promotedKeys = promoted.map(d => keyOf(s.cs.get(d).facts))
    all {
      s.phase == "open", not(s.misconfigured), s.proposed, s.round < MAXR, s.now >= s.roundStart + ROUND_MIN,
      s.now >= s.proposedAt + RERUN_WIN,
      // nothing of round r may still decide a key: the allocation cursor has passed every commit (closed
      // ones too: `allocate` steps over them), the engine's constant-time check
      s.cursor >= s.n,
      s' = ghost({ ...s,
        round: r1, roundStart: s.now, proposed: false, fix: Set(),
        cs: CIDS.mapBy(d => { ...nextC(d), rerun: "none" }),
        holder: s.holder.keys().mapBy(rk =>
          if (rk._1 != r1) s.holder.get(rk)
          else if (carried.contains(rk._2)) s.holder.get((r, rk._2))
          else if (promotedKeys.contains(rk._2)) promoteOf(rk._2)
          else -1),
        spent: s.spent.keys().mapBy(rs =>
          if (rs._1 != r1) s.spent.get(rs)
          else s.spent.get(rs)
               + promoted.filter(d => SLOT.get(keyOf(s.cs.get(d).facts)) == rs._2).fold(0, (a, d) => a + nextC(d).amount)
               + (if (rs._2 == GAPSLOT) gapPromoted.size() * GAPAMT else 0)),
        available: s.available + freed, reserved: s.reserved - freed,
        slots: if (s.addsInv) s.slots.union(Set(GAPSLOT)) else s.slots,
        // a key counts as fixed, for disclosure, only when the fix ships (the release gate holds)
        fixedKeys: if (gate) s.fixedKeys.union(fixedNow.map(k => (r, k))) else s.fixedKeys,
        permits: s.permits + (if (gate) 1 else 0),
        permitLog: s.permitLog.union(Set((r, gate))),
        payLog: s.payLog.union(promoted.map(d => (r1, keyOf(s.cs.get(d).facts), d))),
        // a promotion needs a released key and a re-run that still breaks it
        promoteOk: s.promoteOk and promoted.forall(d => released.contains(keyOf(s.cs.get(d).facts))
                                                         and s.cs.get(d).rerun == "breaks"),
      }),
    }
  }

  // ---- disclosure: permissionless once its condition holds
  action Disclose(dr: int, k: str): bool = all {
    not(s.disclosed.contains((dr, k))), discloseCond((dr, k)),
    s' = ghost({ ...s, disclosed: s.disclosed.union(Set((dr, k))) }),
  }

  // ---- the exit: after the deadline and its grace, nothing can stay stuck
  action Exit = {
    // Duplicates and gap claims can no longer be promoted: their reservations go back first, so they
    // fund the owed claims' debts before anything returns to the project.
    def openClaim(d: int): bool = {
      val x = s.cs.get(d)
      x.st != "free" and x.st != "done" and (x.alloc == "dup" or x.alloc == "gap") and x.end == ""
    }
    val freedFirst = CIDS.filter(d => openClaim(d)).fold(0, (a, d) => a + s.cs.get(d).res)
    // Then debts, in commit order, from what is available.
    val settled = range(0, NC).foldl((s.available + freedFirst, CIDS.mapBy(d => if (openClaim(d)) { ...s.cs.get(d), res: 0 } else s.cs.get(d))), (acc, d) => {
      val x = acc._2.get(d)
      val owed = (x.alloc == "holder" or x.alloc == "promoted") and x.end == "" and x.st != "done"
      val m = if (owed) mn(x.debt, acc._1) else 0
      (acc._1 - m, acc._2.set(d, { ...x, res: x.res + m, debt: x.debt - m }))
    })
    val avail1 = settled._1
    val cs1 = settled._2
    def fin(d: int): Commit = {
      val x = cs1.get(d)
      if (x.st == "free" or x.st == "done") x
      else if (x.st == "committed") { ...x, st: "done", end: "expired" }
      else if ((x.alloc == "holder" or x.alloc == "promoted") and x.end == "")
        { ...x, st: "done", end: if (x.debt == 0) "paid" else "defaulted", res: 0 }
      else if (x.alloc == "dup") { ...x, st: "done", end: "dupconf", res: 0 }
      else if (x.alloc == "gap") { ...x, st: "done", end: "gapopen", res: 0 }
      else { ...x, st: "done", end: "void", chal: "none" }
    }
    def isOwedPaid(d: int): bool = {
      val x = cs1.get(d)
      x.st != "free" and x.st != "done" and (x.alloc == "holder" or x.alloc == "promoted") and x.end == "" and x.debt == 0
    }
    def isOwed(d: int): bool = {
      val x = cs1.get(d)
      x.st != "free" and x.st != "done" and (x.alloc == "holder" or x.alloc == "promoted") and x.end == ""
    }
    // Owed claims are paid what is reserved for them (in full, or in part when the escrow never covered
    // them: the rest is recorded as defaulted). Never a pro-rata cut: debts were funded in commit order.
    val payNow = CIDS.filter(d => isOwed(d)).fold(0, (a, d) => a + cs1.get(d).res)
    val released = CIDS.filter(d => cs1.get(d).st != "done" and cs1.get(d).st != "free" and not(isOwed(d)))
                       .fold(0, (a, d) => a + cs1.get(d).res)
    val expired = CIDS.filter(d => cs1.get(d).st == "committed").size()
    val refunded = CIDS.filter(d => cs1.get(d).st == "verifying" or (cs1.get(d).st == "verdict" and cs1.get(d).alloc == "none")).size()
    val openChal = CIDS.filter(d => cs1.get(d).st == "verdict" and cs1.get(d).chal == "open").size()
    val ret = avail1 + released + expired * BOND
    all {
      canExit,
      s' = ghost({ ...s, phase: "closed", pinsAtOpen: if (s.phase == "setup") s.pins else s.pinsAtOpen, cursor: s.n,
        cs: CIDS.mapBy(d => fin(d)),
        reserved: 0, available: 0,
        paid: s.paid + payNow, returned: s.returned + ret,
        funded: s.funded + expired * BOND,
        bondLocked: s.bondLocked - (expired + refunded) * BOND,
        bondForfeited: s.bondForfeited + expired * BOND,
        bondRefunded: s.bondRefunded + refunded * BOND,
        cbondLocked: s.cbondLocked - openChal * CBOND, cbondRefunded: s.cbondRefunded + openChal * CBOND,
        credits: s.credits + payNow + ret + refunded * BOND + openChal * CBOND,
        payLog: s.payLog.union(CIDS.filter(d => isOwed(d) and cs1.get(d).alloc == "holder" and cs1.get(d).res > 0)
                                   .map(d => (cs1.get(d).round, keyOf(cs1.get(d).facts), d))),
      }),
    }
  }

  // ================================================================ step
  action step = any {
    Tick,
    nondet v = 0.to(2).oneOf() OwnerSetPins(v),
    nondet b = BUGS.oneOf() DeclareKnown(b),
    Open,
    nondet h = HUNTERS.oneOf() MakeCommit(h),
    nondet c = CIDS.oneOf() nondet t = TRUTHS.oneOf() Reveal(c, t),
    nondet c = CIDS.oneOf() Expire(c),
    nondet c = CIDS.oneOf() nondet f = FACTS.oneOf() FactsT0(c, f),
    nondet c = CIDS.oneOf() FactsT1(c),
    nondet c = CIDS.oneOf() ConfirmT1(c),
    nondet c = CIDS.oneOf() Retry(c),
    nondet c = CIDS.oneOf() InfraTimeout(c),
    nondet c = CIDS.oneOf() nondet f = FACTS.oneOf() SecondRun(c, f),
    nondet c = CIDS.oneOf() nondet w = CHALLENGERS.oneOf() Challenge(c, w),
    nondet c = CIDS.oneOf() ResolveT1(c),
    nondet c = CIDS.oneOf() ChallengeTimeout(c),
    Allocate,
    nondet c = CIDS.oneOf() Pay(c),
    nondet c = CIDS.oneOf() PayPromoted(c),
    nondet c = CIDS.oneOf() SettleDebt(c),
    TopUp,
    nondet v = 1.to(3).oneOf() Withdraw(v),
    nondet c = CIDS.oneOf() PauseTicket(c),
    nondet f = BUGS.powerset().oneOf() nondet a = Set(true, false).oneOf() nondet b = Set(true, false).oneOf() ProposeRound(f, a, b),
    nondet c = CIDS.oneOf() Rerun(c),
    OpenRound,
    nondet dr = ROUNDS.oneOf() nondet k = KEYS.oneOf() Disclose(dr, k),
    Exit,
  }

  // ================================================================ properties (docs/properties.md)
  // 1. Escrow conservation, for the program and for every token near-antibody holds.
  val p1_escrow = and {
    s.funded == ESCROW0 + s.topups + s.bondForfeited + s.cbondToEscrow,
    s.funded == s.available + s.reserved + s.paid + s.fees + s.returned,
    s.reserved == CIDS.fold(0, (a, c) => a + s.cs.get(c).res),
    s.available >= 0, s.reserved >= 0,
  }
  val p1_tokens =
    ESCROW0 + s.topups + s.bondIn + s.cbondIn - s.withdrawn
      == s.available + s.reserved + s.bondLocked + s.cbondLocked + s.credits

  // 2. At most one payment per key per round, within each invariant's budget.
  val p2_onePerKey = s.payLog.forall(a => s.payLog.forall(b => (a._1 == b._1 and a._2 == b._2) implies a._3 == b._3))
  val p2_budget = s.spent.keys().forall(rs => s.spent.get(rs) <= BUDGET)

  // 3. A key goes to the earliest valid commit: a duplicate always duplicates an earlier holder, and no
  //    holder is later than a duplicate of its key in the same round.
  val p3_priority = and {
    CIDS.forall(c => s.cs.get(c).alloc == "dup" implies (s.cs.get(c).of >= 0 and s.cs.get(c).of < c)),
    CIDS.forall(a => CIDS.forall(b => (a < b and s.cs.get(a).alloc == "dup" and s.cs.get(b).alloc == "holder"
      and s.cs.get(a).round == s.cs.get(b).round and keyOf(s.cs.get(a).facts) == keyOf(s.cs.get(b).facts)) implies false)),
  }

  // 4. Duplicates are never paid, except through the one-way appeal.
  val p4_dups = and {
    CIDS.forall(c => s.cs.get(c).alloc == "dup" implies s.cs.get(c).end != "paid"),
    s.promoteOk,
  }

  // 5. A round never removes a payment or an invariant.
  val p5_monotone = s.paid >= s.prevPaid and s.prevSlots.subseteq(s.slots)

  // 6. Every in-flight state reaches a terminal state: the exit is never blocked, and after it nothing
  //    is in flight and nothing is locked.
  val p6_exit = and {
    (s.now >= DEADLINE + EXIT_GRACE and s.phase != "closed") implies exitEnabled,
    s.phase == "closed" implies and {
      CIDS.forall(c => s.cs.get(c).st == "free" or s.cs.get(c).st == "done"),
      s.bondLocked == 0, s.cbondLocked == 0, s.reserved == 0, s.available == 0,
    },
  }

  // 7. Bonds are never lost: refunded, or forfeited to the program's escrow.
  val p7_bonds = and {
    s.bondIn == s.bondLocked + s.bondRefunded + s.bondForfeited,
    s.cbondIn == s.cbondLocked + s.cbondToHunter + s.cbondToEscrow + s.cbondRefunded,
  }

  // 8. Instant payments happen only with T1, below the cap, and within the budget of their window.
  val p8_instant = and {
    s.instLog.forall(e => e._4 == "T1" and e._3 <= CAP),
    0.to(TMAX / IWIN).forall(ep => s.instLog.filter(e => e._1 == ep).fold(0, (a, e) => a + e._3) <= IBUDGET),
  }

  // 9. No known issue is added after open, and known issues stay within their cap.
  val p9_known = and {
    not(s.knownAfterOpen),
    s.known.size() <= MAX_KNOWN,
    s.known.fold(0, (a, b) => a + AMT.get(keyOf(factsOf(b)))) <= KNOWN_SHARE,
  }

  // 10. Disclosure cannot be blocked once its condition holds, and never happens before it.
  //     The condition depends on no one's consent and, once true, stays true; `Disclose` is permissionless.
  val p10_disclosure = and {
    s.prevCond.forall(rk => discloseCond(rk)),
    s.disclosed.forall(rk => discloseCond(rk)),
  }

  // 11. Permits follow the committed policy: pause tickets only for T1 holders of policy keys, at most
  //     one per round; release permits only when the release gate held.
  val p11_permits = and {
    ROUNDS.forall(r => s.tickets.get(r) <= 1),
    s.ticketLog.forall(c => s.cs.get(c).tier == "T1" and PAUSE_KEYS.contains(keyOf(s.cs.get(c).facts))
                            and (s.cs.get(c).alloc == "holder" or s.cs.get(c).alloc == "promoted")),
    s.permits == s.permitLog.filter(e => e._2).size(),
  }

  // 12. The owner cannot change a live program.
  val p12_pins = s.phase != "setup" implies s.pins == s.pinsAtOpen

  val all_properties = and {
    p1_escrow, p1_tokens, p2_onePerKey, p2_budget, p3_priority, p4_dups, p5_monotone, p6_exit,
    p7_bonds, p8_instant, p9_known, p10_disclosure, p11_permits, p12_pins,
  }


  // ================================================================ scenarios (quint test)
  run instantPayoutTest =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "VA1")).then(FactsT1(0))
      .then(Allocate).then(Pay(0))
      .expect(and { s.paid == 200, s.instLog.size() == 1, all_properties })
      .then(PauseTicket(0))
      .expect(s.tickets.get(0) == 1)
      .expect(not(canTicket(0)))

  run t0WaitsWindowTest =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "VA1")).then(FactsT0(0, "VA"))
      .expect(not(canAllocate))
      .then(3.reps(_ => Tick))
      .then(Allocate).then(Pay(0))
      .expect(and { s.paid == 200, s.instLog.size() == 0, all_properties })
      // a T0 verdict cannot mint a pause ticket
      .expect(not(canTicket(0)))

  run instantBudgetTest =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "VA1")).then(FactsT1(0))
      .then(Allocate).then(Pay(0))
      .expect(and { s.iSpent == 200, s.instLog.size() == 1 })
      // the same window has 100 left: a second instant payment of 200 would not fit
      .expect(instantOk({ ...FREE, tier: "T1", amount: 100 }) and not(instantOk({ ...FREE, tier: "T1", amount: 200 })))
      .then(4.reps(_ => Tick))                       // the next window resets the budget
      .expect(instantOk({ ...FREE, tier: "T1", amount: 200 }))

  run aboveCapNeedsSecondRunTest =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "VC1")).then(FactsT1(0))
      .expect(not(canAllocate))                       // T1 above the cap is not final at once
      .then(3.reps(_ => Tick))
      .expect(not(canAllocate))                       // the window passed, but no second run agreed
      .then(SecondRun(0, "VC"))
      .then(Allocate).then(Pay(0))
      .expect(and { s.paid == 400, s.instLog.size() == 0, all_properties })

  run disagreeingSecondRunVoidsTest =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "NoV")).then(FactsT0(0, "VC"))
      .then(SecondRun(0, "NoV"))
      .expect(s.cs.get(0).st == "verifying")      // void: retried, priority kept
      .then(FactsT1(0)).then(Allocate)
      .expect(and { s.cs.get(0).end == "forfeit", s.paid == 0, all_properties })

  run shortfallQueueTest =
    init.then(Open)
      .then(MakeCommit("h1")).then(MakeCommit("h2")).then(MakeCommit("h1"))
      .then(Reveal(0, "VA1")).then(Reveal(1, "VA2")).then(Reveal(2, "VC1"))
      .then(FactsT1(0)).then(FactsT1(1)).then(FactsT1(2))
      .then(Allocate).then(Allocate)               // holder of A (200), duplicate of A (reserves 200)
      .then(SecondRun(2, "VC")).then(3.reps(_ => Tick))
      .then(Allocate)                              // C: 400 owed, 100 available
      .expect(and { s.cs.get(2).res == 100, s.cs.get(2).debt == 300, all_properties })
      .expect(not(canPay(2)))                         // never a pro-rata cut
      .then(TopUp).then(SettleDebt(2))
      .expect(s.cs.get(2).debt == 100)
      .then(TopUp).then(SettleDebt(2)).then(Pay(2))
      .expect(and { s.cs.get(2).end == "paid", s.paid == 400, all_properties })

  // Copies of a held proof (their bonds refunded) take one promotion reservation per key, so they cannot
  // lock the escrow against later holders.
  run copiesTakeOneReservationTest =
    init.then(Open)
      .then(MakeCommit("h1")).then(MakeCommit("h2")).then(MakeCommit("h2"))
      .then(Reveal(0, "VA1")).then(Reveal(1, "VA1")).then(Reveal(2, "VA1"))
      .then(FactsT1(0)).then(FactsT1(1)).then(FactsT1(2))
      .then(Allocate).then(Allocate).then(Allocate)
      .expect(and { s.cs.get(1).alloc == "dup", s.cs.get(1).res == 200, s.cs.get(2).alloc == "dup", s.cs.get(2).res == 0,
                    all_properties })

  // At exit, an open duplicate's reservation goes back before debts are funded: it pays an owed claim,
  // not the project.
  run exitFundsDebtsBeforeDuplicatesTest =
    init.then(Open)
      .then(MakeCommit("h1")).then(MakeCommit("h2")).then(MakeCommit("h1"))
      .then(Reveal(0, "VA1")).then(Reveal(1, "VA2")).then(Reveal(2, "VC1"))
      .then(FactsT1(0)).then(FactsT1(1)).then(FactsT1(2))
      .then(Allocate).then(Allocate)
      .then(SecondRun(2, "VC")).then(3.reps(_ => Tick))
      .then(Allocate).then(Pay(0))
      .expect(and { s.cs.get(1).res == 200, s.cs.get(2).debt == 300 })
      .then(9.reps(_ => Tick)).then(Exit)
      .expect(and { s.paid == 500, s.cs.get(2).end == "defaulted", s.cs.get(1).end == "dupconf", s.returned == 0,
                    all_properties })

  run challengeOverturnsLyingT0Test =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "VA1"))
      .then(FactsT0(0, "NoV"))                     // a lying operator rejects a real finding
      .then(Challenge(0, "hunter")).then(ResolveT1(0))
      .expect(and { s.cs.get(0).facts == "VA", s.cs.get(0).tier == "T1", s.cbondRefunded == CBOND })
      .then(Allocate).then(Pay(0))
      .expect(and { s.paid == 200, s.bondForfeited == 0, all_properties })

  run challengeUpheldPaysHunterTest =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "VA1")).then(FactsT0(0, "VA"))
      .then(Challenge(0, "project")).then(ResolveT1(0))
      .expect(s.cbondToHunter == CBOND)
      .then(Allocate).then(Pay(0))
      .expect(and { s.paid == 200, all_properties })

  run challengeTimeoutKeepsPriorityTest =
    init.then(Open).then(MakeCommit("h1")).then(MakeCommit("h2"))
      .then(Reveal(0, "VA1")).then(Reveal(1, "VA2"))
      .then(FactsT0(0, "VA")).then(FactsT1(1))
      .then(Challenge(0, "project"))
      .expect(not(canAllocate))                       // commit 1 waits behind commit 0
      .then(3.reps(_ => Tick)).then(ChallengeTimeout(0))
      .expect(and { s.cs.get(0).st == "verifying", s.cbondRefunded == CBOND })
      .then(FactsT1(0)).then(Allocate).then(Allocate)
      .expect(and { s.cs.get(0).alloc == "holder", s.cs.get(1).alloc == "dup", all_properties })

  run appealPromotesDistinctBugTest =
    init.then(Open).then(MakeCommit("h1")).then(MakeCommit("h2"))
      .then(Reveal(0, "VA1")).then(Reveal(1, "VA2")).then(FactsT1(0)).then(FactsT1(1))
      .then(Allocate).then(Allocate).then(Pay(0))
      .expect(s.cs.get(1).alloc == "dup")
      .then(ProposeRound(Set("VA1"), false, true))  // the fix stops bug 1, not bug 2
      .then(Rerun(0)).then(Rerun(1))
      .expect(and { s.cs.get(0).rerun == "fixed", s.cs.get(1).rerun == "breaks" })
      .then(3.reps(_ => Tick)).then(OpenRound)
      .expect(and { s.cs.get(1).alloc == "promoted", s.holder.get((1, "A")) == 1 })
      .then(PayPromoted(1))
      .expect(and { s.paid == 400, s.payLog.contains((1, "A", 1)), s.permits == 0, all_properties })

  // Duplicates of two keys that share a slot (B and C) are promoted in the same round: they share the
  // slot's budget in commit order, as the engine allocates them, so the second gets what the first left.
  run promotionsShareTheirSlotBudgetTest =
    init.then(Open).then(TopUp).then(TopUp)
      .then(MakeCommit("h1")).then(MakeCommit("h2")).then(MakeCommit("h1")).then(MakeCommit("h2"))
      .then(Reveal(0, "VB1")).then(Reveal(1, "VC1")).then(Reveal(2, "VB2")).then(Reveal(3, "VC2"))
      .then(FactsT1(0)).then(FactsT1(1)).then(FactsT1(2)).then(FactsT1(3))
      // B and C are above the instant cap: an agreeing second run each, then their window
      .then(SecondRun(0, "VB")).then(SecondRun(1, "VC")).then(SecondRun(2, "VB")).then(SecondRun(3, "VC"))
      .then(3.reps(_ => Tick))
      .then(Allocate).then(Allocate).then(Allocate).then(Allocate)
      .expect(and { s.cs.get(2).alloc == "dup", s.cs.get(3).alloc == "dup" })
      .then(Pay(0)).then(Pay(1))
      .then(ProposeRound(Set("VB1", "VC1"), false, true))  // the fix stops the holders' bugs, not the copies'
      .then(Rerun(0)).then(Rerun(1)).then(Rerun(2)).then(Rerun(3))
      .then(3.reps(_ => Tick)).then(OpenRound)
      .expect(and { s.cs.get(2).alloc == "promoted", s.cs.get(3).alloc == "promoted",
                    s.cs.get(2).amount == 400, s.cs.get(3).amount == BUDGET - 400, s.spent.get((1, 1)) == BUDGET,
                    all_properties })

  run confirmedDuplicateReleasesReservationTest =
    init.then(Open).then(MakeCommit("h1")).then(MakeCommit("h2"))
      .then(Reveal(0, "VA1")).then(Reveal(1, "VA1")).then(FactsT1(0)).then(FactsT1(1))
      .then(Allocate).then(Allocate).then(Pay(0))
      .then(ProposeRound(Set("VA1"), false, true)).then(Rerun(0)).then(Rerun(1))
      .then(3.reps(_ => Tick)).then(OpenRound)
      .expect(and { s.cs.get(1).end == "dupconf", s.cs.get(1).res == 0, s.permits == 1, all_properties })

  run gapLanePromotedTest =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "Gap")).then(FactsT1(0)).then(Allocate)
      .expect(and { s.cs.get(0).alloc == "gap", s.bondRefunded == BOND })
      .then(ProposeRound(Set(), true, true)).then(Rerun(0))
      .then(3.reps(_ => Tick)).then(OpenRound)
      .then(PayPromoted(0))
      .expect(and { s.paid == GAPAMT, s.slots.contains(GAPSLOT), all_properties })

  run releaseGateBlocksUnfixedTest =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "VA1")).then(FactsT1(0))
      .then(Allocate).then(Pay(0))
      .then(ProposeRound(Set(), false, true)).then(Rerun(0))   // a "fix" that fixes nothing
      .then(3.reps(_ => Tick)).then(OpenRound)
      .expect(and { s.permits == 0, s.holder.get((1, "A")) == 0, all_properties })

  // A partial fix carries the key; the next round's fix is re-run and releases it.
  run laterFixReleasesCarriedKeyTest =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "VA1")).then(FactsT1(0))
      .then(Allocate).then(Pay(0))
      .then(ProposeRound(Set(), false, true)).then(Rerun(0))
      .then(3.reps(_ => Tick)).then(OpenRound)
      .expect(and { s.permits == 0, s.holder.get((1, "A")) == 0, s.cs.get(0).rerun == "none" })
      .then(ProposeRound(Set("VA1"), false, true)).then(Rerun(0))
      .then(3.reps(_ => Tick)).then(OpenRound)
      .expect(and { s.round == 2, s.permits == 1, s.holder.get((2, "A")) == -1, all_properties })

  // Immune memory: a later fix that brings a paid bug back is not released.
  run regressionBlocksReleaseTest =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "VA1")).then(FactsT1(0))
      .then(Allocate).then(Pay(0))
      .then(ProposeRound(Set("VA1"), false, true)).then(Rerun(0))
      .then(3.reps(_ => Tick)).then(OpenRound)
      .expect(s.permits == 1)
      .then(ProposeRound(Set(), false, true)).then(Rerun(0))   // the new code reintroduces bug 1
      .then(3.reps(_ => Tick)).then(OpenRound)
      .expect(and { s.round == 2, s.permits == 1, all_properties })

  run canariesBlockReleaseTest =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "VA1")).then(FactsT1(0))
      .then(Allocate).then(Pay(0))
      .then(ProposeRound(Set("VA1"), false, false)).then(Rerun(0))
      .then(3.reps(_ => Tick)).then(OpenRound)
      .expect(and { s.permits == 0, all_properties })

  run knownIssueTest =
    init
      .expect(not(canDeclareKnown("VB1")))            // over the cap (400 > 250)
      .then(DeclareKnown("VA1"))
      .then(Open)
      .expect(not(canDeclareKnown("VA2")))            // never after open
      .then(MakeCommit("h1")).then(Reveal(0, "VA2")).then(FactsT1(0)).then(Allocate)
      .expect(and { s.cs.get(0).end == "known", s.paid == 0, all_properties })

  run ownerCannotChangeLiveProgramTest =
    init.then(OwnerSetPins(1)).then(Open).then(OwnerSetPins(2))
      .expect(and { s.pins == 1, all_properties })

  run exitSettlesEverythingTest =
    init.then(Open).then(MakeCommit("h1")).then(MakeCommit("h2")).then(MakeCommit("h1"))
      .then(Reveal(1, "NoV")).then(Reveal(2, "VA1")).then(FactsT1(1)).then(FactsT1(2))
      .then(12.reps(_ => Tick))
      .expect(and { exitEnabled, not(canExit) })      // commit 0 was never revealed: it expires first
      .then(Expire(0)).then(Allocate).then(Allocate).then(Allocate).then(Exit)
      .expect(and { s.phase == "closed", s.bondLocked == 0, s.reserved == 0, s.cs.get(2).end == "paid",
                    s.cs.get(1).end == "forfeit", s.cs.get(0).end == "expired", all_properties })

  run misconfiguredProgramClosesTest =
    init.then(Open).then(MakeCommit("h1")).then(Reveal(0, "Mis")).then(FactsT1(0)).then(Allocate)
      .expect(not(canCommit))
      .then(Exit)
      .expect(and { s.phase == "closed", all_properties })


  run unexaminedDuplicateBlocksReleaseTest =
    init.then(Open).then(MakeCommit("h1")).then(MakeCommit("h2"))
      .then(Reveal(0, "VA1")).then(Reveal(1, "VA1")).then(FactsT1(0)).then(FactsT1(1))
      .then(Allocate).then(Allocate).then(Pay(0))
      .then(ProposeRound(Set("VA1"), false, true)).then(Rerun(0))   // nobody re-runs the duplicate
      .then(3.reps(_ => Tick)).then(OpenRound)
      .expect(and { s.permits == 0, s.cs.get(1).end == "dupconf", all_properties })

  // ================================================================ coverage witnesses
  val w_paid = s.paid > 0
  val w_instant = s.instLog.size() > 0
  val w_window_paid = s.payLog.size() > s.instLog.size()
  val w_promoted = CIDS.exists(c => s.cs.get(c).alloc == "promoted")
  val w_promoted_paid = CIDS.exists(c => s.cs.get(c).alloc == "promoted" and s.cs.get(c).end == "paid")
  val w_debt = CIDS.exists(c => s.cs.get(c).debt > 0)
  val w_debt_settled = CIDS.exists(c => s.cs.get(c).end == "paid") and s.topups > 0
  val w_dupconf = CIDS.exists(c => s.cs.get(c).end == "dupconf")
  val w_challenge_upheld = s.cbondToHunter > 0 or s.cbondToEscrow > 0
  val w_challenge_overturned = s.cbondRefunded > 0
  val w_forfeit = s.bondForfeited > 0
  val w_ticket = s.ticketLog.size() > 0
  val w_permit = s.permits > 0
  val w_round1 = s.round == 1
  val w_gap_promoted = CIDS.exists(c => s.cs.get(c).alloc == "promoted" and s.cs.get(c).facts == "Gap")
  val w_known = s.known.size() > 0 and CIDS.exists(c => s.cs.get(c).end == "known")
  val w_disclosed = s.disclosed.size() > 0
  val w_exhausted = CIDS.exists(c => s.cs.get(c).end == "exhausted")
  val w_closed_with_pay = s.phase == "closed" and s.paid > 0
}
