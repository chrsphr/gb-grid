import tailwindcss from '@tailwindcss/vite';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

// PostgREST, as published by docker-compose (`postgrest` service). Override
// with GB_GRID_API_PROXY to point the dev server at another instance.
const apiTarget = process.env.GB_GRID_API_PROXY ?? 'http://127.0.0.1:3001';

export default defineConfig({
	plugins: [
		tailwindcss(),
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			// A static single-page app: every page fetches its data from PostgREST
			// in the browser, so there is nothing to render on a server. nginx
			// serves build/ and falls back to 200.html for client-side routes.
			adapter: adapter({ fallback: '200.html', precompress: true })
		})
	],
	server: {
		proxy: {
			'/api': {
				target: apiTarget,
				rewrite: (path) => path.replace(/^\/api/, '')
			}
		}
	},
	test: {
		include: ['src/**/*.test.ts']
	}
});
