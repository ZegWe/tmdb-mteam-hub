import { readFileSync } from "node:fs";

const read = (path) => readFileSync(path, "utf8");
const failures = [];

function requireMatch(label, source, pattern) {
  if (!pattern.test(source)) {
    failures.push(`${label}: expected ${pattern}`);
  }
}

function rejectMatch(label, source, pattern) {
  if (pattern.test(source)) {
    failures.push(`${label}: rejected ${pattern}`);
  }
}

const dockerfile = read("Dockerfile");
const compose = read("deploy/nas/docker-compose.yml");
const quality = read(".github/workflows/quality.yml");
const publish = read(".github/workflows/docker-publish.yml");
const backup = read("docs/operations/backup-restore.md");
const upgrade = read("docs/operations/upgrade-rollback.md");
const nasGuide = read("deploy/nas/README.md");
const acceptance = read("scripts/ci/container-acceptance.sh");

requireMatch("Dockerfile frontend stage", dockerfile, /FROM node:[^\n]+ AS frontend/);
requireMatch("Dockerfile clean frontend install", dockerfile, /RUN --mount=[^\n]+ npm ci/);
requireMatch("Dockerfile frontend build", dockerfile, /RUN npm run build/);
requireMatch("Dockerfile backend stage", dockerfile, /FROM rust:[^\n]+ AS backend/);
requireMatch("Dockerfile locked backend build", dockerfile, /cargo build --release --locked/);
requireMatch("Dockerfile current state path", dockerfile, /SUBSCRIPTION_STATE_DIR=\/data\/state/);
requireMatch(
  "Dockerfile liveness URL",
  dockerfile,
  /HEALTHCHECK_URL=http:\/\/127\.0\.0\.1:8787\/healthz/,
);
requireMatch(
  "Dockerfile liveness directive",
  dockerfile,
  /^HEALTHCHECK[\s\S]+CMD curl[^\n]+HEALTHCHECK_URL/m,
);
rejectMatch("Dockerfile legacy migration executable", dockerfile, /COPY[^\n]+subscription-migrate/);
rejectMatch(
  "Dockerfile legacy migration or state reference",
  dockerfile,
  /subscription-migrate-v5|migrate_offline_v4|wanted\.sqlite|wanted_\*\.json/,
);

requireMatch("Compose versioned image input", compose, /TMDB_MTEAM_HUB_TAG:-latest/);
requireMatch("Compose loopback default", compose, /HOST_BIND_IP:-127\.0\.0\.1/);
requireMatch("Compose current state mount", compose, /\.\/state:\/data\/state/);
requireMatch("Compose TMDB cache mount", compose, /\.\/cache\/tmdb:\/data\/cache\/tmdb/);
requireMatch("Compose Douban cache mount", compose, /\.\/cache\/douban:\/data\/cache\/douban/);
requireMatch("Compose shared media mount", compose, /MEDIA_ROOT[^\n]+:\/srv\/media/);
requireMatch("Compose log size retention", compose, /max-size:/);
requireMatch("Compose log file retention", compose, /max-file:/);
rejectMatch("Compose legacy subscription store", compose, /wanted\.sqlite|wanted_\*\.json/);
rejectMatch(
  "Compose legacy migration entry point",
  compose,
  /subscription-migrate-v5|migrate_offline_v4|\bmigration\b/i,
);

requireMatch("Quality pull-request trigger", quality, /^\s+pull_request:\s*$/m);
requireMatch("Quality reusable trigger", quality, /^\s+workflow_call:\s*$/m);
requireMatch(
  "Container depends on all local quality jobs",
  quality,
  /container:[\s\S]+needs:[\s\S]+- rust[\s\S]+- frontend[\s\S]+- browser-e2e[\s\S]+- docs/,
);
requireMatch("Quality defines browser E2E job", quality, /^  browser-e2e:/m);
requireMatch("Browser E2E depends on frontend", quality, /browser-e2e:[\s\S]+needs: frontend/);
requireMatch(
  "Browser E2E installs Chromium and Firefox",
  quality,
  /playwright install --with-deps chromium firefox/,
);
requireMatch("Quality builds an image", quality, /docker\/build-push-action@v6/);
requireMatch("Quality verifies backend boundaries", quality, /verify-backend-boundaries\.sh/);
requireMatch(
  "Quality rejects a legacy migration runtime artifact",
  quality,
  /test ! -e \/usr\/local\/bin\/subscription-migrate-v5/,
);
requireMatch(
  "Quality runs container restore acceptance",
  quality,
  /scripts\/ci\/container-acceptance\.sh/,
);

