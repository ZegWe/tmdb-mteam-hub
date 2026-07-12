---
status: accepted
owner: tmdb-mteam-hub
date: 2026-07-06
last_verified: 2026-07-11
supersedes: docs/archive/superpowers/specs/2026-07-02-detail-url-state-design.md
---

# ADR 0001：独立详情路由

## 背景

最初设计把详情状态编码成列表页 query，并继续使用固定 drawer。这能支持浏览器 Back，但详情
内容仍与列表页面生命周期和大模板耦合。

## 决策

使用 Vue Router hash history 的独立路径：

- `#/detail/:mediaType/:id`
- `#/subscriptions/:id`

详情作为应用内容区中的独立页面，保留左侧导航。路由参数是加载详情的权威输入；从一个详情
切换到另一个详情使用 replace，首次打开使用 push，返回时回到列表路由。

媒体详情和订阅详情模板拆分为聚焦组件，页面/应用壳负责路由同步和数据加载。这些路由现已由
真实 lazy page components 和 `RouterView` 承载，URL 契约保持不变。

## 结果

- 深链和刷新不依赖列表页先存在。
- 浏览器 Back 语义清晰。
- 详情不再使用固定 drawer CSS。
- 搜索和订阅列表的返回状态需要由 URL 或 feature store 明确保存。

## 被替代设计

[Detail Drawer URL State Design](../archive/superpowers/specs/2026-07-02-detail-url-state-design.md)
仅保留为历史背景，不再描述当前路由架构。

实施记录见
[Standalone Detail Page Implementation Plan](../superpowers/plans/2026-07-06-standalone-detail-page.md)。
