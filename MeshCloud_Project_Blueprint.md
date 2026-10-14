# MeshCloud — Project Blueprint and Implementation Roadmap

**Version:** 4.0 (Rust-first, Agent Mesh, Phased Build)
**Status:** In development
**Purpose:** Build a volunteer-computing cloud that lets participants share spare computing and storage resources and earn rewards for verified contributions. AI agents hosted on worker nodes collaborate and delegate subtasks under bounded rules.

Every capability described here is built. The work is organized as a sequence of phases; each phase is a working increment that builds directly on the phase before it. Nothing is skipped or set aside — later phases simply arrive after the earlier ones are standing.

---

## Terminology

- **Worker** — the native Rust daemon (`mesh-worker`) installed on a participant's machine. Manages resources, heartbeats, leases, and the sandbox.
- **AI agent** (or **mesh-agent**) — a long-running software actor hosted by a worker. Has its own Ed25519 identity, capability manifest, tool access, memory, budget, and task state. One worker can host zero, one, or many AI agents.
- **Node** — the logical identity of a participant's machine in the network.
- **Participant** — a human or organization controlling one or more nodes and a wallet.
- **Job** — a unit of scheduled work. Agent tasks are a kind of job (see §9).
- **Root job** — the top of a delegation tree. All jobs in a tree share one `root_job_id`.

"Agent" alone means **AI agent** unless the phrase "worker agent" or "mesh-worker" is used.

---

## 1. Vision

MeshCloud is a distributed cloud-computing network made from participants' spare resources: CPU, GPU, RAM, storage, and bandwidth.

On top of the compute network, MeshCloud runs an **Agent Mesh**: AI agents hosted on worker nodes that can collaborate with each other, split work into subtasks, and delegate to idle peers under bounded, verifiable rules.

Development begins by simulating multiple independent computers using Docker containers on one development machine. The same worker protocol then supports native Rust workers running on Windows, macOS, and Linux hosts, and the same agent protocol supports a heterogeneous mesh of agents.

### Core principle

> **The blockchain rewards verified contributions. MeshCloud performs scheduling, execution, verification, and storage. Agents propose, delegate, and collaborate — within bounds set by MeshCloud. Nothing is rewarded unless it is verified.**

Do not put every heartbeat, resource advertisement, job assignment, agent message, or telemetry event on-chain. Keep operational data off-chain. Use the blockchain for settlement and, where appropriate, auditable commitments.

---

## 2. Design principles

1. **Assume churn:** workers and agents may sleep, reboot, disconnect, or disappear at any time.
2. **Assume untrusted participants:** never trust a worker's or agent's claims without evidence and appropriate verification.
3. **Respect resource owners:** resource sharing is configurable, capped, and revocable.
4. **Schedule by capability:** match work to available CPU, RAM, GPU, storage, tools, and reliability.
5. **Separate execution from settlement:** jobs must work before blockchain rewards are enabled.
6. **Make effects idempotent:** retries must not cause duplicate job effects or duplicate rewards.
7. **Minimize trust:** make verifier assumptions explicit and reduce centralized authority over time.
8. **Build incrementally:** each phase must have working software and acceptance tests before the next phase begins.
9. **Use the type system:** model protocol messages, job states, agent task states, and receipts as Rust types so invalid states are unrepresentable where practical.
10. **Agents are bounded actors:** every delegation has a root job, a depth limit, a fanout limit, a budget, and a deadline. No unbounded recursion, no self-delegation loops, and no free-form mesh chatter that costs resources without accounting.
11. **One scheduler:** agent tasks are jobs with children, not a second scheduling system. If a feature applies to jobs, it applies to agent tasks by construction.
12. **Verification before reward:** a reward is issued only for work whose correctness is checked. Deterministic re-execution and tool-checkable predicates are the primary verification mechanisms; richer verification for other workloads is added in later phases.

---

## 3. Why Rust

Rust is chosen for the core because MeshCloud's hardest parts are native, security-sensitive, and long-running:

- **Native worker agent:** a single static binary per OS, low idle overhead, no garbage-collector pauses on participants' machines.
- **Resource monitoring and enforcement:** fine-grained control over CPU, memory, and process behavior.
- **Sandboxed execution:** first-class Wasmtime integration (Wasmtime is itself written in Rust).
- **Agent runtime:** long-running async services with strict resource budgets and shared types across worker and agent.
- **Shared protocol types:** the control plane, workers, agents, verifier, and CLI share one set of crates, so message formats cannot drift.
- **Memory safety:** compile-time guarantees for safe code, which matters for software running untrusted jobs and untrusted agent code on strangers' machines.

Qualifications:

- Rust does not make a distributed system secure. Protocols, authorization, job isolation, agent isolation, and verification still require careful design.
- Development is slower at first than in Go. Mitigate this by keeping the workspace small, splitting crates only when boundaries are clear, and avoiding premature abstraction.
- Rust owns the core, but not everything: Solidity for on-chain contracts, optional TypeScript/React for a dashboard, Docker Compose for local simulation, PostgreSQL for durable coordination state.

---

## 4. Architecture overview

```text
Participant
├── Wallet / cryptographic identity
├── Resource-sharing policy
└── MeshCloud worker (native Rust binary: mesh-worker)
        │
        ├── Agent runtime host (zero or more AI agents)
        │     ├── Agent A (planner)
        │     ├── Agent B (tool user)
        │     └── Agent C (idle, can accept delegations)
        │
        v
MeshCloud network
├── Membership and node registry
├── Agent registry (identity, capabilities, manifest hash)
├── Resource advertisements and heartbeats
├── API / control plane (Rust, Axum)
├── Scheduler (jobs, leases, retries, AND agent-task DAGs)
├── Sandboxed executor (Wasmtime)
├── Verification service
├── Contribution accounting
└── Storage and repair services
        │
        v
Settlement adapter
└── Blockchain contract(s)
    ├── Authorized reward settlement
    ├── Duplicate-claim prevention
    └── Token/credit balances or claims
```

