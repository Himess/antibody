# Attacks

What each attack on Antibody costs the attacker, and what we found attacking our own system. Every finding
below was fixed and is pinned by a test. The threat model itself is in [threat-model.md](threat-model.md).

## The cost of an attack

| Attack | What it costs the attacker | What the defense gives up | Check |
|---|---|---|---|
| Slop and spam | A bond per commit, forfeited to the program's escrow, plus the verification run | Nothing: bonds are sized from measured cost | Property 7 |
| Prompt injection in a report | The bond | Nothing: the verifier never reads reports | Story step C |
| Copying or front-running a proof | Impossible before its verdict: proofs are sealed to the verifier and priority is the commit | — | AEAD binds the hunter |
| A forged or tampered witness | The bond (`InvalidPoC`) | Nothing: proofs, code and tries are validated | SA-2, SA-3, SA-25, fuzzing |
| Locking an escrow with copies of a paid proof | A bond per copy | Nothing: copies share one reservation per key | SA-9 |
| Filling near-antibody's storage | The storage, paid by the caller (each commit prepays its settlement) | Nothing | SA-24 |
| Steering a finding to a new key to be paid twice | The bond | Nothing | SA-1 |
| A dishonest TEE operator | It can delay; it cannot pay without a quote verified on NEAR | T0 payouts above the cap wait their window and a second run | Property 8 |
| Challenging honest payouts to delay them | A challenge bond, lost when T1 upholds the payout | A delay up to the resolve window | Threat P12 |
| A project closing its program to escape payouts | Needs a T1 `ProgramMisconfigured` or the window; the escrow stays until exit | — | Property 6 |
| A compromised owner | Can change the defaults of new programs only | — | Property 12 |
| A hijacked agent | Bonds within its per-commit cap and a budget that counts losses, on its own programs only; its key cannot withdraw | Its owner's capped credit | Containment tests, SA-13 |
| Draining the practice faucet | Rate-limited per hour; the public vend key's allowance caps the gas | A small, refilled float | `/v1/status` alerts |

## The self-attack log

We attack our own system. Each row below was found by us, fixed, and is pinned by a test: SA-1 to SA-8
while building, SA-9 on in a fresh-context review of the contracts, the crypto and the new mechanisms
before the launch deployment was locked.

| Id | What broke | Fix | Test |
|---|---|---|---|
| SA-1 | **Fingerprint steering.** Evaluations inside a frame that later reverted stayed on the timeline, so one bug could be re-keyed to any entry point and paid again | A failing frame truncates the evaluation timeline | `fixtures/regressions` (`steering`), native and wasm |
| SA-2 | **Tampered jump tables.** A witness's jump table was trusted, so a hunter could make the anchor snapshot fail and close the program | Every bytecode is re-analysed from its hashed bytes | `fixtures/regressions` (`jump-table-*`) |
| SA-3 | **Relabelled trie nodes.** A leaf and an extension can encode alike, so a witness could forge absent state under a valid root | Every trie node's kind is validated | `trie::tests::relabelled_extension_is_refused` |
| SA-4 | **A partial fix locked a program out of releases.** Re-runs were never reset between proposals, so a key one fix carried could never be re-run at the next | Re-runs count per proposal | `later_fix_releases_carried_key`, Quint `laterFixReleasesCarriedKeyTest` |
| SA-5 | **A release permit was not bound to the checked code**: it could install code the re-runs never ran | The round check binds each release's proxy and code hash | `a_round_check_binds_its_releases` |
| SA-6 | **An unjudged re-run counted as fixed.** A re-run that returned `InvalidPoC` opened the release gate | Only a T1 Violation or NoViolation is evidence | `an_unjudged_rerun_is_no_evidence` |
| SA-7 | **A round could drop the rule the round before added** | A round keeps every slot of the open round | `a_round_keeps_the_invariants_of_the_round_before` |
| SA-8 | **Re-runs used the witness's rule code**, not the round manifest's | The manifest's code is the rule code | `a_rerun_uses_the_rounds_invariants_code` |
| SA-9 | **Copies of a paid proof locked an escrow.** Once it read a paid proof, a project could commit copies from other accounts: each reserved its full amount, later holders became debts `pay` refused, and at exit the debts defaulted before the reservations went back to the project | Copies of a held proof share one reservation per key; the exit releases reservations, then funds debts, then pays | `copies_of_a_held_proof_take_one_reservation`, `exit_funds_owed_claims_before_releasing_duplicates` |
| SA-10 | **A disclosed tag stayed open.** Disclosure keys were per tag, so a later round's new proof of the same tag was readable as soon as its answer landed; a tag counted as fixed even when its release gate failed | One key per round and tag; a tag counts as fixed only once released | `a_released_fix_discloses_its_round_only`, `a_fix_that_does_not_ship_discloses_nothing` |
| SA-11 | **The distinctness top-up was an oracle.** Its requester chose the earlier finding and the patch: a no-op patch bypassed the slot budget, and a predicate patch leaked a bit of any sealed proof per run | The top-up is off | `per_invariant_budget_and_no_distinct_top_up` |
| SA-12 | **Gap claims and `unevaluable: violation` paid per copy**, above the cap without a second run | One claim per key and round, copies are duplicates; above the cap they wait for a second run | `one_gap_bug_is_promoted_once`, `an_unevaluable_violation_is_paid_once_per_round_and_waits_above_the_cap` |
| SA-13 | **An agent's budget bounded locked bonds, not losses**: through a hijacked agent, a program built to forfeit bonds drained the owner's credit | Losses count against the budget; an agent may be bound to programs; practice accounts commit to the practice program only | `agent_containment`, `an_agent_commits_only_to_its_programs` |
| SA-14 | **`exit` and `open_round` scanned every commit in one call**: a few hundred commits would stop a program's rounds and its exit for good | Both work a page per call and keep their cursor; the exit's own work is bounded by the program's keys | `the_exit_is_bounded_by_keys_and_records_close_in_pages`, the Quint replay (pages of three) |

