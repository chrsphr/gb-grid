<!--
  BMU dispatch: one station's planned (PN), instructed (dispatched), capacity
  (MEL) and metered (B1610) output, per-BMU dispatch, and the system
  operator's turn-up and curtailment.
-->
<script lang="ts">
	import { createQuery, useQueryClient, useIsFetching } from '@tanstack/svelte-query';
	import { api, type DispatchRow } from '#lib/api.js';
	import EChart from '#lib/charts/EChart.svelte';
	import { axisTooltip, base, legend, timeAxis, valueAxis } from '#lib/charts/theme.js';
	import Picker from '#lib/components/app/Picker.svelte';
	import StationMap from '#lib/components/app/StationMap.svelte';
	import TimeRangePicker from '#lib/components/app/TimeRangePicker.svelte';
	import Ledger from './Ledger.svelte';
	import { pickStep } from '#lib/interval.js';
	import { currentMode } from '#lib/mode.svelte.js';
	import { CATEGORICAL, CHROME, METRIC, fuelGroup, fuelLabel } from '#lib/palette.js';
	import { refetchEvery } from '#lib/query.svelte.js';
	import { resolve as resolveRange, absolute, type TimeRange } from '#lib/time.js';
	import { param, setParams, setRange, timeRange } from '#lib/url-state.svelte.js';

	const DEFAULT: TimeRange = { from: 'now-24h', to: 'now' };
	const range = $derived(timeRange(DEFAULT));
	const refresh = $derived(param('refresh') ?? '30s');
	const mode = $derived(currentMode());
	const client = useQueryClient();
	const fetching = useIsFetching();

	let plotWidth = $state(900);
	// Plot area is the chart width minus the y-axis gutter.
	const step = $derived.by(() => {
		const r = resolveRange(range);
		const span = r ? r.end.getTime() - r.start.getTime() : 86_400_000;
		return pickStep(span, Math.max(200, plotWidth - 80));
	});

	const stationsQ = createQuery(() => ({
		queryKey: ['stations'],
		queryFn: ({ signal }) => api.stations(signal),
		staleTime: 10 * 60_000
	}));

	const dispatchable = $derived((stationsQ.data ?? []).filter((s) => s.has_dispatch));
	const station = $derived(
		param('station') ??
			dispatchable.find((s) => s.latitude !== null && s.fuel_type === 'WIND')?.station ??
			dispatchable[0]?.station ??
			''
	);
	const info = $derived(stationsQ.data?.find((s) => s.station === station));

	const every = $derived(refetchEvery(range, refresh));

	const seriesQ = createQuery(() => ({
		queryKey: ['dispatch', station, range.from, range.to, step.pg],
		enabled: !!station,
		refetchInterval: every,
		queryFn: async ({ signal }) => {
			const r = resolveRange(range);
			if (!r) throw new Error('The time range is invalid.');
			const [rows, b1610] = await Promise.all([
				api.dispatchSeries(station, r.start, r.end, step.pg, signal),
				api.b1610Series(station, r.start, r.end, step.pg, signal)
			]);
			return { r, rows, b1610 };
		}
	}));

	const totalsQ = createQuery(() => ({
		queryKey: ['dispatch-totals', station, range.from, range.to],
		enabled: !!station,
		refetchInterval: every,
		queryFn: ({ signal }) => {
			const r = resolveRange(range)!;
			return api.dispatchTotals(station, r.start, r.end, signal);
		}
	}));

	// --- Shape the data ------------------------------------------------------------

	type Pt = [number, number | null];

	function sumByTs(rows: DispatchRow[], key: keyof DispatchRow, sign = 1): Pt[] {
		const m = new Map<number, number | null>();
		for (const row of rows) {
			const t = Date.parse(row.ts);
			const v = row[key] as number | null;
			const prev = m.get(t);
			m.set(t, v === null ? (prev ?? null) : (prev ?? 0) + sign * v);
		}
		return [...m.entries()].sort((a, b) => a[0] - b[0]);
	}

	const shaped = $derived.by(() => {
		const d = seriesQ.data;
		if (!d) return null;
		const { rows, b1610, r } = d;
		const bmus = [...new Set(rows.map((x) => x.bmu))].sort((a, b) =>
			a.localeCompare(b, undefined, { numeric: true })
		);
		// Eight categorical slots; any further BMUs share one "Other" band.
		const shown = bmus.slice(0, bmus.length > 8 ? 7 : 8);
		const perBmu = new Map<string, Pt[]>();
		for (const b of shown) perBmu.set(b, []);
		const other = new Map<number, number>();
		for (const row of rows) {
			const t = Date.parse(row.ts);
			if (perBmu.has(row.bmu)) perBmu.get(row.bmu)!.push([t, row.boa_level_mw]);
			else other.set(t, (other.get(t) ?? 0) + (row.boa_level_mw ?? 0));
		}
		const mel = sumByTs(rows, 'mel_mw');
		return {
			r,
			tier: rows[0]?.tier ?? b1610[0]?.tier ?? null,
			pn: sumByTs(rows, 'pn_mw'),
			dispatched: sumByTs(rows, 'boa_level_mw'),
			mel,
			peakMel: mel.reduce((m, [, v]) => Math.max(m, v ?? 0), 0),
			b1610: b1610.map((x) => [Date.parse(x.ts), x.b1610_mw] as Pt),
			bmuSeries: [
				...shown.map((b) => ({ name: b, data: perBmu.get(b)! })),
				...(other.size
					? [{ name: `Other ${bmus.length - shown.length} BMUs`, data: [...other.entries()].sort((a, b) => a[0] - b[0]) as Pt[] }]
					: [])
			],
			turnup: sumByTs(rows, 'so_turnup_mw'),
			boaCurt: sumByTs(rows, 'boa_curtailment_mw', -1),
			soCurt: sumByTs(rows, 'so_curtailment_mw', -1),
			empty: rows.length === 0 && b1610.length === 0
		};
	});

	// --- Chart options -------------------------------------------------------------

	const GRID = { left: 64, right: 16, top: 12, bottom: 44 };

	function line(name: string, data: Pt[], color: string, extra: Record<string, unknown> = {}) {
		return {
			name,
			type: 'line',
			data,
			showSymbol: false,
			connectNulls: false,
			color,
			lineStyle: { width: 2, color },
			emphasis: { disabled: true },
			...extra
		};
	}

	const totalOption = $derived.by(() => {
		const s = shaped;
		const c = CHROME[mode];
		return {
			...base(mode),
			grid: GRID,
			legend: legend(mode),
			tooltip: { ...base(mode).tooltip, ...axisTooltip(mode, 'MW') },
			xAxis: timeAxis(mode, s?.r.start.getTime(), s?.r.end.getTime()),
			yAxis: valueAxis(mode, 'MW'),
			series: s
				? [
						line('PN (planned)', s.pn, METRIC.pn[mode]),
						line('Dispatched', s.dispatched, METRIC.dispatched[mode]),
						line('MEL (max export)', s.mel, c.muted, { lineStyle: { width: 1.5, color: c.muted, type: [6, 4] } }),
						line('B1610 (metered)', s.b1610, c.ink, { step: 'start' })
					]
				: []
		};
	});

	const bmuOption = $derived.by(() => {
		const s = shaped;
		const c = CHROME[mode];
		return {
			...base(mode),
			grid: GRID,
			legend: legend(mode),
			tooltip: { ...base(mode).tooltip, ...axisTooltip(mode, 'MW', { hideZero: true }) },
			xAxis: timeAxis(mode, s?.r.start.getTime(), s?.r.end.getTime()),
			yAxis: valueAxis(mode, 'MW'),
			series: (s?.bmuSeries ?? []).map((b, i) => {
				const color = (i < 8 ? CATEGORICAL[i][mode] : c.muted) as string;
				return {
					name: b.name,
					type: 'line',
					stack: 'bmu',
					data: b.data,
					showSymbol: false,
					color,
					// A hairline of surface colour on each band's top edge separates
					// neighbouring fills.
					lineStyle: { width: 1, color: c.surface },
					areaStyle: { color, opacity: 0.9 },
					emphasis: { disabled: true }
				};
			})
		};
	});

	const actionsOption = $derived.by(() => {
		const s = shaped;
		const area = (color: string) => ({ areaStyle: { color, opacity: 0.25 } });
		return {
			...base(mode),
			grid: GRID,
			legend: legend(mode),
			tooltip: { ...base(mode).tooltip, ...axisTooltip(mode, 'MW') },
			xAxis: timeAxis(mode, s?.r.start.getTime(), s?.r.end.getTime()),
			yAxis: valueAxis(mode, 'MW'),
			series: s
				? [
						line('SO turn-up', s.turnup, METRIC.turnup[mode], area(METRIC.turnup[mode])),
						line('BOA curtailment', s.boaCurt, METRIC.boaCurtailment[mode], area(METRIC.boaCurtailment[mode])),
						line('SO curtailment', s.soCurt, METRIC.soCurtailment[mode], area(METRIC.soCurtailment[mode]))
					]
				: []
		};
	});

	const stationItems = $derived(
		dispatchable.map((s) => ({
			value: s.station,
			label: s.station,
			detail: [s.name, fuelLabel(s.fuel_type)].filter(Boolean).join(', '),
			swatch: fuelGroup(s.fuel_type).color[mode]
		}))
	);

	const onzoom = (start: number, end: number) => setRange(absolute(start, end));
	const stale = $derived(seriesQ.isFetching && seriesQ.isPlaceholderData);
	const tierLabel: Record<string, string> = {
		raw: '5-minute data',
		hourly: 'hourly averages',
		daily: 'daily averages'
	};
