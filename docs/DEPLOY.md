# AuditNetwork — Deploy & Operations

This repository is **cloud-agnostic by design** — the runtime is a single Rust
binary in a Docker image plus a SQLite file on a local disk. Provisioning is
Terraform per provider; promotion is a manual GitHub Actions workflow.

> **Local-first caveat.** Live-tailing only sees `~/.claude/projects/` on the
> machine running `auditnetwork`. A cloud deployment is for *ingested* or
> *uploaded* transcripts (team replay / audit warehouse), not for watching a
> remote user's live session.

## Targets

| Provider | Folder | Default instance | Approx €/mo |
|---|---|---|---|
| Hetzner  | `infra/terraform/hetzner/`  | `cax11` (4 ARM / 8 GB / 40 GB) | ~€4.30 |
| OVH      | `infra/terraform/ovh/`      | `d2-2` (1 / 2 GB)  | ~€5    |
| Scaleway | `infra/terraform/scaleway/` | `PLAY2-NANO` (1 / 2 GB) | ~€5 |

Hetzner is the recommended first target.

## 0 — Prerequisites

- A Docker image at `ghcr.io/<owner>/auditnetwork:<tag>` (produced by
  `.github/workflows/release.yml` on tag push).
- A DNS provider you control. **Cloudflare** is recommended so DNS isn't
  coupled to any of the hyperscalers — this is the main thing that keeps
  pivots cheap.
- SSH keypair: `ssh-keygen -t ed25519 -C deploy@auditnetwork`. The public
  half goes into `terraform.tfvars`; the private half goes into the
  GitHub Actions `DEPLOY_SSH_KEY` secret.

## 1 — Provision (Hetzner, first deploy)

```bash
cd infra/terraform/hetzner
cp terraform.tfvars.example terraform.tfvars
$EDITOR terraform.tfvars                      # set domain + ssh_authorized_keys
export HCLOUD_TOKEN=...                       # https://console.hetzner.cloud/projects → API tokens
terraform init
terraform apply
# Outputs include ipv4_address. Point your DNS A record at it.
```

## 2 — Configure GitHub for deploy.yml

In repo Settings → Secrets and variables → Actions:

- `DEPLOY_SSH_KEY` — private key matching the public key in tfvars.
- `HETZNER_HOST`   — the public IPv4 (or hostname) of the box.
- `OVH_HOST`, `SCALEWAY_HOST` — fill in only as you stand them up.

Then in Settings → Environments, create `hetzner` (and `ovh`, `scaleway`
later) — optionally with required reviewers as a soft "are you sure".

## 3 — First deploy

```bash
git tag v0.1.0 && git push origin v0.1.0
# .github/workflows/release.yml runs: builds binaries, builds multi-arch image,
# pushes ghcr.io/<owner>/auditnetwork:{v0.1.0,latest}, creates GH Release.
```

Then from the Actions tab → `deploy` → Run workflow:
- target: `hetzner`
- image_tag: `v0.1.0` (or `latest`)

The workflow SSHes in, points compose at the new tag, restarts the service,
and asserts `/healthz` is green.

## 4 — Pivot to another provider (≈ minutes of downtime)

Because state is just a SQLite file and a DNS record, pivoting is cheap:

```bash
# 1. Stand up the new box.
cd infra/terraform/scaleway     # or ovh
cp terraform.tfvars.example terraform.tfvars
$EDITOR terraform.tfvars        # same domain, same ssh key
terraform init && terraform apply

# 2. (Optional) Pause writes on the old box for a clean copy.
ssh deploy@$OLD "sudo systemctl stop auditnetwork"

# 3. Copy the DB.
rsync -avz --progress \
  deploy@$OLD:/var/lib/auditnetwork/audit.db \
  /tmp/audit.db
rsync -avz /tmp/audit.db deploy@$NEW:/tmp/audit.db
ssh deploy@$NEW "sudo install -o 10001 -g 10001 -m 0600 /tmp/audit.db /var/lib/auditnetwork/audit.db && sudo systemctl restart auditnetwork"

# 4. Flip DNS (Cloudflare or your DNS host).
#    Lower the TTL to 60s an hour beforehand to make the cutover fast.

# 5. Verify, then decom the old box.
curl -fsS https://audit.example.com/healthz
terraform -chdir=infra/terraform/hetzner destroy   # only after the new box is verified
```

What makes pivots cheap:
- **No managed databases.** SQLite is one file; rsync is the migration.
- **No provider-specific blob storage.** All state is on the local volume.
- **No provider-specific compute primitives.** Just Docker + systemd + Caddy.
- **Cloudflare for DNS.** Provider DNS would re-couple us at the apex.

What would make them painful — and is therefore avoided in the
architecture: cloud-managed Postgres, vendor-locked object stores as the
primary record, Cloud Run / Lambda / Functions, provider Kubernetes
flavours, provider DNS for the apex.

## 5 — Backups

The default cloud-init does **not** configure backups; for production:

- Cheapest: enable Hetzner backups (+20% of plan price ≈ €0.85/mo) — they
  snapshot the entire VM nightly. For Scaleway, set
  `scaleway_instance_server.an.enable_dynamic_ip = false` and attach a
  `scaleway_instance_snapshot`. For OVH, use the manager's auto-snapshot.
- Better: nightly `litestream replicate /var/lib/auditnetwork/audit.db s3://...`
  to Scaleway Object Storage (€0.012/GB — cheapest S3 in the EU). This also
  un-couples DR from any single hyperscaler.

Setting up litestream is a follow-up; the volume snapshot is sufficient
for a v1.

## 6 — Observability (minimal)

- `journalctl -u auditnetwork -f` for logs.
- `curl https://audit.example.com/healthz`, `…/readyz` from any uptime
  monitor (UptimeRobot is free).
- The product itself is observability for Claude Code — eat your own dog
  food and ingest the box's own transcripts to monitor it.