There is **one scheduler**, not two. Agent delegation is a module inside `mesh-scheduler` that understands parent/child job relations and delegation messages. It is not a peer service.

### Docker simulation

```text
docker compose
├── mesh-postgres
├── mesh-control-plane
├── mesh-worker-1      (hosts agent-1a, agent-1b)
├── mesh-worker-2      (hosts agent-2a)
└── mesh-worker-3      (hosts no agents, compute only)
```

Each worker has a distinct identity, advertised capacity, heartbeat, and job executor. Containers simulate independent nodes but share the development host's hardware and failure domain. Agents are logically separate from workers even when co-located.

---

## 5. Technology choices

| Area | Initial choice | Evolution |
|---|---|---|
| Core language | Rust (stable toolchain, pinned via `rust-toolchain.toml`) | Same |
| Workspace | Cargo workspace with small crates | Split crates/services where justified |
| Async runtime | Tokio | Same |
| API | Axum + Serde (JSON) | Versioned API, optionally gRPC (tonic) |
| Operational database | PostgreSQL via `sqlx` | Replicated control plane |
| Migrations | `sqlx migrate` | Same |
| Job coordination | PostgreSQL transactions, leases, `FOR UPDATE SKIP LOCKED` | Distributed coordination where justified |
| Worker transport | HTTP on a private Docker network | Encrypted P2P transport |
| Agent transport | Same HTTP transport as workers initially | libp2p request-response / gossip |
| Membership | Explicit registry | `rust-libp2p` and gossip |
| Identity and signing | `ed25519-dalek` | Hardware-backed keys where available |
| CLI | `clap` | Same |
| Resource monitoring | `sysinfo` plus OS-specific APIs | Per-OS enforcement (cgroups, Job Objects) |
| Workload isolation | Safe test executor, then Wasmtime | WASM and carefully isolated Linux workloads |
| Agent isolation | Same runtime host, resource caps, tool allowlist | Per-agent sandboxed process |
| Agent model backends | Pluggable trait (deterministic stub first, then local/remote LLM) | Multiple backends, model attestation |
| Object storage | Local filesystem and metadata | Encrypted chunks and erasure coding |
| Blockchain development | Local EVM chain (Anvil) | Selected production chain |
| EVM client from Rust | `alloy` | Same |
| Smart contracts | Solidity + Foundry | Audited contracts |
| Observability | `tracing` structured logs and health endpoints | Prometheus/Grafana via OpenTelemetry |
| Testing | `cargo test`/`cargo nextest`, `testcontainers`, `proptest` | Chaos and security suites |
| Supply-chain checks | `cargo-audit`, `cargo-deny` | Reproducible release builds |

Keep the deployment small and modular. Do not split every crate into a separate service before there is a demonstrated need.

---

## 6. Repository structure

```text
meshcloud/
├── Cargo.toml                 # workspace manifest
├── Cargo.lock
├── rust-toolchain.toml
├── deny.toml                  # cargo-deny policy
├── PROJECT_BLUEPRINT.md
├── crates/
│   ├── mesh-protocol/         # Shared messages, job states, data types
│   ├── mesh-identity/         # Node & agent identity, signing, auth
│   ├── mesh-worker/           # Native worker daemon (binary)
│   ├── mesh-control-plane/    # API and coordination (binary)
│   ├── mesh-scheduler/        # Placement, leases, retries,
│   │                          #   AND agent-task DAG + delegation
│   ├── mesh-runtime/          # Wasmtime execution and limits
│   ├── mesh-verifier/         # Work verification
│   ├── mesh-accounting/       # Contribution receipts and rewards
│   ├── mesh-settlement/       # Settlement trait + EVM implementation
│   ├── mesh-agent-protocol/   # Agent messages, task states, capabilities
│   ├── mesh-agent-runtime/    # Agent process host, tools, memory, backends
│   ├── mesh-network/          # P2P
│   ├── mesh-storage/          # Distributed storage
│   └── mesh-cli/              # User-facing CLI (binary: `mesh`)
├── contracts/
│   ├── src/
│   ├── test/
│   └── script/
├── migrations/
├── deploy/
│   ├── docker/
│   └── production/
├── configs/
├── examples/
├── tests/
│   ├── integration/
│   ├── chaos/
│   └── security/
├── docker-compose.yml
├── justfile
└── README.md
```

These are logical crates, not necessarily separate processes. **Start with a small workspace** (`mesh-protocol`, `mesh-identity`, `mesh-control-plane`, `mesh-worker`, `mesh-cli`) and add the other crates as the phase that needs them arrives. Introduce agent crates in Phase 6, not before.

Rust conventions for the workspace:

- `#![forbid(unsafe_code)]` in every crate by default; allow `unsafe` only in explicitly reviewed crates (for example, OS-specific resource enforcement) with documented justification.
- Shared dependency versions live in `[workspace.dependencies]`.
- CI runs `cargo fmt --check`, `cargo clippy -- -D warnings`, tests, `cargo audit`, and `cargo deny check`.

---

## 7. Main domain entities

Start with a small schema and evolve it through migrations.

### 7.1 Compute and settlement entities

| Entity | Purpose |
|---|---|
| `participants` | Participant ID, public key, wallet address, status |
| `nodes` | Node ID, participant ID, capabilities, last heartbeat, status |
| `resource_offers` | Node ID, CPU/RAM/GPU/storage limits, expiry, signature |
| `jobs` | Job ID, **kind** (`wasm`, `agent_task`, …), **parent_job_id**, root_job_id, depth, requester, status, deadline, budget |
| `job_attempts` | Job ID, worker ID, attempt number, lease, timestamps, result hash |
| `verification_records` | Attempt ID, method, evidence reference/hash, outcome |
| `contribution_receipts` | Receipt ID, contributor, job/attempt, accepted usage, reward, status |
| `reward_batches` | Batch ID, Merkle root or receipt range, total reward, settlement status |
| `settlements` | Batch ID, chain ID, transaction hash, status |
| `credit_balances` | Participant ID, earned, spent, reserved, available |

