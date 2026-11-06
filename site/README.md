# starlist site

The landing page for starlist, built with SvelteKit and prerendered to static files.

## Develop

```sh
npm install
npm run dev      # http://localhost:5173
npm run check    # type and Svelte checks
npm run build    # writes the static site to build/
npm run preview  # serves build/ locally
```

## Deploy (Cloudflare Pages)

In the Cloudflare dashboard: Workers & Pages, Create, Pages, Connect to Git, pick the starlist repo, then set:

| Setting | Value |
|---|---|
| Production branch | `main` |
| Framework preset | SvelteKit |
| Build command | `npm run build` |
| Build output directory | `build` |
| Root directory | `site` |
| Environment variable | `NODE_VERSION` = `22` |

Under Settings, Builds, Build watch paths, include only `site/*` so Rust-only commits don't rebuild the site. Every pull request that touches `site/` gets its own preview URL.

`static/_headers` sets long caching for hashed assets and a few security headers.