The same review's medium and low findings:

| Id | What broke | Fix | Test |
|---|---|---|---|
| SA-15 | A hunter's own unreadable blob location held the allocation queue for free | Unreadable for the whole grace forfeits the bond | `a_blob_nobody_can_read_for_the_whole_grace_forfeits` |
| SA-16 | The exit refunded final findings not yet allocated | The exit waits until every commit is allocated or closed | `exit_settles_everything` |
| SA-17 | A round check or re-run answered without a quote blocked the program's rounds | Past the run timeout it can be replaced | `a_round_check_without_a_quote_can_be_replaced`, `a_rerun_without_a_quote_can_be_requested_again` |
| SA-18 | Relaying an earlier round's permit could downgrade a proxy | A proxy's releases only move forward | `test_releasesOnlyMoveForward` (forge) |
| SA-19 | A re-run at the project's fix code published impact and harm numbers of unpaid sealed proofs | A re-run publishes only which keys still break | `a_rerun_judges_the_same_poc_at_the_fix` |
| SA-20 | The owner could extend a live program's worker images through another registry | A program pins its registry; later images count only from it, after the delay | `the_owner_cannot_reach_a_live_program_through_another_image_registry` |
| SA-21 | A release wrap the TEE never backed could leave the project unable to read what it paid for | A wrap no quote confirmed can be replaced after the run timeout | `a_release_wrap_no_quote_confirms_can_be_requested_again` |
| SA-22 | The verdict guard froze again on duplicates and known issues, read the first round's slots only, and acted on unconfirmed anchors | First findings only, the slots as of now, light-client anchors only | `only_first_critical_t1_findings_against_a_confirmed_anchor_count` |
| SA-23 | A parent could lock a sub-account's credit by registering it as an agent | An account with credit or commits of its own never becomes an agent | `a_parent_never_makes_an_agent_of_an_account_with_value_of_its_own` |
| SA-24 | near-antibody's own balance paid for others' storage (up to 256 image entries per free sync) | Syncs only from named registries, paid by their caller; run requests charge their payer; each commit prepays its settlement's bytes | `only_a_named_registry_is_synced`, `a_commits_settlement_is_prepaid` |
| SA-25 | Our parser fuzzing found three panics on malformed witnesses in dependencies (bitvec's `set_len` assert and `todo!()`, revm-bytecode's unchecked `original_len`); in wasm a panic is a trap, not a typed verdict | Patched copies in `third_party/patches`: each is a decode error | `a_mutated_witness_is_refused_or_decodes`, `the_verifier_answers_any_mutated_program_witness` (20,000 cases each) |

| SA-26 | A program's manifest location, chosen by whoever creates the program, could make every client (`judge`, the MCP tools, `try`) open a local file, a network share or an endless body | On a public network a location from the chain opens only as `near-tx:` or https, and every fetched body stops at 16 MiB | `a_chain_location_cannot_open_local_files_on_a_public_network`, `a_fetched_body_is_capped` |
| SA-27 | The MCP server opened an agent's absolute path before refusing it (a network share contacted; a different answer for a file outside that exists) | A path is checked by its words first: no `..`, an absolute path only under the workspace; outputs never leave through a symlink | `a_path_outside_is_refused_before_it_is_opened`, `an_output_directory_cannot_leave_through_a_symlink` |
| SA-28 | MCP error results carried text others wrote (a program's location, a contract's message) without the untrusted-data notice | An error is delimited as data like a result | `an_error_is_delimited_as_data` |
| SA-29 | `practice_try` followed the agent's `vending` and `program_id`, so text an agent read could point the server at another contract | Only the deployment's practice vending contract, on the program it vends for | `practice_try_uses_the_deployments_practice_vending_only` |
| SA-30 | A vend or a commit sent before its key or blob was kept could strand an account or lock a bond behind a proof nobody can reveal; the MCP server kept them wherever its client started it | Keys and blobs are written before the transaction; the server's state is inside its workspace, checked writable at startup | `names_and_state_stay_usable_inside_the_workspace` |
| SA-31 | RPC URLs, which may carry keys, reached error messages through the HTTP client's errors | Errors name the RPC's label, never its URL | (`without_url` in `near.rs` and the witness client) |

The review's two remaining notes are stated as residuals in the [threat model](threat-model.md) (I4b,
P13): OutLayer's quote metadata is not checked, and a page that gets a project's wallet to sign the
fixed NEP-413 message derives its key.

Building the end-to-end story found more (fixed, with tests): a failed round check stranded its proposal;
four permits did not fit one call; the gap lane's model allowed a re-run after exit. See
[properties.md](properties.md).