</script>

<svelte:head><title>{station ? `${station} dispatch` : 'Dispatch'} · GB grid</title></svelte:head>

<div class="mx-auto flex max-w-[1800px] flex-col gap-4 p-4">
	<div class="flex flex-wrap items-center justify-between gap-3">
		<Picker
			items={stationItems}
			value={station ? [station] : []}
			onchange={(v) => setParams({ station: v[0] })}
			placeholder="Choose a station"
			searchLabel="Search stations by code or name"
			class="w-56"
		/>
		<TimeRangePicker
			{range}
			onchange={setRange}
			{refresh}
			onrefresh={() => client.invalidateQueries()}
			onrefreshchange={(v) => setParams({ refresh: v }, { replace: true })}
			refreshing={fetching.current > 0}
		/>
	</div>

	<div class="grid gap-4 lg:grid-cols-[minmax(300px,25rem)_1fr]">
		<aside class="flex min-w-0 flex-col gap-4">
			<header class="rounded-md border bg-card px-4 py-3">
				<div class="flex min-w-0 items-baseline gap-3">
					<h1 class="min-w-0 truncate font-display text-4xl leading-none font-semibold tracking-tight">{station || '–'}</h1>
					{#if info}
						<span class="flex shrink-0 items-center gap-1.5 text-sm text-muted-foreground">
							<span class="size-2.5 rounded-full" style="background:{fuelGroup(info.fuel_type).color[mode]}"></span>
							{fuelLabel(info.fuel_type)}
						</span>
					{/if}
				</div>
				<!-- Fixed height, so a long or missing name never moves the map below. -->
				<p class="mt-1 h-5 truncate text-sm text-muted-foreground" title={info?.name ?? ''}>{info?.name ?? ''}</p>
				{#if info}
					<dl class="mt-3 grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
						<dt class="text-muted-foreground">Units (BMUs)</dt>
						<dd class="figures text-right">{info.bmus}</dd>
						<dt class="text-muted-foreground">Peak MEL in window</dt>
						<dd class="figures text-right">
							{shaped?.peakMel ? `${Math.round(shaped.peakMel).toLocaleString('en-GB')} MW` : '–'}
						</dd>
					</dl>
				{/if}
			</header>

			<StationMap
				stations={stationsQ.data ?? []}
				selected={station ? [station] : []}
				onselect={(s) => setParams({ station: s })}
				class="h-80 lg:h-96"
			/>

			<Ledger totals={totalsQ.data} loading={totalsQ.isPending} {mode} />
		</aside>

		<section class="flex min-w-0 flex-col rounded-md border bg-card" bind:clientWidth={plotWidth}>
			{#if seriesQ.error}
				<p class="m-4 rounded-md border border-destructive/40 p-3 text-sm">
					Couldn't load dispatch data: {seriesQ.error.message}
				</p>
			{:else if shaped?.empty}
				<p class="m-4 text-sm text-muted-foreground">
					No dispatch data for {station} in this window. Dispatch is rebuilt hourly from PN and BOA
					data; pick an earlier window or another station.
				</p>
			{/if}

			{#snippet heading(title: string, note?: string)}
				<div class="flex items-baseline justify-between gap-3 px-4 pt-3">
					<h2 class="font-display text-base font-semibold">{title}</h2>
					{#if note}<span class="text-xs text-muted-foreground">{note}</span>{/if}
				</div>
			{/snippet}

			{@render heading(
				'Station output',
				shaped?.tier ? `${tierLabel[shaped.tier] ?? shaped.tier}, ${step.label} buckets. Drag across a chart to zoom.` : undefined
			)}
			<EChart option={totalOption} group="dispatch" {onzoom} {stale} class="h-72" label="Station total PN, dispatched, MEL and metered output" />

			<div class="mx-4 border-t"></div>
			{@render heading('Dispatched, by unit')}
			<EChart option={bmuOption} group="dispatch" {onzoom} {stale} class="h-64" label="Dispatched output stacked by BMU" />

			<div class="mx-4 border-t"></div>
			{@render heading('System operator actions', 'Turn-up above zero, curtailment below')}
			<EChart option={actionsOption} group="dispatch" {onzoom} {stale} class="h-56" label="SO turn-up and curtailment" />
		</section>
	</div>
</div>
