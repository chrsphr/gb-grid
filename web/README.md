# gb-grid web UI

The SvelteKit single-page app for gb-grid. See the "Web UI" section of the
repository README for what it shows, how it gets data (PostgREST over the
database's `api` schema), and how to run it.

```bash
npm install
npm run dev      # http://localhost:5173, /api proxied to PostgREST on :3001
npm run check
npm test
npm run build    # static files in build/
```

UI primitives in `src/lib/components/ui/` come from
[shadcn-svelte](https://shadcn-svelte.com) (`npx shadcn-svelte add <name>`).
