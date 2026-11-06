import { sveltekit } from '@sveltejs/kit/vite';
import adapter from '@sveltejs/adapter-static';
import { defineConfig } from 'vite';

// Fully prerendered static site. Cloudflare Pages serves the build/ folder.
export default defineConfig({
	plugins: [sveltekit({ adapter: adapter({ strict: true }) })]
});
