# Detail Drawer URL State Design

## Goal

Opening a media card on the search page or a subscription card on the subscriptions page should update the URL so browser Back closes the detail drawer instead of leaving the page. The URL should also be shareable and refresh-safe: loading a detail URL should restore the same drawer when possible.

## Chosen Approach

Use route query parameters to represent drawer state within the existing hash router.

- Search page media detail URLs:
  - `#/?detail=movie&id=<tmdb_id>`
  - `#/?detail=tv&id=<tmdb_id>`
  - `#/?detail=douban&id=<douban_subject_id>`
- Subscription detail URLs:
  - `#/subscriptions?detail=subscription&id=<douban_subject_id>`

This keeps the existing page routes unchanged and avoids a broader router-view refactor.

## Behavior

- Clicking a search result card pushes the current page route with `detail` and `id` query parameters.
- Clicking a subscription card pushes `/subscriptions` with `detail=subscription` and the record `subject_id`.
- A route watcher owns opening and closing the drawer:
  - valid media detail query opens the media drawer and loads the detail API.
  - valid subscription detail query opens the subscription drawer after subscription data is available.
  - missing or invalid detail query closes and resets the drawer.
- Closing the drawer removes only detail-related query parameters.
- Browser Back after clicking a card returns to the previous URL without detail query, causing the drawer to close.
- Directly opening or refreshing a detail URL restores the drawer. If the referenced record cannot be found or the detail API fails, the drawer shows the existing error state and can be closed.

## Data Flow

Card click handlers become route writers instead of directly mutating drawer state.

Route query changes drive drawer state:

1. Normalize `route.query.detail` and `route.query.id`.
2. For media detail, call the existing detail loader with the normalized type and ID.
3. For subscription detail, ensure subscriptions are loaded, then select the matching record by `subject_id`.
4. If the selected subscription record changes during auto-sync, preserve the selected ID and refresh `selectedSubscription` from the latest records.

## Components and Functions

- Add a small route-query helper for detail parameters.
- Split direct detail-opening functions into:
  - route push helpers used by card clicks.
  - internal drawer loaders used by the route watcher.
- Keep the existing `<aside id="detail">` UI and detail rendering unchanged.

## Error Handling

- Invalid or incomplete query parameters close the drawer instead of throwing.
- Missing subscription records after loading show a drawer error explaining that the subscription record was not found.
- Media API failures continue to use the existing `detailError` and toast behavior.
- Route updates should ignore duplicate navigation failures.

## Testing

- Add or update frontend tests around route helper behavior if the current test setup can import the relevant logic.
- Run the frontend build/check command available in `package.json`.
- Manually verify:
  - search card click changes URL and opens drawer.
  - subscription card click changes URL and opens drawer.
  - browser Back closes the drawer on both pages.
  - refresh/direct-open restores a media detail.
  - refresh/direct-open restores a subscription detail after subscriptions load.
