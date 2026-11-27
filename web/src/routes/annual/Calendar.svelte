<!--
  One metric as a calendar heatmap: weeks run left to right, Monday at the top.
-->
<script lang="ts">
	import EChart from '#lib/charts/EChart.svelte';
	import { base, fmt } from '#lib/charts/theme.js';
	import { currentMode } from '#lib/mode.svelte.js';
	import { CHROME, sequential } from '#lib/palette.js';
	import type { Resolved } from '#lib/time.js';

	interface Props {
		title: string;
		color: { light: string; dark: string };
		pct: boolean;
		range: Resolved | undefined;
		days: [string, number | null][];
		loading: boolean;
	}

	let { title, color, pct, range, days, loading }: Props = $props();
	const mode = $derived(currentMode());

	const values = $derived(days.filter((d) => d[1] !== null) as [string, number][]);
	const max = $derived(values.reduce((m, [, v]) => Math.max(m, v), 0));
	const total = $derived(values.reduce((s, [, v]) => s + v, 0));

	const iso = (d: Date) =>
		`${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
	const dayFmt = new Intl.DateTimeFormat('en-GB', { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' });

	const option = $derived.by(() => {
		const c = CHROME[mode];
		const unit = pct ? '%' : 'MWh';
		const start = range ? iso(range.start) : iso(new Date(Date.now() - 365 * 86_400_000));
		const end = range ? iso(new Date(range.end.getTime() - 1)) : iso(new Date());
		return {
			...base(mode),
			tooltip: {
				...base(mode).tooltip,
				trigger: 'item',
				formatter: (p: { value: [string, number] }) =>
					`<div style="color:${c.muted}">${dayFmt.format(new Date(p.value[0]))}</div>
					 <div style="font-variant-numeric:tabular-nums">${fmt(p.value[1], unit)}</div>`
			},
			visualMap: {
				show: false,
				min: 0,
				max: pct ? 100 : Math.max(1, max),
				inRange: { color: sequential(color, mode) }
			},
			calendar: {
				range: [start, end],
				top: 22,
				left: 36,
				right: 8,
				bottom: 4,
				cellSize: ['auto', 13],
				orient: 'horizontal',
				splitLine: { show: false },
				itemStyle: { color: c.neutral, borderColor: c.surface, borderWidth: 2 },
				dayLabel: { firstDay: 1, nameMap: ['S', 'M', 'T', 'W', 'T', 'F', 'S'], color: c.muted, fontSize: 10 },
				monthLabel: { color: c.muted, fontSize: 11, nameMap: 'EN' },
				yearLabel: { show: false }
			},
			series: [
				{
					type: 'heatmap',
					coordinateSystem: 'calendar',
					data: values,
					emphasis: { itemStyle: { borderColor: c.ink, borderWidth: 1 } }
				}
			]
		};
	});

	const gradient = $derived(`linear-gradient(to right, ${sequential(color, mode).join(', ')})`);
	const summary = $derived(
		values.length === 0
			? 'No data'
			: pct
				? `Average ${fmt(total / values.length, '%')} over ${values.length} days`
				: `${fmt(total, 'MWh')} over ${values.length} days`
	);
</script>

<section class="rounded-md border bg-card px-4 pt-3 pb-2">
	<div class="flex flex-wrap items-baseline justify-between gap-x-4">
		<h2 class="flex items-center gap-2 font-display text-base font-semibold">
			<span class="size-2.5 rounded-full" style="background:{color[mode]}"></span>{title}
		</h2>
		<div class="flex items-center gap-3 text-xs text-muted-foreground">
			<span class="figures">{summary}</span>
			<span class="flex items-center gap-1.5" aria-hidden="true">
				0<span class="h-2 w-16 rounded-sm" style="background:{gradient}"></span>{pct ? '100%' : fmt(max, 'MWh')}
			</span>
		</div>
	</div>
	<EChart {option} stale={loading} class="h-[130px]" label="{title} per day" />
</section>
