# Subscription Cover Cards Design

## Goal

The subscriptions page should use the same cover-image-and-name card style as the search page. The search page should no longer expose the separate Douban library list entry, while keeping TMDB and Douban search.

## Scope

- Remove the search page header action that opens the Douban library list.
- Keep the search source tabs for TMDB and Douban search.
- Change subscription list cards to poster-oriented cards that show a cover image, title, status, and concise secondary text.
- Keep subscription actions in the detail page instead of on the cover card.
- Persist Douban poster fields on subscription records so the subscription page can render covers without per-card detail requests.

## Architecture

The backend subscription record gains optional `poster_url` and `cover_url` fields. The wanted-list poll path copies those fields from `DoubanLibraryItem` when creating records and refreshes them when records already exist. Existing records without covers remain valid and are filled on the next poll.

The frontend reuses `itemImageUrl()` for subscription records and renders subscription cards in a poster grid. Detail routing and auto-sync keep using `subject_id`, so card clicks and standalone subscription detail pages remain unchanged.

## Testing

- A Rust unit test verifies newly created and refreshed wanted subscription records preserve Douban cover URLs.
- The subscription card display test verifies the template uses an image, title, status, and no inline retry/rerun buttons.
- The search card display test verifies the search page no longer includes the Douban library header action or library view branch.
