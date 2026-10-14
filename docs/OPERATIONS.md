# MeshCloud Operations Runbook

## Starting the local network

```bash
just up                     # docker compose: postgres + control plane + 3 workers
cargo run -p mesh-cli -- health
cargo run -p mesh-cli -- node ls
```

## Health and readiness

- `GET /api/v1/health` — process liveness (never touches the DB).
- `GET /api/v1/ready` — dependency readiness (database).
- `GET /api/v1/metrics` — Prometheus text metrics: nodes/agents online, jobs by
  state, receipts issued/void, storage objects, abuse reports, mesh peers.

## Control-plane replicas and failover

- Set `MESH_REPLICA_ROLE=primary` on exactly one instance. Only the primary runs
  the background writers (offline sweep, job sweep, settlement, reservation
  expiry, storage challenge sweep).
- Set `MESH_REPLICA_ROLE=replica` on read-only instances (API + discovery).
- Failover is an operator action: promote a replica to `primary` and demote the
  old primary. Because strongly consistent state lives in one PostgreSQL, only
  one writer must be active at a time.

## Common tasks

- Inspect nodes/agents: `mesh node ls`, `mesh agent ls`, `mesh agent inspect <id>`.
- Inspect jobs: `mesh job ls`, `mesh job status <id>`, `mesh job logs <id>`,
  `mesh job tree <root-id>`.
- Contributions/rewards: `mesh contribution ls`, `mesh agent rewards <id>`,
  `mesh wallet balance --participant <id>`.
- Marketplace: `mesh marketplace nodes`, `mesh marketplace agents`, `mesh marketplace price`.
- Storage: `mesh storage put <file> --owner <id>`, `mesh storage ls`,
  `mesh storage get <id> <out>`, `mesh storage repair <id>`.

## Incidents

- **A worker misbehaves:** `mesh abuse report --subject-type node --subject <node-id> --category <c>`.
  Past `abuse_suspend_threshold` reports the node is decommissioned and its offer
  revoked, so it is no longer scheduled.
- **A receipt looks wrong:** `POST /api/v1/contributions/{id}/dispute` voids the
  receipt and reverses the credit; it is excluded from settlement.
- **Stale nodes:** the offline sweeper marks nodes (and their agents) offline
  after `node_offline_after_seconds` without a heartbeat.
- **Stuck jobs:** the job sweeper requeues attempts whose leases expired and
  expires jobs past their deadline.

## Resource policy (owner control)

Workers honor owner-configured caps:
- `MESH_MAX_CPU_PERCENT`, `MESH_MAX_MEMORY_MB` — per-workload caps.
- `MESH_FUEL_LIMIT`, `MESH_EXECUTION_TIMEOUT_SECONDS` — WASM CPU/time caps.
- `MESH_AGENT_MAX_MEMORY_MB`, `MESH_AGENT_FUEL_LIMIT`,
  `MESH_AGENT_EXECUTION_TIMEOUT_SECONDS` — per-agent caps.
- `MESH_STORAGE_GB` — storage offered.
- Bandwidth is bounded because jobs and agents have no network access by default.

## Leaving safely

A worker stops accepting new work and exits on `Ctrl-C` (SIGINT). In-flight
attempts finish before the process exits; its node is marked offline by the
sweeper if it does not return.

## Security operations

- Run `cargo audit` and `cargo deny check` (or `just audit`, `just deny`) before
  releases; both gate CI.
- Sign release binaries (see `scripts/sign-windows.ps1` for the local
  development certificate; production releases require a CA-issued certificate).
- Fuzz protocol decoders with `just fuzz` (requires nightly + cargo-fuzz).
