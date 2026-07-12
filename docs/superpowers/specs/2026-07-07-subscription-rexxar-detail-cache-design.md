---
status: implemented
owner: tmdb-mteam-hub
last_verified: 2026-07-11
implementation_plan: docs/superpowers/plans/2026-07-07-subscription-rexxar-detail-cache.md
related_adr: docs/adr/0002-subscription-state-convergence.md
---

# Subscription Rexxar Detail Cache Design

> Historical behavior design: the displayed metadata behavior remains current, while the old watcher,
> aggregate record and store references below have been superseded by the latest-only architecture in
> the [architecture convergence PRD](./2026-07-11-project-architecture-convergence-prd.md).

## Goal

Subscription cards should show only the Chinese title and use the movie release date as the second line. When polling Douban wanted items, the server should cache richer Douban rexxar subject metadata so subscription detail pages can show more media information without opening each Douban detail on demand.

## Scope

- Keep the subscription card poster style from the previous change.
- Card title uses `record.title || record.subject_id` only.
- Card subtitle uses `record.date_published` first, then `record.release_year`; it must not use `douban_date`.
- During wanted polling, fetch rexxar subject details for wanted items and persist a best-effort detail cache on each subscription record.
- Reuse cached detail fields in subscription detail rows: release date, rating, original title, aliases, genres, countries, languages, directors, actors, duration, and summary.
- If a rexxar detail fetch fails for a specific item, keep processing the wanted list with base list data.

## Architecture

`douban.rs` exposes a rexxar-only detail fetcher that reuses the existing parser without the extra authenticated HTML interest lookup. `main.rs` calls it during `run_wanted_watch_poll`, builds a subject-id keyed detail map, and passes that map into the subscription store.

`WantedSubscriptionRecord` gains serialized optional detail-cache fields. The subscription store copies those fields when creating a record and refreshes non-empty values on later polls. Existing records remain readable because all new fields have serde defaults.

The frontend changes only formatting: card subtitle reads release date fields, and `subscriptionDetailRows()` includes cached media detail rows before operational state rows.

## Testing

- Rust tests cover record creation and refresh with cached Douban subject detail fields.
- Frontend subscription card tests cover title-only cards and release-date subtitles.
- Frontend detail tests cover cached media rows in subscription detail.
