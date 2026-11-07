//! When a verdict may be paid, on NEAR's clock. near-antibody counts a verdict under the instant cap
//! final (`is_final`) once it is T1-confirmed or, still T0, once a block's time is past the answer plus
//! the program's challenge window. [`wait`] polls the run and the latest final block together and
//! returns at the first of the two: `antibody try` waits here between the verdict and `allocate`.
//!
//! It decides by final blocks, never by the local clock: a clock ahead of NEAR's would send `allocate`
//! before the claim is final, to read "nothing to allocate", and one behind would wait for nothing. The
//! local clock only paces the polls and bounds the wait.

use std::time::{Duration, Instant};

use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{cmd, ctx::Ctx};

/// How long the wait looks for T1 while the answer's time is unknown: the CLI's former T1 wait.
pub const T1_CAP_S: u64 = 300;

/// The rule that made a verdict payable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayPath {
    /// T1-confirmed: final at once.
    T1,
    /// Still T0: final once a final block is past the answer plus the challenge window.
    T0,
}

impl PayPath {
    pub fn as_str(self) -> &'static str {
        match self {
            PayPath::T1 => "T1",
            PayPath::T0 => "T0",
        }
    }
}

/// The wait's bounds.
#[derive(Clone, Debug)]
pub struct Limits {
    /// The program's challenge window, in seconds (`params.windows.challenge`).
    pub challenge_s: u64,
    /// How long to look for T1 while the answer's time is unknown. Past it the wait says it gave T1
    /// up, but it pays no earlier: a T0 payout still needs the answer's time and the window on NEAR's
    /// clock, and a T1 that lands later still pays at once.
    pub t1_cap_s: u64,
    /// The whole wait, on the local clock: past it, [`NotYet`].
    pub timeout_s: u64,
    /// Seconds between polls.
    pub poll_s: f64,
}

impl Limits {
    /// The cap, then the window, and two minutes for the blocks and the RPC.
    pub fn new(challenge_s: u64) -> Self {
        Self { challenge_s, t1_cap_s: T1_CAP_S, timeout_s: T1_CAP_S + challenge_s + 120, poll_s: 2.0 }
    }
}

/// What the wait reads, injected so that it can be tested without a network ([`NearWatch`] reads NEAR).
pub trait Watch {
    /// The run's verdict now, as [`cmd::run_view`] gives it.
    fn run(&mut self) -> Result<Value>;
    /// The latest final block's time, in whole seconds (near-antibody's `now`).
    fn final_secs(&mut self) -> Result<u64>;
    /// Seconds on the local clock, from any origin: only differences are used, to pace and bound the
    /// wait, never compared with NEAR's clock.
    fn local_secs(&self) -> f64;
    fn sleep(&mut self, secs: f64);
}

/// A verdict that may be paid now: `allocate` and `pay` take it.
#[derive(Clone, Debug)]
pub struct Payable {
    pub path: PayPath,
    /// The run then ([`cmd::run_view`]).
    pub run: Value,
    /// Why the verdict stayed T0, in words (the T0 path).
    pub note: Option<String>,
    /// The program's challenge window, in seconds.
    pub challenge_s: u64,
    /// The final block's time that allowed a T0 payout; on the T1 path, the last one read, if any.
    pub chain_secs: Option<u64>,
    /// Local seconds the wait took.
    pub waited_s: f64,
}

/// The wait's bound passed with neither rule met: the run is still verifying, or its challenge window
/// ends later. Not a failure, since the verdict may still be paid (the keeper allocates and pays final
/// verdicts): a caller that need not fail tells it apart with `report.downcast_ref::<NotYet>()`.
#[derive(Clone, Debug)]
pub struct NotYet {
    pub run_id: u64,
    pub run: Value,
    /// The answer's time plus the window: a T0 payout once a final block is past it (unknown until the
    /// run is answered).
    pub t0_after: Option<u64>,
    /// The latest final block's time read, if any.
    pub chain_secs: Option<u64>,
    pub waited_s: f64,
}

impl std::fmt::Display for NotYet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (id, waited, run) = (self.run_id, self.waited_s, &self.run);
        match (self.t0_after, self.chain_secs) {
            (Some(after), Some(chain)) => write!(
                f,
                "run {id} is not T1-confirmed after {waited:.0} s, and its challenge window has not ended on NEAR's clock \
                 (a T0 payout once a final block is past {after}; the latest is at {chain}): {run}"
            ),
            _ if run.get("pending").is_some() => write!(f, "run {id} has no verdict after {waited:.0} s: {run}"),
            _ => write!(f, "run {id} is not T1-confirmed after {waited:.0} s, and the time of its answer is unknown: {run}"),
        }
    }
}

