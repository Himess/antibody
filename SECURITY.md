# Security

## Deliberately vulnerable test fixtures

`contracts/evm/src/acme/` ("Acme Lend") is a **fictional protocol with bugs planted on purpose**. Every
file there is labelled `DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet`. Its proofs of
concept (`contracts/evm/test/acme/pocs/`) work only against this repository's fixture. They exist so that
Antibody's verifier and settlement contract can be demonstrated end to end on testnets.

This repository does not target, analyse or test third-party protocols, and contains no exploit code for
real-world systems.

## Networks

Everything runs on testnets (NEAR testnet, Ethereum Sepolia, Base Sepolia) or local chains. No mainnet
transactions are sent. The "mainnet" view is a clearly labelled mock built from recorded data.

## Reporting a vulnerability in Antibody itself

Please open a private security advisory on this repository (GitHub → Security → Report a vulnerability)
rather than a public issue.

## Secrets

Keys are read from environment variables or the git-ignored `.secrets/` directory and are never printed or
committed. Every commit is scanned with gitleaks (`.githooks/pre-commit`, enabled with
`git config core.hooksPath .githooks`) and again in CI.