requireMatch(
  "Publication calls reusable quality workflow",
  publish,
  /uses: \.\/\.github\/workflows\/quality\.yml/,
);
requireMatch("Publication waits for quality", publish, /build-and-push:[\s\S]+needs: quality/);
requireMatch("Publication pushes immutable SHA tag", publish, /type=sha,prefix=sha-/);
requireMatch("Publication pushes release tag", publish, /type=ref,event=tag/);

requireMatch("Runbook stops writers before backup", backup, /docker compose stop/);
requireMatch("Runbook backup is fail-fast", backup, /set -Eeuo pipefail/);
requireMatch("Runbook copies current config", backup, /config\/config\.toml/);
requireMatch("Runbook copies current SQLite", backup, /state\/subscriptions\.sqlite/);
rejectMatch("Runbook whole-directory backup", backup, /cp -a config state/);
requireMatch(
  "Runbook rejects SQLite sidecars",
  backup,
  /subscriptions\.sqlite-(?:wal|shm|journal)/,
);
requireMatch("Runbook restores config mode", backup, /chmod 600 config\/config\.toml/);
requireMatch(
  "Runbook creates backup config as 0600",
  backup,
  /install -m 600 config\/config\.toml/,
);
requireMatch("Runbook declares legacy state unsupported", backup, /不提供数据库迁移或导入/);
requireMatch("NAS guide copies current config", nasGuide, /cp -p config\/config\.toml/);
requireMatch("NAS guide copies current SQLite", nasGuide, /cp -p state\/subscriptions\.sqlite/);
rejectMatch("NAS guide whole-directory backup", nasGuide, /cp -a config state/);
requireMatch("Rollback pins image tag or digest", upgrade, /镜像标签或 digest/);
requireMatch("Rollback restores current SQLite", upgrade, /subscriptions\.sqlite.*备份/s);

requireMatch("Acceptance verifies schema version", acceptance, /schema_version/);
requireMatch("Acceptance verifies subscription count", acceptance, /subscription_count/);
requireMatch("Acceptance seeds non-empty restore evidence", acceptance, /seed_acceptance_log/);
requireMatch("Acceptance records verification time", acceptance, /verified_at=/);
requireMatch("Acceptance records image reference", acceptance, /image_reference=/);
requireMatch("Acceptance records image ID", acceptance, /image_id=/);
requireMatch("Acceptance records image digests", acceptance, /repo_digests=/);
requireMatch("Acceptance records restored config mode", acceptance, /restored config mode/);
requireMatch("Acceptance checks SQLite integrity", acceptance, /PRAGMA integrity_check/);
requireMatch("Acceptance tests readiness", acceptance, /\/readyz/);
requireMatch("Acceptance tests static page", acceptance, /\.index\.html/);
requireMatch(
  "Acceptance preserves legacy SQLite sentinel",
  acceptance,
  /source_legacy_sqlite_sha256/,
);
requireMatch("Acceptance preserves legacy JSON sentinel", acceptance, /source_legacy_json_sha256/);
requireMatch("Acceptance restores into a clean root", acceptance, /restored_root/);
requireMatch(
  "Acceptance backs up only current config",
  acceptance,
  /cp -p "\$source_root\/config\/config\.toml" "\$backup_root\/config\/config\.toml"/,
);
requireMatch(
  "Acceptance backs up only current SQLite",
  acceptance,
  /cp -p "\$source_root\/state\/subscriptions\.sqlite" "\$backup_root\/state\/subscriptions\.sqlite"/,
);
requireMatch(
  "Acceptance fixes the backup payload at two files",
  acceptance,
  /find "\$backup_root" -type f \| wc -l\)" -eq 2/,
);
requireMatch(
  "Acceptance excludes legacy SQLite from backup",
  acceptance,
  /test ! -e "\$backup_root\/state\/wanted\.sqlite"/,
);
requireMatch(
  "Acceptance excludes legacy JSON from backup",
  acceptance,
  /test ! -e "\$backup_root\/state\/wanted_ci\.json"/,
);
requireMatch(
  "Acceptance excludes legacy SQLite from restore",
  acceptance,
  /test ! -e "\$restored_root\/state\/wanted\.sqlite"/,
);
requireMatch(
  "Acceptance excludes legacy JSON from restore",
  acceptance,
  /test ! -e "\$restored_root\/state\/wanted_ci\.json"/,
);
rejectMatch(
  "Acceptance copies a whole source config/state directory",
  acceptance,
  /cp[^\n]*"\$source_root\/(?:config|state)"(?:\s|$)/,
);

if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log("delivery contracts verified: Docker, Compose, CI publication, backup, and restore");