### 7.2 Agent entities

| Entity | Purpose |
|---|---|
| `agents` | Agent ID, node ID, participant ID, public key, manifest hash, status, last heartbeat |
| `agent_capabilities` | Agent ID, skills, tools, models, data access, max concurrent tasks |
| `agent_delegations` | Parent job, child job, delegator agent, delegatee agent, state, lease |
| `agent_messages` | Signed messages, job references, payload hash, timestamp, nonce |

Agent tasks are **not** a separate table. An agent task is a row in `jobs` with `kind = 'agent_task'`. Its attempts are rows in `job_attempts`. There is one receipt pipeline, one lease system, and one verifier.

Avoid storing secrets or private keys in ordinary database fields. Store evidence references and hashes where appropriate, and define retention rules for larger artifacts.

In Rust, represent statuses as enums mapped to Postgres enums or constrained text, and use newtypes (`NodeId`, `AgentId`, `JobId`, `ReceiptId`) so identifiers cannot be mixed up.

---

## 8. Resource offers and identity

A node publishes a signed, expiring offer describing what it is willing to share. The offer is not a guarantee of availability. Agents publish an analogous signed capability manifest.

Example node offer payload (illustrative; production signing needs canonical serialization, domain separation, protocol versioning, and replay protection):

```json
{
  "node_id": "node-alice-01",
  "capabilities": {
    "cpu_cores": 4,
    "ram_gb": 16,
    "storage_gb": 100,
    "gpu": false
  },
  "limits": {
    "max_cpu_percent": 50,
    "max_memory_mb": 4096
  },
  "availability": "when_idle",
  "expires_at": "<RFC3339 timestamp>",
  "nonce": 1042
}
```

Example agent capability manifest (illustrative):

```json
{
  "agent_id": "agent-alice-planner-01",
  "node_id": "node-alice-01",
  "protocol_version": 1,
  "capabilities": {
    "skills": ["planning", "task_split", "code_review"],
    "tools": ["wasm_exec", "http_fetch_allowlisted", "kv_store"],
    "models": ["local-llm-small", "noop-deterministic"],
    "max_concurrent_tasks": 4,
    "max_depth_supported": 3
  },
  "policy": {
    "accepts_delegation": true,
    "min_budget_credits": 1,
    "max_task_seconds": 300
  },
  "expires_at": "<RFC3339 timestamp>",
  "nonce": 77
}
```

Requirements:

- Use a cryptographic identity (Ed25519 via `ed25519-dalek`) and signed advertisements for both nodes and agents, with separate keys for the node and each agent.
- Sign a **canonical byte encoding**, not "whatever JSON Serde produced." Choose one canonical form (for example, a deterministic encoding such as canonical CBOR, or a fixed-field binary layout) in `mesh-protocol` and test it with property tests.
- Include a domain-separation tag and protocol version in every signed payload.
- Offers and manifests expire and can be revoked.
- Measure availability again when a job or agent task is scheduled.
- A wallet or public key proves control of a key, not the participant's real-world identity or trustworthiness.
- Never reward a node or agent merely for advertising capacity or creating more identities.
- Store private keys with restrictive file permissions (or the OS keychain where available), and use `zeroize` for in-memory key material.

---

## 9. Job lifecycle and API

### 9.1 Jobs (all kinds)

| Endpoint | Purpose |
|---|---|
| `POST /api/v1/nodes/register` | Register a worker |
| `POST /api/v1/nodes/{id}/heartbeat` | Report liveness and current capacity |
| `GET  /api/v1/nodes` | List nodes and status |
| `POST /api/v1/jobs` | Submit a job (any kind) |
| `GET  /api/v1/workers/{id}/jobs/next` | Claim eligible work |
| `POST /api/v1/jobs/{id}/complete` | Submit result and evidence |
| `POST /api/v1/jobs/{id}/fail` | Report failure |

Job states:

`pending -> scheduled -> running -> succeeded`

Failure and recovery states: `retry_wait`, `failed`, `cancelled`, `expired`. Jobs with children (`kind = agent_task`) add `waiting_on_children` between `running` and `succeeded`.

Implementation rules:

- Use atomic database claims or leases (for example, `SELECT ... FOR UPDATE SKIP LOCKED` inside a transaction) to prevent conflicting claims.
- Expire leases when a worker or agent disappears.
- Make retries safe and job effects idempotent.
- A timeout is evidence that a worker is unreachable, not proof that it is permanently dead.
- Do not issue a reward solely because a worker says it completed a job.
- Model the job state machine as a Rust enum with explicit transition functions so illegal transitions are rejected in one place.
- Put request/response types in `mesh-protocol` so the control plane, workers, and CLI share them.

### 9.2 Agent task API (a subset of the job API)

Because agent tasks are jobs, most of §9.1 applies unchanged. The additional endpoints are:

| Endpoint | Purpose |
|---|---|
| `POST /api/v1/agents/register` | Register an agent on a node |
| `POST /api/v1/agents/{id}/heartbeat` | Agent liveness and current load |
| `GET  /api/v1/agents` | List agents, capabilities, status |
| `POST /api/v1/jobs/{id}/delegate` | Create a child job via delegation |
| `GET  /api/v1/agents/{id}/jobs/next` | Claim eligible agent work (bid/award model) |
| `GET  /api/v1/jobs/{id}/tree` | Return the DAG rooted at `root_job_id` |
| `POST /api/v1/jobs/{id}/cancel` | Cancel a job and cascade to children |

The scheduler enforces DAG invariants. Neither delegator nor delegatee can widen scope.

### 9.3 Delegation bounds (in types and in the database)

Every job row carries:

```text
root_job_id       # shared across the whole tree; prevents cross-root fusion
parent_job_id     # null for the root
depth             # 0 for the root, +1 per delegation
max_depth         # set by the root requester, enforced at delegation
fanout            # number of direct children
max_fanout        # cap
budget_credits    # shared pool across the tree; children draw from parent
deadline          # inherited or narrowed by children, never widened
```

