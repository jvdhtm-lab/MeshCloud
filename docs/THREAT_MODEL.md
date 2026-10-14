# MeshCloud Threat Model

Scope: the local prototype through Phase 12. Participants, workers, agents, and
jobs are all treated as untrusted.

## Assets

- Participants' wallets and private keys.
- Node and agent signing keys.
- Job inputs, results, and stored objects.
- Contribution receipts, credit balances, and settlement funds.
- The control plane's availability and the integrity of the job ledger.

## Trust boundaries

| Boundary | Trusted | Untrusted |
|---|---|---|
| Participant host | Owner-configured resource policy | Everything a job or agent does |
| Worker process | The worker binary and its sandbox | Job code, agent code, network input |
| Control plane | The primary database and the operator | Worker/agent claims, manifests, offers |
| Blockchain | The settlement decision it records | Claims that computation happened |

## Adversaries and mitigations

### 1. Fabricated work (worker or agent claims success)
- **Threat:** a worker returns a result it never computed.
- **Mitigation:** verification by deterministic re-execution (`mesh-verifier`);
  only reproduced results earn receipts. Test: `verification_gates_reward_and_is_idempotent`.

### 2. Duplicate/replayed rewards
- **Threat:** replay a completion or a receipt to mint credit twice.
- **Mitigation:** receipts are unique per `(attempt_id, role)`; batches are
  unique per receipt; settlement claims are unique per `receipt_hash` on-chain.
  Tests: idempotent completion, local settlement, `MeshReward` duplicate-claim.

### 3. Sybil / fake capacity
- **Threat:** create many identities to farm rewards.
- **Mitigation:** rewards require verified work, not advertisements; offers
  expire; revoked offers are not scheduled.

### 4. Malicious job / agent code
- **Threat:** steal host files, credentials, or attack peers.
- **Mitigation:** WASM has no WASI, no preopened dirs, no network by default;
  memory, fuel, and wall-clock limits; optional per-job process isolation.
  Tests: fuel/timeout/memory limits, host-import rejection.

### 5. Prompt injection / tool poisoning (agents)
- **Threat:** task input steers an agent into unsafe tool use.
- **Mitigation:** task input is data, never instructions; tools are an explicit
  capability allowlist enforced at the worker (`allows_workload`); only
  deterministic, tool-verifiable subtasks are rewarded.

### 6. Delegation abuse (runaway DAGs, cycles, self-delegation)
- **Threat:** exhaust the mesh with unbounded recursion.
- **Mitigation:** `max_depth`, `max_fanout`, shared budget, inherited deadline,
  cycle/self-delegation rejection, cascade cancellation, message metering.
  Tests: `check_bounds`, delegation e2e.

### 7. Abuse / spam
- **Threat:** spam reports, chatter, or malicious nodes.
- **Mitigation:** abuse reports with an auto-quarantine threshold (decommission
  node + revoke offer); agent message rate + byte metering.

### 8. Disputed contributions
- **Threat:** an incorrect reward is issued.
- **Mitigation:** disputes void the receipt and reverse the credit; voided
  receipts are excluded from settlement.

### 9. Network partitions
- **Threat:** partitions create conflicting accepted jobs or double settlement.
- **Mitigation:** strongly consistent state lives in a single-writer primary
  PostgreSQL (leases via `FOR UPDATE SKIP LOCKED`, unique constraints); the mesh
  only gossips eventually consistent discovery.

### 10. Supply chain
- **Threat:** a malicious dependency.
- **Mitigation:** `cargo-audit` and `cargo-deny` gate CI; dependency tree kept
  small; `#![forbid(unsafe_code)]` everywhere.

## Residual risks (must be addressed before public launch)

- A single verifier can authorize arbitrary receipts until independent
  verification and dispute handling are decentralized.
- Storage is held by the control plane in the prototype; a real deployment must
  store shards on assigned nodes with signed challenges.
- The token/economic model is not validated; no real value should be attached.
