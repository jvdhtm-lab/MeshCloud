# MeshCloud

**A volunteer-computing cloud with verifiable contributions and a bounded agent mesh — written in Rust.**

MeshCloud lets participants share spare CPU, RAM, storage, and bandwidth, and earn rewards for **verified** contributions. On top of the compute network, a mesh of AI agents can split work into deterministic subtasks and delegate them to idle peers under strict, auditable bounds.

> The blockchain rewards verified contributions. MeshCloud performs scheduling, execution, verification, and storage. Agents propose, delegate, and collaborate — within bounds set by MeshCloud. **Nothing is rewarded unless it is verified.**

[![CI](https://github.com/jvdhtm-lab/MeshCloud/actions/workflows/ci.yml/badge.svg)](https://github.com/jvdhtm-lab/MeshCloud/actions/workflows/ci.yml)

---

## Highlights

- **Reliable execution under churn** — atomic lease-based job claims (`FOR UPDATE SKIP LOCKED`), retries, expiry, and idempotent completion.
- **Sandboxed execution** — Wasmtime with memory limits, fuel metering, epoch-based timeouts, no WASI, no network, and optional per-job process isolation.
- **Proof of Contribution** — results are verified by independent re-execution; only reproduced results earn receipts. Receipts are unique per `(attempt, role)`.
- **Blockchain settlement** — Merkle-batched rewards settled on-chain via `alloy` and the audited-by-design `MeshReward` contract, with duplicate-claim protection.
- **Agent mesh** — signed agent identities and capability manifests; agent tasks are jobs with a DAG (`root_job_id`/`parent_job_id`/depth/fanout/budget/deadline), enforced in types and in the database.
- **Distributed storage** — client-side AES-256-GCM encryption, Reed-Solomon erasure coding, retrieval challenges, repair, and availability-based rewards.
- **Distributed networking** — libp2p peer identities, signed node advertisements, gossipsub discovery, and a single-writer control-plane replica model.
- **Hardening** — per-agent capability allowlists and resource caps, abuse reporting with auto-quarantine, contribution disputes, Prometheus metrics, and fuzzed protocol decoders.

## Architecture

```text
Participant
├── Wallet / cryptographic identity
└── mesh-worker (native Rust daemon)
        └── Agent runtime host (zero or more AI agents)
              │
              v
Control plane (Rust, Axum)          Mesh (libp2p)
├── Node & agent registry           ├── Signed advertisements
├── Scheduler (jobs, leases, DAGs)  └── Gossip discovery
├── Sandboxed executor (Wasmtime)
├── Verification (re-execution)
├── Contribution accounting
└── Settlement (Settlement trait) ──> MeshReward.sol (EVM)
```

## Quick start (Docker)

One command starts PostgreSQL, the control plane, and three workers:

```bash
docker compose up --build -d
```

Open the dashboard in your browser:

```
http://localhost:8080/
```

> If host ports are busy, edit the mappings in `docker-compose.yml`. The app
> uses the internal Docker network, so only the control-plane port matters to you.

Submit a job and watch it flow through the mesh:

```bash
cargo run -p mesh-cli -- node ls
cargo run -p mesh-cli -- job submit --type wasm --file examples/hello.wat
cargo run -p mesh-cli -- job ls
cargo run -p mesh-cli -- contribution ls
```

Stop and remove volumes:

```bash
docker compose down -v
```

## Dashboard

The control plane serves a self-contained web dashboard (no build step) at `/`:

- live health, replica role, and mesh peers;
- node and agent tables (including **agents per node**);
- jobs, contributions, settlements, and storage objects;
- Prometheus metrics.

## Local development

```bash
# 1. Start PostgreSQL only
docker compose up -d mesh-postgres

# 2. Run the control plane (applies migrations on start)
cargo run -p mesh-control-plane

# 3. Run one or more workers
cargo run -p mesh-worker
```

## Repository layout

```text
crates/
  mesh-protocol/        Shared messages, IDs, lifecycle types, canonical encoding
  mesh-identity/        Ed25519 node & agent identity, signing, persistence
  mesh-scheduler/       Placement, atomic claims, leases, retries, delegation DAGs
  mesh-runtime/         Wasmtime sandbox and process isolation
  mesh-verifier/        Verification by re-execution
  mesh-accounting/      Contribution receipts, credit ledger, reservations
  mesh-settlement/      Settlement trait, local ledger, EVM (alloy) backend
  mesh-agent-protocol/  Agent manifests, messages, API types
  mesh-agent-runtime/   Agent backends (deterministic stub)
  mesh-network/         libp2p peer identity, advertisements, discovery, replicas
  mesh-storage/         AES-GCM encryption, erasure coding, retrieval challenges
  mesh-control-plane/   HTTP API, dashboard, coordination (binary)
  mesh-worker/          Native worker daemon (binary)
  mesh-cli/             `mesh` command-line interface (binary)
contracts/              Foundry project (MeshReward.sol)
migrations/             sqlx migrations (embedded at compile time)
docs/                   Threat model, operations runbook, security review
examples/               Example WASM/WAT workloads
fuzz/                   cargo-fuzz targets for protocol decoders
```

## Testing

```bash
cargo test --workspace            # unit + router tests, no external services
just test-integration             # Docker-backed integration tests (testcontainers)
forge test --root contracts       # Solidity contract tests
just check                        # fmt + clippy -D warnings + tests
just audit && just deny           # supply-chain checks
```

CI runs `rustfmt`, `clippy`, tests on Linux/macOS/Windows, `cargo-audit`, and `cargo-deny`.

## Configuration

Precedence: built-in defaults → `configs/*.toml` → `MESH_*` environment variables
(nested keys use a double underscore, e.g. `MESH_LOG__LEVEL=debug`).

| Variable | Default | Purpose |
|---|---|---|
| `MESH_BIND_ADDR` | `0.0.0.0:8080` | Control-plane HTTP bind address |
| `MESH_DATABASE_URL` | local Postgres | PostgreSQL connection string |
| `MESH_REPLICA_ROLE` | `primary` | `primary` runs writers; `replica` is read-only |
| `MESH_SETTLEMENT_BACKEND` | `local` | `local` ledger or `evm` (needs `--features evm`) |
| `MESH_REWARD_RATE_PER_UNIT` | `1` | Credits per accepted unit |
| `MESH_DELEGATOR_REWARD_RATE_PER_UNIT` | `1` | Credits per unit to a delegating agent |
| `MESH_PARTICIPANT_QUOTA_CREDITS` | `0` | Max credits a participant may reserve (0 = unlimited) |
| `MESH_STORAGE_DATA_SHARDS` / `MESH_STORAGE_PARITY_SHARDS` | `4` / `2` | Erasure-coding layout |
| `MESH_NETWORK_LISTEN` | unset | libp2p listen multiaddr (needs `--features libp2p`) |

### Optional features

The default build stays small. Two capabilities are opt-in cargo features:

```bash
cargo run -p mesh-control-plane --features evm      # on-chain settlement (alloy)
cargo run -p mesh-worker       --features libp2p    # libp2p mesh transport
```

## Security

MeshCloud treats every participant, worker, agent, and job as untrusted. See:

- [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md) — assets, trust boundaries, and mitigations.
- [`docs/OPERATIONS.md`](docs/OPERATIONS.md) — runbook, replica failover, incident response.
- [`docs/SECURITY_REVIEW.md`](docs/SECURITY_REVIEW.md) — pre-launch review checklist.

A production launch is gated on independent security and smart-contract review.

## Roadmap

The project was built in cumulative phases; all are implemented.

| Phase | Area | Status |
|---|---|---|
| 0 | Repository and development environment | ✅ |
| 1 | Simulated nodes | ✅ |
| 2 | Job API and scheduler | ✅ |
| 3 | Sandboxed execution | ✅ |
| 4 | Proof of Contribution v1 | ✅ |
| 5 | Blockchain settlement | ✅ |
| 6 | Agent registry and messaging | ✅ |
| 7 | Bounded delegation and collaboration | ✅ |
| 8 | Agent contribution accounting | ✅ |
| 9 | Participant accounts and marketplace | ✅ |
| 10 | Distributed networking and control plane | ✅ |
| 11 | Distributed storage | ✅ |
| 12 | Public-network hardening | ✅ |

See [`MeshCloud_Project_Blueprint.md`](MeshCloud_Project_Blueprint.md) for the full design.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