Hard rules (rejected if violated):

- `delegator_id == delegatee_id`
- `delegatee` is an ancestor of `delegator` (cycle detection)
- `depth + 1 > max_depth`
- Sum of children's reserved budget > parent's remaining budget
- Child deadline > parent deadline
- Cancellation does not cascade

### 9.4 Agent messaging metering

Agent messages (proposals, bids, awards, progress, results) consume control-plane and network resources. They are metered from the phase that introduces delegation:

- Per-agent message rate limits (token bucket).
- Per-agent byte budget per root job.
- Delegation attempts count against budget even if rejected.
- Free-form agent-to-agent chatter is not supported; all messages reference a job.

---

## 10. Agent Mesh

The Agent Mesh is a **layer of jobs**, not a separate scheduling system.

### 10.1 What an agent is

An agent is a long-running process hosted by a worker with:

- **Identity** — its own Ed25519 keypair, separate from the node key and from any wallet.
- **Manifest** — a signed capability advertisement.
- **Job ownership** — a set of jobs and their child jobs.
- **Tools** — an explicit allowlist of callable operations.
- **Memory** — scoped, per-agent state; not shared.
- **Budget** — credits, wall-clock, and CPU/GPU quota per root job.
- **Backend** — a model or policy engine behind a trait; a deterministic stub first, LLM backends added later.

An agent does not hold wallet seed phrases, node private keys, or settlement credentials.

### 10.2 Delegation

Every job belongs to a root job. Delegation creates a child job inside that root. The bounds from §9.3 apply, and the scheduler — not the agent — enforces them.

Delegation flow:

```text
Requester submits root job to Agent A (via control plane)
  Agent A plans and creates child job J1 (depth 1)
  A calls POST /jobs/J1/delegate
  Scheduler matches J1 against idle, capable agents
  Scheduler sends TaskProposal to Agent B
  B replies TaskBid (accept/reject, price, ETA)
  Scheduler issues TaskAward with a lease
  B runs J1 sandboxed; heartbeats progress
  B returns TaskResult with signed output + evidence
  A aggregates results and returns the root result
  Verifier checks; Accounting issues receipts for A and B
```

### 10.3 Idle detection

Do **not** trust an agent's idle claim. Combine:

- Node resource heartbeat (CPU, RAM, GPU, running jobs).
- Agent heartbeat (owned jobs, max concurrency, backend health).
- Lease: accepting a delegated job grants a time-bound lease.

Eligibility is a conjunction of tool availability, model/GPU/RAM, data policy, load, deadline, budget, and reliability policy.

### 10.4 Agent task state machine

Same enum as jobs, with `waiting_on_children` inserted:

```text
pending -> offered -> accepted -> running -> waiting_on_children -> succeeded
                                  |               |
                                  v               v
                                failed         cancelled
                                  |
                                  v
                               expired
```

Model this as a Rust enum with explicit transition functions, exactly like the job state machine.

### 10.5 Delegation protocol

Signed messages, canonical encoding, protocol version, and nonce on every message. Initial set in `mesh-agent-protocol`:

```text
AgentHello
AgentHeartbeat
AgentCapabilityAdvertisement
TaskProposal
TaskBid
TaskAward
TaskAccept
TaskProgress
TaskResult
TaskFailure
TaskCancel
DelegateTask
CancelDelegation
```

### 10.6 Verification of agent work

AI outputs vary in how easily they can be checked. Verification capability grows phase by phase:

1. **Deterministic / tool-verifiable** — code execution, math, file transforms, database queries, unit tests, WASM runs. Correctness reduces to a predicate a verifier can re-run without trusting the agent.
2. **Redundant execution** — two or more independent agents run the same deterministic-but-expensive subtask; results are compared bit-for-bit.
3. **Sampled / human acceptance** — for subjective outputs, used for high-value work.
4. **Staking and slashing** — economic deterrence for harder-to-verify work.

A receipt for an agent subtask includes:

- Agent ID, node ID, participant ID.
- Root job ID and child job ID.
- Accepted work units.
- Verification method and evidence hash.
- Transcript hash, tool-call log hash, model identifier, manifest hash.
- Reward-policy version and calculated amount.
- Issuance status.

Reward **accepted, verified subtask results** — never raw messages, heartbeats, "I did work" claims, or advertising.

### 10.7 Agent security rules

In addition to the general rules in §14:

1. Agent identity is not reputation. Keys prove control, not trustworthiness.
2. Agents never hold wallet seed phrases, node private keys, or settlement credentials.
3. Delegation is a bounded DAG: `max_depth`, `max_fanout`, shared budget, inherited deadline.
4. No unbounded recursion, self-delegation, or ancestor delegation.
5. Agents run sandboxed: no host filesystem, default-deny network, per-agent resource caps.
6. Tools are capability-based: an agent can only call tools explicitly granted in its manifest.
7. Task input is untrusted data, never trusted instructions. Prompt injection and tool poisoning are first-class threats.
8. Rate-limit and meter agents per participant and per node.
9. Collusion is expected. Use redundant verification for high-value deterministic tasks; do not rely on transcripts alone.
10. Idle claims are not proof; leases and heartbeats are.
11. Log every delegation and cancellation with signed envelopes for audit.
12. Prefer a deterministic stub backend for agent acceptance tests; add LLM backends once the surrounding machinery is sound.

---

## 11. Proof of Contribution (PoC)

MeshCloud rewards accepted contributions, not raw claims. This applies equally to jobs and agent tasks.

### Contribution pipeline

1. A job (possibly an agent task) is assigned with a unique ID, policy, and limits.
2. A worker or agent executes it and submits output plus usage evidence.
3. The verifier checks output correctness and whatever resource evidence is appropriate for the workload.
4. The system creates a unique contribution receipt.
5. Accounting calculates the reward using a versioned policy.
6. A settlement service submits approved receipts or batches to the blockchain.
7. Settlement status is reconciled against the chain; retries must not duplicate rewards.

### Candidate contribution types

