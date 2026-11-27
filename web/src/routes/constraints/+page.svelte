<!--
  Day-ahead constraint flows: for each boundary, the forecast flow's 24-hour
  average and daily range against its limit, as small multiples.
-->
<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api, type ConstraintSeries } from '#lib/api.js';
	import EChart from '#lib/charts/EChart.svelte';
	import { axisTooltip, base, legend, timeAxis, valueAxis } from '#lib/charts/theme.js';
	import Picker from '#lib/components/app/Picker.svelte';
	import TimeRangePicker from '#lib/components/app/TimeRangePicker.svelte';
	import { currentMode } from '#lib/mode.svelte.js';
	import { CHROME, METRIC } from '#lib/palette.js';
	import { absolute, resolve as resolveRange, type TimeRange } from '#lib/time.js';
	import { listParam, setParams, setRange, timeRange } from '#lib/url-state.svelte.js';

	const DEFAULT: TimeRange = { from: 'now-30d', to: 'now' };
	const range = $derived(timeRange(DEFAULT));
	const mode = $derived(currentMode());
	const picked = $derived(listParam('group'));

	const groupsQ = createQuery(() => ({
		queryKey: ['constraint-groups', range.from, range.to],
		queryFn: ({ signal }) => {
			const r = resolveRange(range);
			if (!r) throw new Error('The time range is invalid.');
			return api.constraintGroups(r.start, r.end, signal);
		}
	}));

	const available = $derived(groupsQ.data ?? []);
	const shown = $derived(
		picked === null || picked.length === 0
			? available
			: available.filter((g) => picked.includes(g.constraint_group))
	);

	const seriesQ = createQuery(() => ({
		queryKey: ['constraint-series', range.from, range.to, shown.map((g) => g.constraint_group).join(',')],
		enabled: shown.length > 0,
		queryFn: async ({ signal }) => {
			const r = resolveRange(range)!;
			const rows = await api.constraintSeries(
				shown.map((g) => g.constraint_group),
				r.start,
				r.end,
				signal
			);
			return { r, byGroup: new Map(rows.map((x) => [x.constraint_group, x])) };
		}
	}));

	const FLOW = { light: '#2a78d6', dark: '#3987e5' };

	function option(s: ConstraintSeries | undefined, r: { start: Date; end: Date } | undefined) {
		const c = CHROME[mode];
		const pts = (vals: (number | null)[]) => (s ? s.ts.map((t, i) => [t, vals[i]]) : []);
		const range = s ? s.ts.map((t, i) => [t, s.daily_max_mw[i] !== null && s.daily_min_mw[i] !== null ? s.daily_max_mw[i]! - s.daily_min_mw[i]! : null]) : [];
		const flow = FLOW[mode];
		const tip = axisTooltip(mode, 'MW');
		return {
			...base(mode),
			grid: { left: 64, right: 12, top: 10, bottom: 40 },
			legend: { ...legend(mode), data: ['24h average', 'Daily range', 'Limit'] },
			tooltip: {
				...base(mode).tooltip,
				...tip,
				// The band is drawn as min + (max − min); show the real max instead.
				formatter: (raw: { seriesName: string; value: [number, number | null] }[]) =>
					tip.formatter(
						raw
							.filter((p) => p.seriesName !== 'Daily range')
							.map((p) => p) as never
					)
			},
			xAxis: timeAxis(mode, r?.start.getTime(), r?.end.getTime()),
			yAxis: valueAxis(mode, 'MW'),
			series: [
				{
					name: 'Daily min',
					type: 'line',
					data: pts(s?.daily_min_mw ?? []),
					stack: 'band',
					// Stack on the minimum even when it's negative (the default
					// 'samesign' would restart the band from zero).
					stackStrategy: 'all',
					showSymbol: false,
					lineStyle: { width: 0 },
					color: flow,
					emphasis: { disabled: true }
				},
				{
					name: 'Daily range',
					type: 'line',
					data: range,
					stack: 'band',
					stackStrategy: 'all',
					showSymbol: false,
					lineStyle: { width: 0 },
					areaStyle: { color: flow, opacity: 0.18 },
					color: flow,
					emphasis: { disabled: true }
				},
				{
					name: 'Daily max',
					type: 'line',
					data: pts(s?.daily_max_mw ?? []),
					showSymbol: false,
					lineStyle: { width: 0 },
					color: flow,
					emphasis: { disabled: true }
				},
				{
					name: 'Flow',
					type: 'line',
					data: pts(s?.flow_mw ?? []),
					showSymbol: false,
					lineStyle: { width: 0 },
					color: c.muted,
					emphasis: { disabled: true }
				},
				{
					name: '24h average',
					type: 'line',
					data: pts(s?.avg_24h_mw ?? []),
					showSymbol: false,
					lineStyle: { width: 2, color: flow },
					color: flow,
					emphasis: { disabled: true }
				},
				{
					name: 'Limit',
					type: 'line',
					data: pts(s?.limit_mw ?? []),
					showSymbol: false,
					step: 'end',
					lineStyle: { width: 2, color: METRIC.boaCurtailment[mode] },
					color: METRIC.boaCurtailment[mode],
					emphasis: { disabled: true }
				}
			]
		};
	}

	const onzoom = (start: number, end: number) => setRange(absolute(start, end));
</script>

<svelte:head><title>Constraints · GB grid</title></svelte:head>

<div class="mx-auto flex max-w-[1800px] flex-col gap-4 p-4">
	<div class="flex flex-wrap items-end justify-between gap-3">
		<div>
			<h1 class="font-display text-2xl font-semibold tracking-tight">Day-ahead constraint flows</h1>
			<p class="text-sm text-muted-foreground">
				NESO's forecast flow across each transmission boundary against its limit. The shaded band is
				each day's range of flow.
			</p>
		</div>
		<div class="flex flex-wrap items-center gap-2">
			<Picker
				items={available.map((g) => ({ value: g.constraint_group, label: g.label }))}
				value={picked ?? []}
				onchange={(v) => setParams({ group: v.length ? v : null })}
				multiple
				placeholder="Boundaries"
				emptyLabel="All boundaries"
				searchLabel="Search boundaries"
				class="w-48"
			/>
			<TimeRangePicker {range} onchange={setRange} />
		</div>
	</div>

	{#if groupsQ.error || seriesQ.error}
		<p class="rounded-md border p-3 text-sm">
			Couldn't load constraint flows: {(groupsQ.error ?? seriesQ.error)?.message}
		</p>
	{:else if groupsQ.data && available.length === 0}
		<p class="rounded-md border bg-card p-4 text-sm text-muted-foreground">
			No constraint data in this window. NESO publishes it on weekday mornings; pick an earlier
			window if the latest refresh hasn't run.
		</p>
	{/if}

	<div class="grid gap-4 xl:grid-cols-2">
		{#each shown as g (g.constraint_group)}
			<section class="rounded-md border bg-card">
				<div class="flex items-baseline justify-between gap-3 px-4 pt-3">
					<h2 class="font-display text-base font-semibold">{g.label}</h2>
					{#if g.label !== g.constraint_group}
						<span class="text-xs text-muted-foreground">{g.constraint_group}</span>
					{/if}
				</div>
				<EChart
					option={option(seriesQ.data?.byGroup.get(g.constraint_group), seriesQ.data?.r)}
					group="constraints"
					{onzoom}
					stale={seriesQ.isFetching}
					class="h-60"
					label="{g.label} flow and limit"
				/>
			</section>
		{/each}
	</div>
</div>
