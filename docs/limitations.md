# Limitations

What Antibody does not do today, and the limits every program on this deployment runs under. The site's
list of limits (`/judges#limits`) is generated from this file by `scripts/docs/judges.py`.

- **Only rule breaks are paid automatically.** Anything else is at the project's discretion (`top_up`).
- **Duplicates are judged by (rule, entry point).** Two bugs behind one entry point count as one until
  the first is fixed (rounds).
- **One block.** A proof runs in the anchor block's environment: no time travel, no multi-block attacks.
- **Ethereum L1 for verification** (Sepolia). Other chains are pause and release targets.
- **The EVM rules are the manifest's, not the chain's.** Sepolia runs the Glamsterdam upgrade (its headers
  carry `blockAccessListHash` and `slotNumber`, and the anchor check hashes them). A proof runs under the
  rules its manifest commits (`evm_spec: osaka`), so gas is counted by osaka's costs: state creation costs
  five to seven times more on Sepolia itself.
- **Public Sepolia RPCs prove about 128 blocks back**, so witnesses are built within minutes of an anchor,
  and the light client runs about 80 blocks behind: a program is created some 15 minutes after its anchor.
- **Claims are paid in commit order.** A commit that is never revealed holds the later ones until its
  reveal window ends and the keeper expires it (on testnet, a practice try's payout waited ten minutes
  this way).
- **The testnet light client has one trusted relayer.** Anchors it cannot confirm are labelled
  project-attested.
- **TEE trust is tiered, not removed.** T1 needs DCAP collateral. Without T1 a verdict rests on OutLayer's
  operator (T0): under the instant cap it becomes final once its challenge window passes (60 s on testnet),
  so a false one that nobody challenges stands, and a false Violation is paid; above the cap a second
  OutLayer run must also agree. A challenge demands T1; anyone may file one, but neither the CLI nor the
  services do.
- **Worker images.** A worker image OutLayer approves later counts for existing programs only after the
  image delay (2 days on testnet); until then a run on it gets no T1, so its payout waits the challenge
  window and its pause waits too, since a pause needs T1 (docs/runbooks/worker-images.md).
- **Old anchors.** A proof that reads beyond the pinned base needs an RPC that keeps the anchor's state.
- **Rule strength is the rules' measure,** and a survivor is not necessarily a bug.
- **Testnet only.** The payout token, wNEAR on testnet, has no value.