| Type | Measure | Verification |
|---|---|---|
| CPU (deterministic) | Accepted task units or CPU-seconds | Re-execution, unit tests, tool checks |
| WASM subtask | Accepted results | Deterministic re-run, hash match |
| GPU | Verified inference/compute units | Defined per workload; nondeterministic results handled by redundant execution |
| Storage | Verified GB-hours | Proofs of ongoing availability and recoverability |
| Bandwidth | Validated delivered bytes | Proof against fabricated traffic and collusion |
| Relay | Validated useful traffic | Distinguish useful service from self-generated traffic |
| Agent subtask (deterministic) | Accepted results | Tool-checkable predicate; re-execution |
| Agent subtask (subjective) | Accepted results | Redundant execution, sampling, and staking policies |

### Reward formula

`reward = accepted_usage_units × workload_rate × bounded_quality_factor`

Define units, caps, rate versioning, rounding, and evidence requirements before issuing real tokens. Do not use a universal resource-hour as though one CPU hour, one GPU hour, and one GB-hour were interchangeable. Use integer or fixed-point arithmetic for amounts, never floating point.

### Receipts and settlement

A receipt includes:

- Unique receipt ID and job/attempt reference.
- Contributor identity (participant, node, and — if applicable — agent).
- Workload and accepted usage units.
- Verification method and evidence hash/reference.
- Reward-policy version and calculated amount.
- Issuance status and audit timestamps.

The contract prevents duplicate claims. For scale, approved receipts are batched with a Merkle root, with individual claims proven by Merkle proofs.

**Trust limitation:** a blockchain preserves the settlement decision; it does not automatically prove that computation happened or that an agent behaved correctly. If a single verifier can authorize arbitrary receipts, that verifier is trusted. Independent verification, dispute handling, rate limits, and audits are built in the later phases.

### Token strategy

Begin with non-transferable test credits and a local-chain token during development, and progress to a selected production chain as the verification and accounting layers mature. Assess applicable legal, tax, financial, and consumer-protection requirements before enabling real value.

---

## 12. Implementation phases

Phase boundaries are gates: each phase must satisfy its "Done when" list before the next phase begins, and each phase adds a working layer on top of the previous one. The sequence is cumulative — by the end, every capability described in this document is built.

### Phase status

| Phase | Name | Status |
|---|---|---|
| 0 | Repository and development environment | ✅ Complete |
| 1 | Simulated nodes | ✅ Complete |
| 2 | Job API and scheduler | ✅ Complete |
| 3 | Sandboxed execution | ✅ Complete |
| 4 | Proof of Contribution v1 | ✅ Complete |
| 5 | Blockchain settlement | ✅ Complete |
| 6 | Agent registry and messaging | ✅ Complete |
| 7 | Bounded delegation and collaboration | ✅ Complete |
| 8 | Agent contribution accounting | ✅ Complete |
| 9 | Participant accounts and marketplace | ✅ Complete |
| 10 | Distributed networking and control plane | ✅ Complete |
| 11 | Distributed storage | ✅ Complete |
| 12 | Public-network hardening | ✅ Complete |

---

### Phase 0 — Repository and development environment ✅ Complete

**Builds on:** nothing. This is the foundation.

**Build**

- Initialize the Cargo workspace, `rust-toolchain.toml`, and repository structure.
- Add configuration loading (`figment`), `tracing` structured logging, and health endpoints.
- Add Docker Compose and PostgreSQL.
- Add `sqlx` migration tooling and a `justfile`.
- Add `cargo test` / `cargo nextest` commands and integration tests using `testcontainers`.
- Add CI: `fmt`, `clippy -D warnings`, tests, `cargo audit`, `cargo deny`.
- Add a cross-platform build matrix (Linux, macOS, Windows) for the worker crate.

**Done when**

- A clean checkout starts with one documented command.
- Migrations run safely.
- Tests run without a chain or remote host.
- CI passes on all three target operating systems.

---

### Phase 1 — Simulated nodes ✅ Complete

**Builds on:** Phase 0. The workspace, database, and containers now carry real node lifecycle behavior.

**Build**

- Run one control plane and three worker containers.
- Give each worker a distinct, persistent Ed25519 identity.
- Register capabilities and signed resource offers with canonical encoding.
- Implement heartbeats and online/offline state.
- Add configurable failure simulation.
- Implement `mesh node ls` and `mesh node inspect`.

**Done when**

- Three workers register independently.
- Killing one worker marks it offline without marking others offline.
- Restarting a worker restores its identity and status.

---

### Phase 2 — Job API and scheduler ✅ Complete

**Builds on:** Phase 1. Nodes are known, so work can be routed to them.

**Build**

- Implement job submission, claiming, completion, and failure.
- Add atomic claims, leases, deadlines, and retry handling.
- Match jobs to worker capabilities and available capacity.
- Capture logs and result hashes.
- Introduce `mesh-scheduler` with a job state machine and explicit transitions.

**Done when**

- Jobs go to eligible workers.
- The same attempt cannot be concurrently claimed successfully by two workers.
- Jobs recover after a worker disappears.
- Retried work cannot automatically produce duplicate rewards.

---

### Phase 3 — Sandboxed execution ✅ Complete

**Builds on:** Phase 2. The scheduler assigns work; a worker can now run it safely.

**Build**

- Define a job manifest and input/output format.
- Introduce Wasmtime for WASM jobs in `mesh-runtime`.
- Apply memory limits (`StoreLimits`), fuel metering, and epoch-based interruption for wall-clock timeouts.
- Disable network access by default; grant no WASI preopened directories unless the manifest requires a scoped, temporary one.
- Run each job in a fresh store/instance; never share state between jobs.
- Run the runtime in a separate low-privilege process for defense in depth.

**Done when**

- Test jobs run and return reproducible results.
- Excessive resource use is terminated.
- Untrusted code cannot freely access host files, credentials, or other workloads.

---

### Phase 4 — Proof of Contribution v1 ✅ Complete

**Builds on:** Phase 3. Deterministic jobs produce results; those results become verified receipts and credit.

