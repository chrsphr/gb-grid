<!--
  A thin ECharts host. Pass a complete `option`; it is applied with notMerge
  whenever it changes. Charts that share a `group` share a crosshair and
  tooltip. With `onzoom`, dragging across the plot selects a time window and
  reports it instead of zooming in place, so the page can refetch at a finer
  resolution.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { echarts, type Option } from './echarts';

	interface Props {
		option: Option;
		group?: string;
		onzoom?: (start: number, end: number) => void;
		onclick?: (params: { data?: unknown; value?: unknown; name?: string; seriesName?: string }) => void;
		class?: string;
		label: string;
		/** Hold the last render dimmed while new data loads. */
		stale?: boolean;
	}

	let { option, group, onzoom, onclick, class: cls = '', label, stale = false }: Props = $props();

	let el: HTMLDivElement;
	let chart = $state<echarts.ECharts>();

	onMount(() => {
		const c = echarts.init(el, null, { renderer: 'canvas' });
		chart = c;
		if (group) {
			c.group = group;
			echarts.connect(group);
		}
		const ro = new ResizeObserver(() => c.resize());
		ro.observe(el);

		c.on('datazoom', () => {
			if (!onzoom) return;
			const dz = (c.getOption() as { dataZoom?: { startValue?: number; endValue?: number }[] })
				.dataZoom?.[0];
			if (dz?.startValue !== undefined && dz?.endValue !== undefined && dz.endValue > dz.startValue) {
				onzoom(Number(dz.startValue), Number(dz.endValue));
			}
		});
		c.on('click', (p) => onclick?.(p as never));

		return () => {
			ro.disconnect();
			c.dispose();
		};
	});

	$effect(() => {
		if (!chart) return;
		const opt = onzoom
			? {
					...option,
					// The toolbox's box-select zoom, driven from code (its buttons stay hidden).
					toolbox: { show: false, feature: { dataZoom: { yAxisIndex: 'none', filterMode: 'none' } } }
				}
			: option;
		chart.setOption(opt, { notMerge: true });
		if (onzoom) {
			chart.dispatchAction({ type: 'takeGlobalCursor', key: 'dataZoomSelect', dataZoomSelectActive: true });
		}
	});
</script>

<div
	bind:this={el}
	class="w-full transition-opacity duration-150 {cls}"
	class:opacity-50={stale}
	role="img"
	aria-label={label}
></div>
