<script lang="ts">
	import './layout.css';
	import Moon from '@lucide/svelte/icons/moon';
	import Sun from '@lucide/svelte/icons/sun';
	import { QueryClient, QueryClientProvider } from '@tanstack/svelte-query';
	import { ModeWatcher, toggleMode } from 'mode-watcher';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	const queryClient = new QueryClient({
		defaultOptions: {
			queries: {
				// Keep the previous result on screen (dimmed) while a new window loads.
				placeholderData: (prev: unknown) => prev,
				staleTime: 60_000,
				refetchOnWindowFocus: false,
				retry: 1
			}
		}
	});

	const NAV = [
		{ href: '/dispatch', label: 'Dispatch' },
		{ href: '/output', label: 'Output' },
		{ href: '/annual', label: 'Annual' },
		{ href: '/constraints', label: 'Constraints' }
	] as const;

	// Carry the time window across pages so switching views keeps context.
	function navHref(href: (typeof NAV)[number]['href']) {
		const q = new URLSearchParams();
		const from = page.url.searchParams.get('from');
		const to = page.url.searchParams.get('to');
		if (from && to) {
			q.set('from', from);
			q.set('to', to);
		}
		const s = q.toString();
		return `${resolve(href)}${s ? `?${s}` : ''}`;
	}
</script>

<svelte:head>
	<title>GB grid</title>
	<meta name="description" content="Great Britain's electricity grid, unit by unit." />
</svelte:head>

<!-- Follows the system setting until the toggle picks one (remembered per browser). -->
<ModeWatcher defaultMode="system" />
<QueryClientProvider client={queryClient}>
	<Tooltip.Provider delayDuration={300}>
		<div class="flex min-h-dvh flex-col">
			<header class="border-b bg-card">
				<div class="flex h-12 items-center gap-6 px-4">
					<a href={resolve('/dispatch')} class="flex items-center gap-2 font-display text-lg font-semibold tracking-tight">
						<svg viewBox="0 0 24 24" class="size-5" aria-hidden="true">
							<!-- A transmission pylon, drawn in the ink colour. -->
							<path
								d="M12 2 7 22M12 2l5 20M8.6 15h6.8M9.6 10.5h4.8M5 7h14M7 7l5 3.5L17 7M8.6 15 12 18.5l3.4-3.5"
								fill="none"
								stroke="currentColor"
								stroke-width="1.6"
								stroke-linejoin="round"
								stroke-linecap="round"
							/>
						</svg>
						GB grid
					</a>
					<nav aria-label="Views" class="flex h-full items-stretch gap-1 overflow-x-auto">
						{#each NAV as item (item.href)}
							{@const active = page.url.pathname.startsWith(item.href)}
							<a
								href={navHref(item.href)}
								aria-current={active ? 'page' : undefined}
								class="flex items-center border-b-2 px-2.5 text-sm transition-colors {active
									? 'border-foreground font-medium text-foreground'
									: 'border-transparent text-muted-foreground hover:text-foreground'}"
							>
								{item.label}
							</a>
						{/each}
					</nav>
					<div class="ml-auto">
						<Button variant="ghost" size="icon-sm" onclick={toggleMode} aria-label="Switch light or dark mode">
							<Sun class="hidden dark:block" />
							<Moon class="dark:hidden" />
						</Button>
					</div>
				</div>
			</header>
			<main class="flex-1">
				{@render children()}
			</main>
		</div>
	</Tooltip.Provider>
</QueryClientProvider>
