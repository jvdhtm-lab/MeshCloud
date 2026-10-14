# MeshCloud Security Review Checklist

A production launch is gated on independent security and smart-contract review.
Use this checklist before attaching real value.

## Code and dependencies

- [ ] `cargo fmt --check`, `cargo clippy -- -D warnings` clean.
- [ ] `cargo audit` and `cargo deny check` clean; dependency tree reviewed.
- [ ] Every `unsafe` block justified and reviewed (default is
      `#![forbid(unsafe_code)]`).
- [ ] Protocol and agent-message decoders fuzzed (`just fuzz`).
- [ ] No `unwrap()`/`expect()` on untrusted network input.

## Isolation and resource limits

- [ ] WASM runs with no WASI, no preopened directories, and no network.
- [ ] Memory, fuel, and wall-clock limits enforced; per-agent caps enforced.
- [ ] Per-job/per-agent process isolation enabled for public workers.
- [ ] Owner-configured CPU/memory/storage caps enforced per OS.
- [ ] The Docker socket and host credentials are never exposed to jobs/agents.

## Identity and authorization

- [ ] Node, agent, and wallet keys are separate; private keys never leave the host.
- [ ] Offers and manifests are signed over canonical bytes with domain
      separation, protocol version, and nonce; expiry and revocation enforced.
- [ ] Settlement issuance is restricted to the authorized issuer; duplicate
      claims rejected on-chain.

## Verification and accounting

- [ ] Every reward maps to an accepted, independently verified result.
- [ ] Receipts are unique per `(attempt, role)`; batching and settlement are
      idempotent.
- [ ] Disputes void receipts and reverse credit; voided receipts are not settled.
- [ ] Verifier centralization reduced; independent verification and dispute
      handling defined.

## Operations

- [ ] Threat model reviewed (`docs/THREAT_MODEL.md`).
- [ ] Metrics and alerting wired; incident runbook reviewed (`docs/OPERATIONS.md`).
- [ ] Single-writer replica model documented; failover tested.
- [ ] Economic model and cost per accepted result measured (blueprint App. C).

## Smart contracts

- [ ] Independent audit of `contracts/src/MeshReward.sol`.
- [ ] Duplicate-claim, unauthorized-issuer, and invalid-proof tests pass.
- [ ] Merkle leaf encoding verified to match the off-chain implementation.
