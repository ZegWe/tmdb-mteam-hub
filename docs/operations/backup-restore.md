---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-12
---

# 备份与恢复

## 备份范围

当前版本恢复所需的权威文件只有：

- `config/config.toml`
- `state/subscriptions.sqlite`

`config/` 中由安全规范化留下的历史备份可以按运维策略另行归档，但不能代替当前
`config.toml`。

可以不备份：

- `cache/tmdb/`
- `cache/douban/`

媒体文件由 NAS 现有媒体备份策略负责。

## 停机备份

当前推荐在容器停止时复制 SQLite，避免得到不一致快照：

```bash
set -Eeuo pipefail
cd /volume1/docker/tmdb-mteam-hub
stopped=0
restart_service() {
  if [ "$stopped" -eq 1 ]; then
    docker compose start
  fi
}
trap restart_service EXIT

docker compose stop
stopped=1
backup_dir="backups/$(date +%Y%m%d-%H%M%S)"
umask 077
mkdir -p "$backup_dir/config" "$backup_dir/state"
test ! -e state/subscriptions.sqlite-wal
test ! -e state/subscriptions.sqlite-shm
test ! -e state/subscriptions.sqlite-journal
install -m 600 config/config.toml "$backup_dir/config/config.toml"
cp -p state/subscriptions.sqlite "$backup_dir/state/subscriptions.sqlite"
test -s "$backup_dir/config/config.toml"
test -s "$backup_dir/state/subscriptions.sqlite"
docker compose start
stopped=0
trap - EXIT
```

确认备份包含非空的 `config/config.toml` 和 `state/subscriptions.sqlite`：

```bash
test -s "$backup_dir/config/config.toml"
test -s "$backup_dir/state/subscriptions.sqlite"
```

建议保留 7 份日备份和 4 份周备份，并把至少一份复制到不同存储设备。

若任一 SQLite sidecar 检查失败，不要继续用普通文件复制制作备份。先确认所有服务实例和其他
writer 已停止，再调查未完成事务；当前版本没有受支持的在线备份命令。

## 旧状态文件

当前版本不提供数据库迁移或导入。生产服务不会枚举、探测或打开 `wanted.sqlite`、
`wanted_*.json`，也不会读取、转换、修改或删除它们。需要留档时可以在服务外单独复制到归档
目录；它们不是恢复当前版本所需的文件，也不能替代 `subscriptions.sqlite`。

## 恢复

1. 停止容器。
2. 把当前 `config/`、`state/` 移到隔离目录，不要直接覆盖后删除。
3. 创建干净的 `config/`、`state/`，从同一时间点的备份恢复上述两个权威文件；不要把缓存或旧
   状态文件混入恢复目录。
4. 恢复运行 UID/GID 的属主和 `config.toml` 的 `0600` 权限。
5. 使用生成该 `subscriptions.sqlite` 的当前版本镜像启动。

示例：

```bash
set -Eeuo pipefail
docker compose stop
mv config "config.failed.$(date +%s)"
mv state "state.failed.$(date +%s)"
mkdir -p config state cache/tmdb cache/douban
install -m 600 backups/20260711-120000/config/config.toml ./config/config.toml
cp -p backups/20260711-120000/state/subscriptions.sqlite ./state/subscriptions.sqlite
chown -R 1026:100 config state
chmod 600 config/config.toml
docker compose up -d
```

## 恢复验证

- 容器保持运行且日志没有配置解析或 SQLite schema 错误。
- 首页可以加载。
- 订阅数量、最近操作日志和关键状态与备份时一致。
- 缓存可以为空并由后续请求重建。

仓库中的 `scripts/ci/container-acceptance.sh` 把这套停机备份流程应用到临时源部署，再从只含
`config.toml` 和 `subscriptions.sqlite` 的备份启动一个干净恢复部署。CI 会记录并比较 SQLite
完整性、schema 版本、订阅数和操作日志数，同时验证配置权限、`/healthz`、`/readyz`、静态首页，
以及源部署旁边的旧 SQLite/JSON sentinel 字节不变。该脚本在 GitHub-hosted Docker 上首次成功前，
只能证明 acceptance 已实现，不能作为真实容器恢复演练已经通过的证据。

升级与版本回滚见[升级与回滚](upgrade-rollback.md)。
