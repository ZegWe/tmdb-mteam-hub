# NAS Docker Compose 部署

目标目录：

```bash
/volume1/docker/tmdb-mteam-hub
```

目录结构：

```text
/volume1/docker/tmdb-mteam-hub/
  docker-compose.yml
  config/
    config.toml
  cache/
```

启动或更新：

```bash
cd /volume1/docker/tmdb-mteam-hub
docker compose pull
docker compose up -d
```

应用监听端口是 `8787`，配置文件、TMDB/豆瓣缓存和订阅状态都会保存在容器外。
