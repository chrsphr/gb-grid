<!--
  The page's time window: step back / forward by half a window, zoom out, and a
  popover of preset ranges plus a calendar for whole days. Optional refresh
  controls for live pages.
-->
<script lang="ts">
	import ChevronLeft from '@lucide/svelte/icons/chevron-left';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import Clock from '@lucide/svelte/icons/clock';
	import RefreshCw from '@lucide/svelte/icons/refresh-cw';
	import ZoomOut from '@lucide/svelte/icons/zoom-out';
	import { getLocalTimeZone, today, CalendarDate, type DateValue } from '@internationalized/date';
	import type { DateRange } from 'bits-ui';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import { RangeCalendar } from '#lib/components/ui/range-calendar/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { QUICK_RANGES, absolute, describe, resolve, shift, zoomOut, type TimeRange } from '#lib/time.js';

	interface Props {
		range: TimeRange;
		onchange: (range: TimeRange) => void;
		/** Refresh controls are shown when `onrefresh` is given. */
		onrefresh?: () => void;
		refresh?: string;
		onrefreshchange?: (value: string) => void;
		refreshing?: boolean;
	}

	let { range, onchange, onrefresh, refresh = 'off', onrefreshchange, refreshing = false }: Props =
		$props();

	let open = $state(false);
	let days = $state<DateRange | undefined>();

	const label = $derived(describe(range));

	function toCalendar(d: Date): CalendarDate {
		return new CalendarDate(d.getFullYear(), d.getMonth() + 1, d.getDate());
	}

	$effect(() => {
		if (!open) return;
		const r = resolve(range);
		if (r) days = { start: toCalendar(r.start), end: toCalendar(new Date(r.end.getTime() - 1)) };
	});

	function applyDays() {
		if (!days?.start || !days?.end) return;
		const tz = getLocalTimeZone();
		const start = (days.start as DateValue).toDate(tz);
		const end = (days.end as DateValue).add({ days: 1 }).toDate(tz);
		onchange(absolute(start, Math.min(end.getTime(), Date.now())));
		open = false;
	}

	function pickQuick(q: TimeRange) {
		onchange({ from: q.from, to: q.to });
		open = false;
	}

	const REFRESH = [
		{ value: 'off', label: 'Off' },
		{ value: '30s', label: '30 s' },
		{ value: '1m', label: '1 min' },
		{ value: '5m', label: '5 min' }
	];
</script>

<div class="flex flex-wrap items-center gap-1">
	<Button variant="outline" size="icon-sm" aria-label="Earlier" onclick={() => onchange(shift(range, -1))}>
		<ChevronLeft />
	</Button>

	<Popover.Root bind:open>
		<Popover.Trigger>
			{#snippet child({ props })}
				<Button {...props} variant="outline" size="sm" class="min-w-44 justify-start font-normal">
					<Clock class="text-muted-foreground" />
					<span class="figures">{label}</span>
				</Button>
			{/snippet}
		</Popover.Trigger>
		<Popover.Content align="end" class="w-auto p-0">
			<div class="flex flex-col sm:flex-row">
				<ul class="grid content-start gap-0.5 border-b p-2 sm:w-40 sm:border-r sm:border-b-0">
					{#each QUICK_RANGES as q (q.from)}
						<li>
							<button
								type="button"
								class="w-full rounded-sm px-2 py-1.5 text-left text-sm hover:bg-accent focus-visible:bg-accent focus-visible:outline-none"
								class:font-semibold={q.from === range.from && q.to === range.to}
								onclick={() => pickQuick(q)}
							>
								{q.label}
							</button>
						</li>
					{/each}
				</ul>
				<div class="p-2">
					<RangeCalendar
						bind:value={days}
						locale="en-GB"
						weekStartsOn={1}
						maxValue={today(getLocalTimeZone())}
						numberOfMonths={1}
					/>
					<div class="flex justify-end px-2 pb-1">
						<Button size="sm" disabled={!days?.start || !days?.end} onclick={applyDays}>Show these days</Button>
					</div>
				</div>
			</div>
		</Popover.Content>
	</Popover.Root>

	<Button variant="outline" size="icon-sm" aria-label="Later" onclick={() => onchange(shift(range, 1))}>
		<ChevronRight />
	</Button>
	<Button variant="outline" size="icon-sm" aria-label="Zoom out" onclick={() => onchange(zoomOut(range))}>
		<ZoomOut />
	</Button>

	{#if onrefresh}
		<div class="ml-1 flex items-center">
			<Button
				variant="outline"
				size="sm"
				class="rounded-r-none"
				aria-label="Refresh now"
				onclick={onrefresh}
			>
				<RefreshCw class={refreshing ? 'animate-spin' : ''} />
			</Button>
			<Select.Root type="single" value={refresh} onValueChange={(v) => onrefreshchange?.(v)}>
				<Select.Trigger size="sm" class="-ml-px rounded-l-none" aria-label="Auto-refresh">
					{refresh === 'off' ? 'Auto off' : `Every ${REFRESH.find((r) => r.value === refresh)?.label}`}
				</Select.Trigger>
				<Select.Content>
					{#each REFRESH as r (r.value)}
						<Select.Item value={r.value}>{r.label === 'Off' ? 'Auto-refresh off' : `Every ${r.label}`}</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</div>
	{/if}
</div>
