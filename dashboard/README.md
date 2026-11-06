# starlist dashboard

A small SvelteKit app that starlist embeds and serves at `/`. It only uses the
public JSON API ([docs/api.md](../docs/api.md)), so it is also an example of
building your own UI on starlist.

## Build it into starlist

```sh
cd dashboard
npm ci
npm run build          # writes build/
cd ..
cargo build --release  # embeds dashboard/build
```

Without `dashboard/build`, `cargo build` still works; the binary then serves a
short page at `/` saying how to build the dashboard. `starlist serve
--no-dashboard` turns it off.

## Work on it

Run starlist, then the dev server, which proxies `/api` to it:

```sh
starlist serve --file app.log           # API on http://127.0.0.1:7341
cd dashboard && npm run dev             # http://localhost:5173
STARLIST_API=http://other-host:7341 npm run dev
```

`npm run check` type-checks the app.

## Notes

- Single-page app: starlist serves `200.html` for any path that isn't a file.
- No external fonts, scripts or requests; the page's Content-Security-Policy
  only allows its own origin.
- With a token (starlist listening beyond localhost), the dashboard asks for it
  once and keeps it in session storage for that tab.
- Refreshes every 10 seconds while visible, and at once when an alert arrives
  on `/api/v1/alerts/stream`.
