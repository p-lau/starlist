import { sveltekit } from '@sveltejs/kit/vite';
import adapter from '@sveltejs/adapter-static';
import { defineConfig } from 'vite';

// A single-page app: starlist embeds build/ and serves 200.html for any path
// that isn't a file. In development, /api is proxied to a running starlist
// (STARLIST_API, default http://127.0.0.1:7341).
const api = (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env.STARLIST_API ?? 'http://127.0.0.1:7341';

export default defineConfig({
	plugins: [
		sveltekit({
			adapter: adapter({ fallback: '200.html', precompress: true }),
			output: { bundleStrategy: 'single' }
		})
	],
	server: {
		proxy: { '/api': { target: api, changeOrigin: true } }
	}
});