impl std::error::Error for NotYet {}

/// Waits until run `run_id`'s verdict may be paid: T1-confirmed (path T1), or a final block past its
/// answer plus the challenge window (path T0), whichever comes first. A failed run is an error, and so
/// is the bound passing first ([`NotYet`]).
///
/// The rules are near-antibody's for a claim under the instant cap; above it a payout also needs an
/// agreeing second run.
pub fn wait(w: &mut impl Watch, run_id: u64, lim: &Limits) -> Result<Payable> {
    let start = w.local_secs();
    // Whether the cap passed with the answer's time still unknown: the T0 path then says so.
    let mut capped = false;
    let mut chain = None;
    loop {
        let run = w.run()?;
        let waited_s = w.local_secs() - start;
        // T1 pays at once whenever it lands, past the cap too: waiting out the window gains nothing.
        if run["tier"] == json!("T1") {
            return Ok(Payable { path: PayPath::T1, run, note: None, challenge_s: lim.challenge_s, chain_secs: chain, waited_s });
        }
        if run.get("failed").is_some() || run["kind"] == json!("InfraError") {
            return Err(eyre!("run {run_id} failed: {run}"));
        }
        let t0_after = run["answered_at"].as_u64().map(|at| at + lim.challenge_s);
        let mut nap = lim.poll_s;
        match t0_after {
            Some(after) => {
                let c = w.final_secs()?;
                chain = Some(c);
                // near-antibody's `now > at + challenge`. The `allocate` sent next executes in a later
                // block, so at a time at least `c`.
                if c > after {
                    let window = lim.challenge_s;
                    let note = if capped {
                        format!("no T1 in the {} s waited for it: the verdict stays T0 and is paid after the {window} s challenge window", lim.t1_cap_s)
                    } else {
                        format!("no T1 within the {window} s challenge window: the verdict stays T0 and is paid after it")
                    };
                    return Ok(Payable { path: PayPath::T0, run, note: Some(note), challenge_s: window, chain_secs: chain, waited_s });
                }
                // Pacing only: look again about when the window ends on NEAR's clock.
                nap = nap.min((after + 1 - c) as f64);
            }
            // Without the answer's time the window cannot be timed on NEAR's clock: wait for it.
            None => capped |= waited_s > lim.t1_cap_s as f64,
        }
        if waited_s > lim.timeout_s as f64 {
            return Err(NotYet { run_id, run, t0_after, chain_secs: chain, waited_s }.into());
        }
        w.sleep(nap);
    }
}

/// The wait on NEAR: near-antibody's `get_run` and the latest final block.
pub struct NearWatch<'a> {
    ctx: &'a Ctx,
    run_id: u64,
    start: Instant,
}

impl<'a> NearWatch<'a> {
    pub fn new(ctx: &'a Ctx, run_id: u64) -> Self {
        Self { ctx, run_id, start: Instant::now() }
    }
}

impl Watch for NearWatch<'_> {
    fn run(&mut self) -> Result<Value> {
        Ok(cmd::run_view(&cmd::run(self.ctx, self.run_id)?))
    }

    fn final_secs(&mut self) -> Result<u64> {
        self.ctx.near.final_block_secs()
    }

    fn local_secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn sleep(&mut self, secs: f64) {
        std::thread::sleep(Duration::from_secs_f64(secs.max(0.0)));
    }
}

/// A program's challenge window, in seconds.
pub fn challenge_window(ctx: &Ctx, program: u32) -> Result<u64> {
    ctx.program(program)?["params"]["windows"]["challenge"].as_u64().ok_or_else(|| eyre!("program {program} has no challenge window"))
}