**Build**

- Introduce `mesh-verifier` with workload-specific verification policies (re-execution for deterministic WASM).
- Generate unique contribution receipts.
- Record evidence, accepted usage, and policy version.
- Add duplicate detection (unique constraints) and an auditable reward ledger in `mesh-accounting`.
- Establish local credit balances.
- Implement `mesh contribution ls`.

**Done when**

- Invalid outputs earn no rewards.
- Replaying a receipt does not create extra credit.
- Every reward traces back to an accepted job and verification record.

---

### Phase 5 — Blockchain settlement ✅ Complete

**Builds on:** Phase 4. Verified receipts exist and are now settled on-chain.

**Build**

- Select an EVM-compatible development environment (Anvil).
- Implement Solidity contracts using Foundry.
- Restrict reward authorization and define issuance rules.
- Prevent duplicate claims.
- Implement `mesh-settlement` behind a `Settlement` trait, with an EVM implementation using `alloy`.
- Add a settlement worker and reconciliation logic.
- Batch approved receipts with a Merkle root and support Merkle-proof claims.
- Implement `mesh wallet balance`.

**Done when**

- A verified test contribution earns the correct test tokens.
- Invalid signatures, unauthorized issuers, and duplicate claims fail.
- Restarting the settlement service does not duplicate rewards.
- Off-chain and on-chain settlement states reconcile after failures.

---

### Phase 6 — Agent registry and messaging ✅ Complete

**Builds on:** Phase 5. The compute and settlement layers are working; signed agent actors are added as a new kind of job owner.

**Build**

- Create `mesh-agent-protocol` with agent message types and canonical encoding.
- Add `agents`, `agent_capabilities`, and `agent_messages` tables.
- Implement agent registration and signed capability manifests.
- Implement agent heartbeats and load reporting.
- Add `mesh-agent-runtime` with a **deterministic stub backend** (no LLM yet).
- Wire a minimal agent host into `mesh-worker`.
- Add the registration, heartbeat, and listing endpoints from §9.2.
- Implement `mesh agent ls` and `mesh agent inspect`.

**Done when**

- Agents on different workers register independently.
- Killing a worker marks its agents offline; other agents are unaffected.
- Agent capability manifests verify against signed bytes.
- Restarting a worker restores agent identity from a restricted-permission key file.

---

### Phase 7 — Bounded delegation and collaboration ✅ Complete

**Builds on:** Phase 6. Agents exist; now they can split a root job and delegate deterministic subtasks.

**Build**

- Add agent-task support to `mesh-scheduler` (DAG, bounds, cascade cancellation).
- Add the `delegate`, `tree`, and `cancel` endpoints from §9.2.
- Implement `DelegateTask`, `TaskProposal`, `TaskBid`, `TaskAward`, `TaskAccept`, `TaskResult`, `TaskFailure`, and `TaskCancel`.
- Enforce cycle detection and depth, fanout, budget, and deadline limits in types and in database constraints.
- Add lease-based claims for agent tasks, mirroring the job scheduler.
- Run delegated subtasks through `mesh-runtime`.
- Implement message metering (§9.4).
- Implement `mesh job tree` and `mesh agent task submit`.

**Done when**

- A root job on Agent A splits into N subtasks, some delegated to Agents B and C.
- No task can be delegated to itself or to an ancestor.
- Depth and fanout limits are enforced; violating delegations are rejected.
- Cancelling a parent cancels all live children.
- Killing a delegatee mid-task causes retry or reassignment without duplicate effects.
- Message rate and byte limits are enforced per agent.

---

### Phase 8 — Agent contribution accounting ✅ Complete

**Builds on:** Phase 7. Delegated subtasks produce verifiable results that feed the same ledger as jobs.

**Build**

- Extend `contribution_receipts` with `subject_type = agent_task_attempt`.
- Add verification policies for deterministic agent subtasks (tool-checked outputs).
- Add redundant execution mode for higher-value deterministic subtasks.
- Add duplicate detection across root/child job IDs and attempt numbers.
- Credit both delegator and delegatee according to the reward policy.
- Implement `mesh contribution ls --agent <id>` and `mesh agent rewards <id>`.

**Done when**

- Every accepted agent subtask produces exactly one receipt.
- Replaying the same result or attempt does not create extra credit.
- Both delegator and delegatee are credited according to policy.
- A failed verification produces no receipt.

---

### Phase 9 — Participant accounts and marketplace ✅ Complete

**Builds on:** Phase 8. Contributions are accounted for; participants can now offer, request, and inspect work.

**Build**

- Associate participant accounts with wallets.
- Support signed resource offers and revocation.
- Filter available compute and agents by capabilities.
- Add resource reservations, quotas, and price estimates.
- Show contributions, rewards, and spending.
- Surface agent manifests and delegation history.

**Done when**

- One participant can offer resources and another can submit eligible work.
- Expired or revoked offers and manifests are not scheduled.
- Usage and reward history is inspectable per participant, node, and agent.

---

### Phase 10 — Distributed networking and control plane ✅ Complete

**Builds on:** Phase 9. The single-node control plane is replaced by a network of peers.

**Build**

- Add `rust-libp2p` peer identities and signed advertisements in `mesh-network`.
- Add gossip-based discovery and NAT traversal/relays.
- Introduce control-plane replicas and define which state requires consensus.
- Move worker and agent messaging onto the P2P transport behind a trait.
- Test coordinator failure, partitions, and recovery.

**Done when**

- Workers and agents on different physical machines can join securely.
- Nodes recover after network disruptions.
- Partitions do not create conflicting accepted jobs, agent tasks, or reward claims.

---

### Phase 11 — Distributed storage ✅ Complete

**Builds on:** Phase 10. The network supports peers; storage now uses them.

**Build**

- Encrypt objects client-side.
- Split objects into chunks and erasure-coded shards (for example, with a Reed-Solomon crate).
- Track placement, availability, and repair.
- Add storage retrieval challenges and workload-specific proofs.

**Done when**

