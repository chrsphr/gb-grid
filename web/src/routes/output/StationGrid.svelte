<!--
  One fuel's stations as a heatmap: a row per station, north at the top, a
  column per half-hour. Loads when scrolled into view. Clicking a row opens
  that station's dispatch page for the same window.
-->
<script lang="ts" module>
	/** Left gutter shared with the fuel chart above, so columns line up. */
	export const GRID_LEFT = 72;
</script>

<script lang="ts">
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import { createQuery } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { resolve as resolvePath } from '$app/paths';
	import { api } from '#lib/api.js';
	import EChart from '#lib/charts/EChart.svelte';
	import { base, fmt, fmtTime } from '#lib/charts/theme.js';
	import * as Collapsible from '#lib/components/ui/collapsible/index.js';
	import { currentMode } from '#lib/mode.svelte.js';
	import { CHROME, diverging, sequential } from '#lib/palette.js';
	import type { Resolved, TimeRange } from '#lib/time.js';

	interface Props {
		fuel: string;
		title: string;
		note: string;
		pct: boolean;
		range: TimeRange;
		resolved: Resolved;
		color: { light: string; dark: string };
	}

	let { fuel, title, note, pct, range, resolved, color }: Props = $props();
	const mode = $derived(currentMode());

	let open = $state(true);
	let seen = $state(false);
	let el = $state<HTMLElement>();

	$effect(() => {
		if (!el || seen) return;
		const io = new IntersectionObserver(
			(entries) => {
				if (entries.some((e) => e.isIntersecting)) {
					seen = true;
					io.disconnect();
				}
			},
			{ rootMargin: '200px' }
		);
		io.observe(el);
		return () => io.disconnect();
	});

	const q = createQuery(() => ({
		queryKey: ['station-grid', fuel, pct, range.from, range.to],
		enabled: seen && open,
		queryFn: ({ signal }) => api.stationGrid(fuel, resolved.start, resolved.end, pct, signal)
	}));

	const SLOT = 30 * 60_000;
	const shaped = $derived.by(() => {
		const rows = q.data ?? [];
		const t0 = Math.floor(resolved.start.getTime() / SLOT) * SLOT;
		const n = Math.max(1, Math.ceil((resolved.end.getTime() - t0) / SLOT));
		const slots = Array.from({ length: n }, (_, i) => t0 + i * SLOT);
		const stations = rows.map((r) => r.station);
		const cells: [number, number, number][] = [];
		let maxAbs = 0;
		rows.forEach((r, y) => {
			r.ts.forEach((t, i) => {
				const x = Math.round((t - t0) / SLOT);
				if (x < 0 || x >= n) return;
				const v = r.value[i];
				maxAbs = Math.max(maxAbs, Math.abs(v));
				cells.push([x, y, v]);
			});
		});
		return { slots, stations, cells, maxAbs };
	});

	const ROW = 14;
	const height = $derived(Math.max(1, shaped.stations.length) * ROW + 40);

	const dayLabel = new Intl.DateTimeFormat('en-GB', { weekday: 'short', day: 'numeric', month: 'short' });

	const option = $derived.by(() => {
		const c = CHROME[mode];
		const { slots, stations, cells, maxAbs } = shaped;
		const unit = pct ? '%' : 'MW';
		const span = Math.max(1, Math.ceil(maxAbs));
		return {
			...base(mode),
			grid: { left: GRID_LEFT, right: 16, top: 4, bottom: 28 },
			tooltip: {
				...base(mode).tooltip,
				trigger: 'item',
				formatter: (p: { value: [number, number, number] }) => {
					const [x, y, v] = p.value;
					return `<div style="font-weight:600">${stations[y]}</div>
						<div style="color:${c.muted}">${fmtTime(slots[x])}</div>
						<div style="font-variant-numeric:tabular-nums">${fmt(v, unit)}</div>`;
				}
			},
			xAxis: {
				type: 'category',
				data: slots,
				axisLine: { lineStyle: { color: c.axis } },
				axisTick: { show: false },
				splitLine: { show: false },
				axisLabel: {
					color: c.muted,
					// Label each local midnight.
					interval: (i: number) => new Date(slots[i]).getHours() === 0 && new Date(slots[i]).getMinutes() === 0,
					formatter: (v: string) => dayLabel.format(new Date(Number(v)))
				}
			},
			yAxis: {
				type: 'category',
				data: stations,
				inverse: true,
				triggerEvent: true,
				axisLine: { show: false },
				axisTick: { show: false },
				axisLabel: { color: c.ink2, fontSize: 11, interval: 0, cursor: 'pointer' }
			},
			visualMap: {
				show: false,
				type: 'continuous',
				dimension: 2,
				min: pct ? 0 : -span,
				max: pct ? 100 : span,
				inRange: { color: pct ? sequential(color, mode) : diverging(mode) }
			},
			series: [
				{
					type: 'heatmap',
					data: cells,
					itemStyle: { borderColor: c.surface, borderWidth: 0.5 },
					emphasis: { itemStyle: { borderColor: c.ink, borderWidth: 1 } },
					progressive: 0
				}
			]
		};
	});

	function openStation(params: { value?: unknown; componentType?: string; name?: string }) {
		let station: string | undefined;
		if ((params as { componentType?: string }).componentType === 'yAxis') {
			station = (params as { value?: string }).value;
		} else if (Array.isArray(params.value)) {
			station = shaped.stations[params.value[1] as number];
		}
		if (!station) return;
		const qs = new URLSearchParams({ station, from: range.from, to: range.to });
		goto(`${resolvePath('/dispatch')}?${qs}`);
	}

	const legendGradient = $derived(
		pct
			? `linear-gradient(to right, ${sequential(color, mode).join(', ')})`
			: `linear-gradient(to right, ${diverging(mode).join(', ')})`
	);