/// [`wait`] on NEAR for run `run_id` of `program`, with its challenge window and the default bounds.
pub fn wait_on_near(ctx: &Ctx, program: u32, run_id: u64) -> Result<Payable> {
    wait(&mut NearWatch::new(ctx, run_id), run_id, &Limits::new(challenge_window(ctx, program)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWERED: u64 = 1_793_809_254;
    const WINDOW: u64 = 60;

    /// near-antibody's run and NEAR's blocks as the wait reads them, on a clock only its sleeps advance.
    struct Chain {
        /// NEAR's clock at the head, in seconds.
        t: f64,
        /// The local clock minus NEAR's.
        skew: f64,
        /// How far the final block trails the head.
        lag: f64,
        /// `get_run` shows the answer from here (pending before), and its time from `time_from`.
        answered_from: f64,
        time_from: f64,
        answered_at: u64,
        kind: &'static str,
        /// When T1 lands; `None`, never (a worker image inside its delay).
        t1_at: Option<f64>,
        failed: Option<&'static str>,
        polls: u32,
    }

    impl Chain {
        /// Answered at `ANSWERED` and read just after; T1 eleven seconds later.
        fn new() -> Self {
            let a = ANSWERED as f64;
            Self {
                t: a + 0.5,
                skew: 0.0,
                lag: 2.0,
                answered_from: a,
                time_from: a,
                answered_at: ANSWERED,
                kind: "Violation",
                t1_at: Some(a + 11.0),
                failed: None,
                polls: 0,
            }
        }

        fn no_t1(self) -> Self {
            Self { t1_at: None, ..self }
        }

        fn skewed(self, skew: f64) -> Self {
            Self { skew, ..self }
        }

        /// Still verifying when the wait starts; answered at `at`, its time shown with it.
        fn answered(self, at: u64) -> Self {
            Self { answered_from: at as f64, time_from: at as f64, answered_at: at, ..self }
        }
    }

    impl Watch for Chain {
        fn run(&mut self) -> Result<Value> {
            self.polls += 1;
            if let Some(reason) = self.failed {
                return Ok(json!({"run_id": 7, "mode": "verify", "commit_id": 24, "failed": reason}));
            }
            if self.t < self.answered_from {
                return Ok(json!({"run_id": 7, "mode": "verify", "commit_id": 24, "pending": true}));
            }
            let tier = if self.t1_at.is_some_and(|x| self.t >= x) { "T1" } else { "T0" };
            let at = if self.t >= self.time_from { json!(self.answered_at) } else { Value::Null };
            Ok(json!({"run_id": 7, "mode": "verify", "commit_id": 24, "kind": self.kind, "tier": tier, "answered_at": at}))
        }

        fn final_secs(&mut self) -> Result<u64> {
            Ok((self.t - self.lag).floor() as u64)
        }

        fn local_secs(&self) -> f64 {
            self.t + self.skew
        }

        fn sleep(&mut self, secs: f64) {
            self.t += secs;
        }
    }

    fn waited(c: &mut Chain) -> Payable {
        wait(c, 7, &Limits::new(WINDOW)).unwrap()
    }

    #[test]
    fn t1_pays_at_once() {
        let mut c = Chain::new();
        let p = waited(&mut c);
        assert_eq!((p.path, p.note), (PayPath::T1, None));
        // Within a poll of T1, long before the window ends.
        let t1 = ANSWERED as f64 + 11.0;
        assert!(c.t >= t1 && c.t < t1 + 2.0, "paid at {}, T1 at {t1}", c.t);
    }

    #[test]
    fn without_t1_pays_t0_as_soon_as_a_final_block_is_past_the_window() {
        let mut c = Chain::new().no_t1();
        let p = waited(&mut c);
        assert_eq!(p.path, PayPath::T0);
        // near-antibody's `now > at + challenge`, read on a final block, `lag` behind the head.
        assert_eq!(p.chain_secs, Some(ANSWERED + WINDOW + 1));
        let first = (ANSWERED + WINDOW + 1) as f64 + c.lag;
        assert!(c.t >= first && c.t < first + 2.0, "paid at {}, allowed from {first}", c.t);
        assert_eq!(p.note.as_deref(), Some("no T1 within the 60 s challenge window: the verdict stays T0 and is paid after it"));
        assert_eq!(p.run["tier"], json!("T0"));
    }

    #[test]
    fn a_t1_wait_shorter_than_the_window_still_pays_only_after_it() {
        let mut c = Chain::new().no_t1();
        let p = wait(&mut c, 7, &Limits { t1_cap_s: 20, ..Limits::new(WINDOW) }).unwrap();
        assert_eq!((p.path, p.chain_secs), (PayPath::T0, Some(ANSWERED + WINDOW + 1)));
    }

    #[test]
    fn a_skewed_local_clock_changes_nothing() {
        let outcome = |mut c: Chain| {
            let p = waited(&mut c);
            (p.path, p.chain_secs, c.t, c.polls)
        };
        for skew in [-30.0, 30.0] {
            assert_eq!(outcome(Chain::new().skewed(skew)), outcome(Chain::new()));
            let t0 = outcome(Chain::new().no_t1().skewed(skew));
            assert_eq!(t0, outcome(Chain::new().no_t1()));
            // 30 s fast, the local clock has the window end 30 s early: a final block is still past it.
            assert!(t0.1.is_some_and(|c| c > ANSWERED + WINDOW), "{t0:?}");
        }
    }

    #[test]
    fn with_the_answers_time_unknown_waits_for_it_then_pays_t0_after_the_cap_and_the_window() {
        // The verdict is read at once, the time of its answer only after the 300 s cap.
        let mut c = Chain::new().no_t1();
        c.time_from = ANSWERED as f64 + 320.0;
        let p = waited(&mut c);
        assert_eq!(p.path, PayPath::T0);
        // NEAR's clock passed the window long before, but the wait never guesses the answer's time.
        assert!(p.waited_s > 300.0 && c.t >= c.time_from, "paid at {} after {} s", c.t, p.waited_s);
        assert!(p.chain_secs.is_some_and(|s| s > ANSWERED + WINDOW));
        assert_eq!(p.note.as_deref(), Some("no T1 in the 300 s waited for it: the verdict stays T0 and is paid after the 60 s challenge window"));
    }

    #[test]
    fn a_late_verdict_pays_t0_once_its_own_window_ends_under_the_cap() {
        let late = ANSWERED + 200;
        let mut c = Chain::new().no_t1().answered(late);
        let p = waited(&mut c);
        assert_eq!((p.path, p.chain_secs), (PayPath::T0, Some(late + WINDOW + 1)));
        assert!(p.waited_s < 300.0, "{}", p.waited_s);
        assert_eq!(p.note.as_deref(), Some("no T1 within the 60 s challenge window: the verdict stays T0 and is paid after it"));
    }

    #[test]
    fn a_t1_that_lands_after_the_cap_still_pays_at_once() {
        let late = ANSWERED + 310;
        let mut c = Chain { t1_at: Some(late as f64 + 11.0), ..Chain::new().answered(late) };
        let p = waited(&mut c);
        assert_eq!(p.path, PayPath::T1);
        assert!(p.waited_s > 300.0 && c.t < (late + WINDOW) as f64, "paid at {} after {} s", c.t, p.waited_s);
    }

    #[test]
    fn a_failed_run_is_an_error() {
        let mut c = Chain { failed: Some("OutLayer: the worker failed"), ..Chain::new() };
        let e = wait(&mut c, 7, &Limits::new(WINDOW)).unwrap_err();
        assert!(e.to_string().starts_with("run 7 failed"), "{e}");
        assert!(e.downcast_ref::<NotYet>().is_none());
        // An infrastructure error is retried, never paid.
        let mut c = Chain { kind: "InfraError", ..Chain::new().no_t1() };
        assert!(wait(&mut c, 7, &Limits::new(WINDOW)).unwrap_err().to_string().starts_with("run 7 failed"));
    }

    #[test]
    fn neither_rule_within_the_bound_is_not_yet_rather_than_a_failure() {
        // Never answered.
        let mut c = Chain { answered_from: f64::INFINITY, ..Chain::new() };
        let e = wait(&mut c, 7, &Limits::new(WINDOW)).unwrap_err();
        let n = e.downcast_ref::<NotYet>().expect("NotYet");
        assert!(n.t0_after.is_none() && n.waited_s > (T1_CAP_S + WINDOW + 120) as f64, "{n:?}");
        assert!(e.to_string().starts_with("run 7 has no verdict after"), "{e}");
        // A challenge window longer than the wait: the T0 payout's time is known, not yet reached.
        let mut c = Chain::new().no_t1();
        let e = wait(&mut c, 7, &Limits { timeout_s: 120, ..Limits::new(3_600) }).unwrap_err();
        let n = e.downcast_ref::<NotYet>().expect("NotYet");
        assert_eq!(n.t0_after, Some(ANSWERED + 3_600));
        assert!(n.chain_secs.is_some_and(|c| c <= ANSWERED + 3_600));
    }
}