- Objects remain recoverable within the configured fault tolerance.
- Missing redundancy is repaired.
- Storage rewards depend on verifiable availability, not just allocated disk space.

---

### Phase 12 — Public-network hardening ✅ Complete

**Builds on:** every prior phase. The system now operates safely with untrusted public participants.

**Build**

- Package native Rust workers and the agent host for Windows, macOS, and Linux (signed binaries, installers, auto-update strategy).
- Enforce owner-configured CPU, memory, storage, and bandwidth policies using per-OS mechanisms (cgroups on Linux, Job Objects on Windows, platform equivalents on macOS).
- Enforce per-agent resource caps and tool allowlists at the OS/runtime boundary.
- Add abuse reporting, network-egress controls, and stronger isolation.
- Reduce verifier centralization and define disputes and penalties.
- Add metrics, alerting, security review, and contract audits.
- Review every `unsafe` block and dependency; fuzz parsers and protocol decoders (`cargo-fuzz`), including agent message codecs.
- Run a public testnet and economic attack simulations, including prompt-injection and collusion scenarios.

**Done when**

- Workers and agents respect limits and can leave safely.
- Major attack scenarios have documented mitigations and tests.
- Production launch is gated on independent security and smart-contract review.

---

## 13. First end-to-end demo

Once Phases 0–5 are complete, the following workflow works end to end:

```bash
# Start the simulated network
docker compose up --build -d

# Inspect workers
mesh node ls

# Submit a test WASM job (WAT is compiled by the runtime; .wasm is accepted too)
mesh job submit --type wasm --file examples/hello.wat

# Inspect execution
mesh job status <job-id>
mesh job logs <job-id>

# Inspect accepted contribution
mesh contribution ls

# Check test-chain balance
mesh wallet balance
```

End-to-end acceptance criteria:

1. A job is submitted and assigned.
2. A worker executes it and submits the result.
3. The verifier accepts the output and produces a receipt.
4. Accounting calculates the reward.
5. The settlement worker submits the reward to the local chain.
6. The participant's test-token balance increases by the expected amount.
7. Replaying the same receipt does not increase the balance again.

Once Phases 6–9 are complete, the agent-mesh workflow works on the same foundation:

```bash
mesh agent ls
mesh agent task submit --agent agent-a --spec examples/tasks/factor.json
mesh job tree <root-job-id>
mesh contribution ls --agent agent-b
```

Agent-mesh acceptance: a root job is split into deterministic subtasks, delegated to an idle peer, executed in the sandbox, verified, and produces receipts for both the delegator and the delegatee — without double-paying on retries.

---

## 14. Non-negotiable engineering rules

1. **At-least-once execution; idempotent effects.** A job or agent task may run more than once. Its accounting effect must not.
2. **Identity is not reputation.** Possessing a key does not prove trustworthiness or resource ownership — for nodes or agents.
3. **Advertisements are not proof.** Reported capacity, uptime, and idle status do not automatically qualify for rewards.
4. **Protect private keys.** Workers and agents must not access wallet seed phrases or unrestricted settlement credentials.
5. **Version reward policies.** Each receipt must reference the exact policy used to calculate it.
6. **Expect failures.** Design rescheduling, storage repair, and settlement reconciliation from the start.
7. **Keep settlement replaceable.** Use an internal trait so the core scheduler does not depend on a particular chain.
8. **Do not expose the Docker socket to untrusted workloads.**
9. **Do not run arbitrary public jobs or agent code with unrestricted host or network access.**
10. **Audit before real value is involved.** Do not deploy public rewards or meaningful token value before security and economic review.
11. **No `unsafe` without justification.** Default to `#![forbid(unsafe_code)]`; every exception is documented and reviewed.
12. **Do not panic on untrusted input.** Network messages, agent messages, and job outputs are parsed into typed errors (`Result`), never `unwrap()`/`expect()` on external data.
13. **Never block the async runtime.** Run CPU-heavy or blocking work (including WASM execution and model inference) on dedicated threads or `spawn_blocking`, not on Tokio worker threads.
14. **Vet dependencies.** Keep the dependency tree small; gate CI on `cargo audit` and `cargo deny`.
15. **Bound every delegation.** `max_depth`, `max_fanout`, shared budget, inherited deadline, and cycle detection are mandatory. No exceptions.
16. **Treat agent inputs as untrusted data, not trusted instructions.** Prompt injection and tool poisoning are first-class threats.
17. **Agent identity is separate from node identity and from wallet identity.** Do not conflate them, and do not reuse key material across roles.
18. **One scheduler.** Agent tasks are jobs with children.
19. **No reward without verification.** If it cannot be verified deterministically or by a tool-checkable predicate, no reward is issued for it.
20. **Meter agent messaging.** No free chatter.

---

## 15. First sprint checklist

- [x] Initialize the Cargo workspace and pin the toolchain.
- [x] Create `mesh-protocol`, `mesh-identity`, `mesh-control-plane`, `mesh-worker`, and `mesh-cli` crates.
- [x] Add Docker Compose and PostgreSQL with `sqlx` migrations.
- [x] Start the control plane and three worker containers.
- [x] Implement node registration and persistent identity.
- [x] Implement signed resource offers with canonical encoding.
- [x] Implement heartbeats and expiry handling.
- [x] Build `mesh node ls` and `mesh node inspect`.
- [x] Test worker termination and restart recovery.
- [x] Set up CI (fmt, clippy, tests, audit, deny) with a Linux/macOS/Windows build matrix.
- [x] Document local setup and troubleshooting.

---

## 16. Project success criteria

MeshCloud is successful when it demonstrates:

- Reliable job recovery under worker churn.
- Safe, resource-limited execution of supported workloads.
- Verifiable, auditable contribution receipts.
- Correct and duplicate-resistant blockchain settlement.
- Secure participation by nodes on different physical machines.
- A working agent mesh where agents register, advertise capabilities, delegate deterministic subtasks to idle peers, and are credited for accepted work — under DAG bounds.
- Storage durability and repair.
- A documented threat model, economic model, and operational runbook covering nodes, agents, and the settlement path.