</script>

<section bind:this={el} class="rounded-md border bg-card">
	<Collapsible.Root bind:open>
		<div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 px-4 py-3">
			<Collapsible.Trigger class="group flex items-center gap-2 text-left">
				<ChevronDown class="size-4 text-muted-foreground transition-transform group-data-[state=closed]:-rotate-90" />
				<span class="size-2.5 rounded-full" style="background:{color[mode]}"></span>
				<h2 class="font-display text-base font-semibold">{title}</h2>
				{#if q.data}<span class="text-sm text-muted-foreground">{q.data.length} stations</span>{/if}
			</Collapsible.Trigger>
			<div class="flex items-center gap-3 text-xs text-muted-foreground">
				<span>{note}</span>
				<span class="flex items-center gap-1.5" aria-hidden="true">
					{pct ? '0%' : `−${fmt(Math.ceil(shaped.maxAbs))}`}
					<span class="h-2 w-20 rounded-sm" style="background:{legendGradient}"></span>
					{pct ? '100%' : `+${fmt(Math.ceil(shaped.maxAbs))} MW`}
				</span>
			</div>
		</div>
		<Collapsible.Content>
			{#if q.error}
				<p class="px-4 pb-3 text-sm">Couldn't load {title.toLowerCase()} output: {q.error.message}</p>
			{:else if q.data && q.data.length === 0}
				<p class="px-4 pb-3 text-sm text-muted-foreground">
					No metered output for located {title.toLowerCase()} stations in this window.
				</p>
			{:else if !q.data}
				<div class="mx-4 mb-3 h-24 animate-pulse rounded bg-muted/50"></div>
			{:else}
				<div style="height:{height}px">
					<EChart
						{option}
						onclick={openStation}
						stale={q.isFetching && q.isPlaceholderData}
						class="h-full"
						label="{title} output by station, half-hourly"
					/>
				</div>
			{/if}
		</Collapsible.Content>
	</Collapsible.Root>
</section>
