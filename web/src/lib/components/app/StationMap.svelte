<!--
  Located stations as dots coloured by fuel group. Hover shows the station;
  click calls `onselect`. Selected stations get an ink ring.
-->
<script lang="ts">
	import 'maplibre-gl/dist/maplibre-gl.css';
	// MapLibre runs its tile parsing in a web worker. Point it at the worker as
	// a Vite-built asset (the dep pre-bundler otherwise breaks the worker URL).
	import { setWorkerUrl } from 'maplibre-gl';
	import workerUrl from 'maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url';
	import { CircleLayer, GeoJSONSource, MapLibre, NavigationControl, Popup } from 'svelte-maplibre-gl';
	import type { FeatureCollection, Point } from 'geojson';
	import type { Station } from '#lib/api.js';
	import { currentMode } from '#lib/mode.svelte.js';
	import { CHROME, FUEL_GROUPS, fuelGroup, fuelLabel } from '#lib/palette.js';

	interface Props {
		stations: Station[];
		selected?: string[];
		onselect?: (station: string) => void;
		class?: string;
	}

	let { stations, selected = [], onselect, class: cls = '' }: Props = $props();

	setWorkerUrl(workerUrl);

	const mode = $derived(currentMode());
	const style = $derived(
		mode === 'dark'
			? 'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json'
			: 'https://basemaps.cartocdn.com/gl/positron-gl-style/style.json'
	);

	const located = $derived(stations.filter((s) => s.latitude !== null && s.longitude !== null));
	const selectedSet = $derived(new Set(selected));

	const data = $derived<FeatureCollection>({
		type: 'FeatureCollection',
		features: located.map((s) => ({
			type: 'Feature',
			geometry: { type: 'Point', coordinates: [s.longitude!, s.latitude!] },
			properties: {
				station: s.station,
				name: s.name ?? '',
				fuel: s.fuel_type,
				bmus: s.bmus,
				color: fuelGroup(s.fuel_type).color[mode],
				selected: selectedSet.has(s.station)
			}
		}))
	});

	const groupsPresent = $derived(
		FUEL_GROUPS.filter((g) => located.some((s) => fuelGroup(s.fuel_type).id === g.id))
	);

	let hovered = $state<{ lnglat: [number, number]; station: string; name: string; fuel: string; bmus: number } | null>(null);
</script>

<div class="relative overflow-hidden rounded-md border bg-card {cls}">
	<MapLibre
		class="size-full"
		{style}
		bounds={[
			[-8.6, 49.8],
			[2.0, 60.9]
		]}
		fitBoundsOptions={{ padding: 8 }}
		minZoom={3}
		maxZoom={11}
		attributionControl={{ compact: true }}
		autoloadGlobalCss={false}
		cursor={hovered ? 'pointer' : ''}
	>
		<NavigationControl position="top-left" showCompass={false} />
		<GeoJSONSource id="stations" {data}>
			<CircleLayer
				id="stations-dots"
				paint={{
					'circle-color': ['get', 'color'],
					'circle-radius': ['interpolate', ['linear'], ['zoom'], 4, 3.5, 8, 7],
					'circle-opacity': 0.9,
					'circle-stroke-color': ['case', ['get', 'selected'], CHROME[mode].ink, CHROME[mode].surface],
					'circle-stroke-width': ['case', ['get', 'selected'], 2.5, 0.75]
				}}
				onmousemove={(e) => {
					const f = e.features?.[0];
					if (!f) return;
					const p = f.properties as { station: string; name: string; fuel: string; bmus: number };
					const [lng, lat] = (f.geometry as Point).coordinates;
					hovered = { lnglat: [lng, lat], ...p };
				}}
				onmouseleave={() => (hovered = null)}
				onclick={(e) => {
					const s = e.features?.[0]?.properties?.station;
					if (s) onselect?.(s);
				}}
			/>
		</GeoJSONSource>
		{#if hovered}
			<Popup lnglat={hovered.lnglat} closeButton={false} offset={10} class="station-popup">
				<div class="font-display text-sm font-semibold text-foreground">{hovered.station}</div>
				{#if hovered.name}<div class="text-xs text-muted-foreground">{hovered.name}</div>{/if}
				<div class="mt-1 text-xs text-muted-foreground">
					{fuelLabel(hovered.fuel)}, {hovered.bmus} BMU{hovered.bmus === 1 ? '' : 's'}
				</div>
			</Popup>
		{/if}
	</MapLibre>

	<ul
		class="pointer-events-none absolute bottom-2 left-2 grid gap-0.5 rounded-md bg-card/90 px-2 py-1.5 text-xs text-muted-foreground shadow-sm backdrop-blur-sm"
		aria-label="Fuel colours"
	>
		{#each groupsPresent as g (g.id)}
			<li class="flex items-center gap-1.5">
				<span class="size-2 rounded-full" style="background:{g.color[mode]}"></span>{g.label}
			</li>
		{/each}
	</ul>
</div>

<style>
	:global(.station-popup .maplibregl-popup-content) {
		background: var(--popover);
		color: var(--popover-foreground);
		border: 1px solid var(--border);
		border-radius: 6px;
		padding: 6px 10px;
		box-shadow: 0 4px 16px rgb(0 0 0 / 0.15);
	}
	:global(.station-popup .maplibregl-popup-tip) {
		display: none;
	}
</style>