**Build order:** local simulation → scheduler → sandbox → verification → reward ledger → blockchain settlement → agent registry → bounded delegation → agent accounting → participant accounts → P2P network → distributed storage → native workers and public-network hardening.

The goal is a useful distributed cloud with a credible contribution economy — not a blockchain token attached to unverified resource claims, and not a swarm of unaccountable agents.

---

## 17. Target workloads

The network is built to run workloads that can be verified. The candidate classes below are the workloads the system is designed to schedule, verify, and settle.

| Workload | Why MeshCloud | Verifiable? |
|---|---|---|
| Blender / batch rendering | Cost per frame vs. cloud render farms | Yes — output frames are deterministic given a seed |
| Monte Carlo parameter sweeps | Embarrassingly parallel sweeps | Yes — results reproducible from a seed |
| Molecular dynamics sweeps | Cost vs. HPC allocation queues | Yes — deterministic given inputs |
| Cryptographic proving (zk) | Cost per proof | Yes — a proof verifies or it does not |
| Batch LLM inference (seeded) | Cost per token | Yes when the seed and model are fixed |
| Deterministic agent subtasks | Delegated tool-checked work | Yes — tool-checkable predicate |

Each workload added to the mesh is paired with a verification policy and a measured cost per accepted result. Appendix C lists the quantities to measure for each.

---

## Appendix A — The biggest bottlenecks

### 1. Verifying computation (critical)

A worker might claim it ran a job for 30 minutes, but it could have done nothing, returned fabricated results, or submitted someone else's work. An agent might claim it delegated and aggregated correctly.

**Solution:** deterministic workloads, independent verification, redundant execution for selected jobs and agent subtasks, and cryptographic receipts. Blockchain records accepted contributions; it does not independently prove that a computation was performed correctly.

### 2. Network reliability and churn (critical)

Home computers disconnect, sleep, restart, change IP addresses, and have inconsistent upload speeds. A job or agent task may disappear halfway through execution.

**Solution:** leases, heartbeats, checkpointing, retries, idempotent jobs, and replication for important workloads.

### 3. Splitting workloads effectively (critical)

Not every workload benefits from distributed computing. A CPU batch job can be split into independent tasks; a model-training job that constantly exchanges GPU data may perform terribly over home internet connections. Agent delegation makes this harder if unbounded — a runaway DAG can exhaust the mesh.

**Solution:** start with embarrassingly parallel workloads and bound agent delegation with `max_depth`, `max_fanout`, budget, and deadline.

### 4. Security between strangers (critical)

A malicious job or agent could try to steal data, access host files, abuse credentials, attack other workers, or poison peers via prompt injection and collusion.

**Solution:** sandboxed execution, strict resource limits, short-lived credentials, encrypted transport, capability-scoped tools, isolation that does not expose the Docker socket or host privileges, and redundant verification for high-value agent tasks.

### 5. Data movement and storage

Moving a 100 GB dataset to a volunteer machine can cost more time than the computation saves. Results must also be returned, validated, and stored reliably.

**Solution:** schedule jobs and agent tasks near their data, cache reusable inputs, transfer only necessary data, and add distributed storage with encryption and redundancy.

### 6. Economics and useful supply

Participants want rewards, but customers want cheap, reliable results. Verification, failed work, network transfer, redundant execution, and agent chatter all consume resources. A token does not automatically create demand for useful compute.

**Solution:** measure the actual cost per accepted result and compare it with centralized alternatives. Charge for agent messaging and delegation so the mesh cannot be spammed for free. Start with internal credits, then introduce on-chain settlement as the accounting becomes trustworthy.

### Where the real performance bottleneck depends on the workload

| Workload | Likely bottleneck |
|---|---|
| CPU batch jobs | Scheduling overhead and worker availability |
| AI inference | GPU VRAM, model loading, and request routing |
| Distributed AI training | Network latency and bandwidth |
| Rendering and simulations | Task distribution and transferring inputs/results |
| Distributed storage | Disk reliability, bandwidth, and data availability |
| Agent delegation | Scheduler churn, task-graph overhead, and redundant verification cost |

---

## Appendix B — What to build first

Prioritize the bottlenecks in this order:

1. **Reliable execution.** A job or agent task gets assigned, runs, fails safely if necessary, and can be retried without corrupting its result.
2. **Verifiable contribution accounting.** Every reward maps to an accepted result, with evidence, a unique receipt, and an auditable calculation — including for delegated agent subtasks.
3. **Secure execution.** Treat every worker, every agent, and every submitted job as potentially untrusted.
4. **Bounded agent delegation.** Depth, fanout, budget, and deadline limits, plus cycle detection, before any LLM is involved.
5. **Performance and scale.** Benchmark scheduling latency, useful work per second, failed-job rate, network overhead, and cost per accepted result.
6. **Blockchain and peer-to-peer expansion.** Add these on top of the working execution and accounting system.

---

## Appendix C — Cost model

These quantities are measured as the corresponding phases are built, so the economics of the network are understood at every step.

| Cost | Unit | Notes |
|---|---|---|
| Control-plane CPU per job | ms | Heartbeats + scheduling |
| Control-plane DB writes per job | rows | Job, attempt, receipt, settlement |
| Network overhead per job | KB | Protocol + result transfer |
| WASM execution overhead | % over native | Wasmtime vs native |
| Verification cost (re-execution) | × job cost | Typically 1× extra for deterministic re-run |
| Verification cost (redundant agents) | × subtask cost | 2–3× |
| Agent message overhead | bytes/msg | Proposal, bid, award, result |
| Storage cost per GB-period | — | For inputs/outputs |
| Settlement cost (local ledger) | ns | Negligible |
| Settlement cost (EVM) | gas/tx | Current gas × price |
| Reward per accepted unit | credits | Set by policy |

Benchmark before scaling. As each phase lands, the cost per accepted result is measured and compared against centralized alternatives for the target workloads in §17.
