<!--
  Energy totals for the window, as a ledger rather than a row of tiles: each
  line carries the colour key of the series it sums on the charts.
-->
<script lang="ts">
	import type { DispatchTotals } from '#lib/api.js';
	import { CHROME, METRIC, type Mode } from '#lib/palette.js';

	interface Props {
		totals: DispatchTotals | undefined;
		loading: boolean;
		mode: Mode;
	}

	let { totals, loading, mode }: Props = $props();

	const nf = new Intl.NumberFormat('en-GB', { maximumFractionDigits: 0 });
	const fmt = (v: number | null | undefined) => (v === null || v === undefined ? '–' : nf.format(v));

	const rows = $derived([
		{ label: 'Planned (PN)', value: totals?.pn_mwh, key: METRIC.pn[mode] },
		{ label: 'Metered (B1610)', value: totals?.b1610_mwh, key: CHROME[mode].ink },
		{ label: 'Turned up by the SO', value: totals?.turnup_mwh, key: METRIC.turnup[mode] },
		{ label: 'Curtailed through BOAs', value: totals?.turndown_mwh, key: METRIC.boaCurtailment[mode] }
	]);
</script>

<section class="rounded-md border bg-card px-4 py-3" aria-labelledby="ledger-title">
	<div class="flex items-baseline justify-between">
		<h2 id="ledger-title" class="font-display text-base font-semibold">Energy</h2>
		<span class="text-xs text-muted-foreground">MWh, whole UTC days in the window</span>
	</div>
	<table class="figures mt-2 w-full text-sm" class:opacity-50={loading}>
		<tbody>
			{#each rows as r (r.label)}
				<tr class="border-b border-border/60 last:border-0">
					<th scope="row" class="py-1.5 text-left font-normal text-muted-foreground">
						<span class="mr-2 inline-block h-[3px] w-3 rounded-full align-middle" style="background:{r.key}"></span>{r.label}
					</th>
					<td class="py-1.5 text-right">{fmt(r.value)}</td>
				</tr>
			{/each}
			<tr class="border-t-2 border-border">
				<th scope="row" class="py-1.5 text-left font-medium">Planned minus metered</th>
				<td class="py-1.5 text-right font-medium">{fmt(totals?.pn_minus_b1610_mwh)}</td>
			</tr>
		</tbody>
	</table>
	{#if totals && totals.pn_mwh !== null && totals.b1610_mwh === null}
		<p class="mt-2 text-xs text-muted-foreground">
			Metered data arrives about five working days late, so recent days have none yet.
		</p>
	{:else if totals && totals.pn_mwh === null}
		<p class="mt-2 text-xs text-muted-foreground">
			No daily totals for these days yet. They're built from the dispatch data once a day is
			complete.
		</p>
	{/if}
</section>
