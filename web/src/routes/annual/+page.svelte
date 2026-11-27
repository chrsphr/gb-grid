<!--
  Annual summary: a calendar per metric (a cell per day) for a selection of
  fuel types and stations, in MWh/day or as a share of capacity.
-->
<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api, type CalendarDay } from '#lib/api.js';
	import Picker from '#lib/components/app/Picker.svelte';
	import StationMap from '#lib/components/app/StationMap.svelte';
	import TimeRangePicker from '#lib/components/app/TimeRangePicker.svelte';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import { currentMode } from '#lib/mode.svelte.js';
	import { CHROME, METRIC, fuelGroup, fuelLabel } from '#lib/palette.js';
	import { resolve as resolveRange, type TimeRange } from '#lib/time.js';
	import { listParam, param, setParams, setRange, timeRange } from '#lib/url-state.svelte.js';
	import Calendar from './Calendar.svelte';

	const DEFAULT: TimeRange = { from: 'now-1y', to: 'now' };
	const range = $derived(timeRange(DEFAULT));
	const mode = $derived(currentMode());

	// `fuel` absent means the default (wind); `fuel=` (empty) means all fuels.
	const fuels = $derived.by(() => {
		const l = listParam('fuel');
		if (l === null) return ['WIND'];
		return l;
	});
	const stations = $derived(listParam('station') ?? []);
	const pct = $derived(param('unit') === 'pct');

	const stationsQ = createQuery(() => ({
		queryKey: ['stations'],
		queryFn: ({ signal }) => api.stations(signal),
		staleTime: 10 * 60_000
	}));
	const fuelsQ = createQuery(() => ({
		queryKey: ['fuel-types'],
		queryFn: ({ signal }) => api.fuelTypes(signal),
		staleTime: 10 * 60_000
	}));

	const calQ = createQuery(() => ({
		queryKey: ['calendar', range.from, range.to, fuels.join(','), stations.join(','), pct],
		queryFn: async ({ signal }) => {
			const r = resolveRange(range);
			if (!r) throw new Error('The time range is invalid.');
			const days = await api.dailyCalendar(
				{
					start: r.start,
					end: r.end,
					fuels: fuels.length ? fuels : null,
					stations: stations.length ? stations : null,
					pct
				},
				signal
			);
			return { r, days };
		}
	}));

	type Key = keyof Omit<CalendarDay, 'day'>;
	const METRICS = $derived<{ key: Key; title: string; color: { light: string; dark: string } }[]>([
		{ key: 'pn', title: 'Planned (PN)', color: METRIC.pn },
		{ key: 'dispatched', title: 'Dispatched', color: METRIC.dispatched },
		{ key: 'b1610', title: 'Metered (B1610)', color: { light: CHROME.light.ink2, dark: CHROME.dark.ink2 } },
		{ key: 'so_turnup', title: 'Turned up by the SO', color: METRIC.turnup },
		{ key: 'boa_curtailment', title: 'Curtailed through BOAs', color: METRIC.boaCurtailment }
	]);

	const fuelItems = $derived(
		(fuelsQ.data ?? []).map((f) => ({
			value: f,
			label: fuelLabel(f),
			detail: f,
			swatch: fuelGroup(f).color[mode]
		}))
	);

	// Stations offered in the picker follow the fuel filter.
	const stationItems = $derived(
		(stationsQ.data ?? [])
			.filter((s) => fuels.length === 0 || fuels.includes(s.fuel_type))
			.map((s) => ({
				value: s.station,
				label: s.station,
				detail: [s.name, fuelLabel(s.fuel_type)].filter(Boolean).join(', '),
				swatch: fuelGroup(s.fuel_type).color[mode]
			}))
	);

	const mapStations = $derived(
		(stationsQ.data ?? []).filter((s) => fuels.length === 0 || fuels.includes(s.fuel_type))
	);

	function toggleStation(s: string) {
		setParams({ station: stations.includes(s) ? stations.filter((x) => x !== s) : [...stations, s] });
	}

	const selectionText = $derived.by(() => {
		const f = fuels.length === 0 ? 'all fuels' : fuels.map(fuelLabel).join(', ');
		const s = stations.length === 0 ? 'every station' : `${stations.length} station${stations.length === 1 ? '' : 's'}`;
		return `${s}, ${f}`;
	});
</script>

<svelte:head><title>Annual summary · GB grid</title></svelte:head>

<div class="mx-auto flex max-w-[1800px] flex-col gap-4 p-4">
	<div class="flex flex-wrap items-end justify-between gap-3">
		<div>
			<h1 class="font-display text-2xl font-semibold tracking-tight">Daily energy across the year</h1>
			<p class="text-sm text-muted-foreground">Showing {selectionText}.</p>
		</div>
		<div class="flex flex-wrap items-center gap-2">
			<Picker
				items={fuelItems}
				value={fuels}
				onchange={(v) => setParams({ fuel: v.length ? v : '' })}
				multiple
				placeholder="Fuel types"
				emptyLabel="All fuels"
				searchLabel="Search fuel types"
				class="w-44"
			/>
			<Picker
				items={stationItems}
				value={stations}
				onchange={(v) => setParams({ station: v.length ? v : null })}
				multiple
				placeholder="Stations"
				emptyLabel="All stations"
				searchLabel="Search stations by code or name"
				class="w-44"
			/>
			<ToggleGroup.Root
				type="single"
				variant="outline"
				size="sm"
				value={pct ? 'pct' : 'mwh'}
				onValueChange={(v) => v && setParams({ unit: v === 'pct' ? 'pct' : null })}
				aria-label="Units"
			>
				<ToggleGroup.Item value="mwh">MWh per day</ToggleGroup.Item>
				<ToggleGroup.Item value="pct">% of capacity</ToggleGroup.Item>
			</ToggleGroup.Root>
			<TimeRangePicker {range} onchange={setRange} />
		</div>
	</div>

	<div class="grid gap-4 xl:grid-cols-[minmax(280px,22rem)_1fr]">
		<aside class="flex flex-col gap-2">
			<StationMap
				stations={mapStations}
				selected={stations}
				onselect={toggleStation}
				class="h-96 xl:h-[36rem]"
			/>
			<p class="text-xs text-muted-foreground">
				Click stations to add them to the selection, or click again to remove them. The map shows
				stations of the selected fuel types.
			</p>
		</aside>

		<div class="flex min-w-0 flex-col gap-4">
			{#if calQ.error}
				<p class="rounded-md border p-3 text-sm">Couldn't load daily totals: {calQ.error.message}</p>
			{/if}
			{#each METRICS as m (m.key)}
				<Calendar
					title={m.title}
					color={m.color}
					{pct}
					range={calQ.data?.r}
					days={(calQ.data?.days ?? []).map((d) => [d.day, d[m.key]] as [string, number | null])}
					loading={calQ.isFetching}
				/>
			{/each}
		</div>
	</div>
</div>
