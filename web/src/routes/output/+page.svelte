<!--
  Output: national generation by fuel, then one heatmap per fuel type with a
  row per station (north at the top) showing half-hourly metered output.
-->
<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '#lib/api.js';
	import EChart from '#lib/charts/EChart.svelte';
	import { axisTooltip, base, legend, timeAxis, valueAxis } from '#lib/charts/theme.js';
	import TimeRangePicker from '#lib/components/app/TimeRangePicker.svelte';
	import { pickStep } from '#lib/interval.js';
	import { currentMode } from '#lib/mode.svelte.js';
	import { CHROME, FUEL_GROUPS, fuelGroup } from '#lib/palette.js';
	import { absolute, resolve as resolveRange, type TimeRange } from '#lib/time.js';
	import { setRange, timeRange } from '#lib/url-state.svelte.js';
	import StationGrid, { GRID_LEFT } from './StationGrid.svelte';

	// Metered (B1610) output lags about five working days, so default to the
	// most recent week that is likely to be complete.
	const DEFAULT: TimeRange = { from: 'now-12d', to: 'now-5d' };
	const range = $derived(timeRange(DEFAULT));
	const mode = $derived(currentMode());

	let plotWidth = $state(1200);
	const resolved = $derived(resolveRange(range));
	const step = $derived.by(() => {
		const span = resolved ? resolved.end.getTime() - resolved.start.getTime() : 7 * 86_400_000;
		return pickStep(span, Math.max(200, plotWidth - 100));
	});

	const mixQ = createQuery(() => ({
		queryKey: ['fuel-mix', range.from, range.to, step.pg],
		queryFn: async ({ signal }) => {
			const r = resolveRange(range);
			if (!r) throw new Error('The time range is invalid.');
			return { r, series: await api.fuelMix(r.start, r.end, step.pg, signal) };
		}
	}));

	// Sum BMRS fuels into the eight chart groups, keeping stack order.
	const groups = $derived.by(() => {
		const d = mixQ.data;
		if (!d) return [];
		const byGroup = new Map<string, Map<number, number>>();
		for (const s of d.series) {
			const g = fuelGroup(s.fuel_type).id;
			const m = byGroup.get(g) ?? new Map<number, number>();
			s.ts.forEach((t, i) => m.set(t, (m.get(t) ?? 0) + (s.mw[i] ?? 0)));
			byGroup.set(g, m);
		}
		return FUEL_GROUPS.filter((g) => byGroup.has(g.id)).map((g) => ({
			group: g,
			data: [...byGroup.get(g.id)!.entries()].sort((a, b) => a[0] - b[0])
		}));
	});

	const mixOption = $derived.by(() => {
		const c = CHROME[mode];
		const r = mixQ.data?.r;
		return {
			...base(mode),
			grid: { left: GRID_LEFT, right: 16, top: 12, bottom: 44 },
			legend: legend(mode),
			tooltip: { ...base(mode).tooltip, ...axisTooltip(mode, 'MW', { hideZero: true }) },
			xAxis: timeAxis(mode, r?.start.getTime(), r?.end.getTime()),
			yAxis: valueAxis(mode, 'MW'),
			// A group that flips sign (pumped storage, interconnectors) is split into
			// its positive and negative parts, so the stack doesn't draw a spike each
			// time it crosses zero. Both parts share the legend entry.
			series: groups.flatMap(({ group, data }) => {
				const parts: [string, [number, number | null][]][] = data.some(([, v]) => v < 0)
					? [
							['pos', data.map(([t, v]) => [t, v > 0 ? v : 0])],
							// Null, not 0, where it isn't negative: ECharts stacks a 0 on
							// the positive side, which would draw it up to the top.
							['neg', data.map(([t, v]) => [t, v < 0 ? v : null])]
						]
					: [['all', data]];
				return parts.map(([, d]) => ({
					name: group.label,
					type: 'line',
					stack: 'mix',
					stackStrategy: 'samesign',
					data: d,
					showSymbol: false,
					color: group.color[mode],
					lineStyle: { width: 1, color: c.surface },
					areaStyle: { color: group.color[mode], opacity: 0.9 },
					emphasis: { disabled: true }
				}));
			})
		};
	});

	interface Section {
		fuel: string;
		title: string;
		note: string;
		pct: boolean;
	}

	const LOAD = 'Output as a share of capacity (peak MEL in the window), half-hourly';
	const NET = 'Net output, half-hourly. Below zero: importing or charging';
	const SECTIONS: Section[] = [
		{ fuel: 'WIND', title: 'Wind', note: LOAD, pct: true },
		{ fuel: 'CCGT', title: 'Gas, combined cycle', note: LOAD, pct: true },
		{ fuel: 'OCGT', title: 'Gas, open cycle', note: LOAD, pct: true },
		{ fuel: 'NUCLEAR', title: 'Nuclear', note: LOAD, pct: true },
		{ fuel: 'BIOMASS', title: 'Biomass', note: LOAD, pct: true },
		{ fuel: 'NPSHYD', title: 'Hydro', note: LOAD, pct: true },
		{ fuel: 'PS', title: 'Pumped storage', note: NET, pct: false },
		{ fuel: 'INTEW', title: 'East–West interconnector', note: NET, pct: false },
		{ fuel: 'OTHER', title: 'Other (batteries, solar and more)', note: NET, pct: false }
	];

	const onzoom = (start: number, end: number) => setRange(absolute(start, end));
</script>

<svelte:head><title>Output · GB grid</title></svelte:head>

<div class="mx-auto flex max-w-[1800px] flex-col gap-4 p-4">
	<div class="flex flex-wrap items-end justify-between gap-3">
		<div>
			<h1 class="font-display text-2xl font-semibold tracking-tight">Output by fuel and station</h1>
			<p class="text-sm text-muted-foreground">
				Metered output arrives about five working days late, so the last few days are blank.
			</p>
		</div>
		<TimeRangePicker {range} onchange={setRange} />
	</div>

	<section class="rounded-md border bg-card" bind:clientWidth={plotWidth}>
		<div class="flex items-baseline justify-between gap-3 px-4 pt-3">
			<h2 class="font-display text-base font-semibold">National generation by fuel</h2>
			<span class="text-xs text-muted-foreground">{step.label} averages. Drag across the chart to zoom.</span>
		</div>
		{#if mixQ.error}
			<p class="m-4 text-sm">Couldn't load generation: {mixQ.error.message}</p>
		{/if}
		<EChart
			option={mixOption}
			{onzoom}
			stale={mixQ.isFetching && mixQ.isPlaceholderData}
			class="h-80"
			label="National generation stacked by fuel group"
		/>
	</section>

	{#if resolved}
		{#each SECTIONS as s (s.fuel)}
			<StationGrid
				fuel={s.fuel}
				title={s.title}
				note={s.note}
				pct={s.pct}
				{range}
				{resolved}
				color={fuelGroup(s.fuel).color}
			/>
		{/each}
	{/if}
</div>
