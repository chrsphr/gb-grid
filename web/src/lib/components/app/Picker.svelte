<!--
  A searchable picker over a long list (e.g. 2,000+ stations), single or multi
  select. Filters on the label and an optional detail line, and shows at most
  `limit` matches so the list stays fast.
-->
<script lang="ts" generics="T extends { value: string; label: string; detail?: string; swatch?: string }">
	import Check from '@lucide/svelte/icons/check';
	import ChevronsUpDown from '@lucide/svelte/icons/chevrons-up-down';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Command from '#lib/components/ui/command/index.js';
	import * as Popover from '#lib/components/ui/popover/index.js';

	interface Props {
		items: T[];
		/** Selected values. For single select, one value or none. */
		value: string[];
		onchange: (value: string[]) => void;
		multiple?: boolean;
		placeholder: string;
		/** Button text when nothing (multi: everything) is selected. */
		emptyLabel?: string;
		searchLabel?: string;
		limit?: number;
		class?: string;
	}

	let {
		items,
		value,
		onchange,
		multiple = false,
		placeholder,
		emptyLabel,
		searchLabel = 'Search',
		limit = 80,
		class: cls = ''
	}: Props = $props();

	let open = $state(false);
	let search = $state('');

	const selected = $derived(new Set(value));
	const matches = $derived.by(() => {
		const q = search.trim().toLowerCase();
		const hits = q
			? items.filter(
					(i) => i.label.toLowerCase().includes(q) || (i.detail ?? '').toLowerCase().includes(q)
				)
			: items;
		// Selected items first, so they're easy to clear.
		const sorted = multiple
			? [...hits.filter((i) => selected.has(i.value)), ...hits.filter((i) => !selected.has(i.value))]
			: hits;
		return { shown: sorted.slice(0, limit), total: hits.length };
	});

	const buttonText = $derived.by(() => {
		if (value.length === 0) return emptyLabel ?? placeholder;
		if (value.length === 1) return items.find((i) => i.value === value[0])?.label ?? value[0];
		return `${value.length} selected`;
	});

	function choose(v: string) {
		if (!multiple) {
			onchange([v]);
			open = false;
			search = '';
			return;
		}
		onchange(selected.has(v) ? value.filter((x) => x !== v) : [...value, v]);
	}
</script>

<Popover.Root bind:open>
	<Popover.Trigger>
		{#snippet child({ props })}
			<Button
				{...props}
				variant="outline"
				size="sm"
				role="combobox"
				aria-expanded={open}
				aria-label={placeholder}
				class="justify-between font-normal {cls}"
			>
				<span class="truncate">{buttonText}</span>
				<ChevronsUpDown class="opacity-50" />
			</Button>
		{/snippet}
	</Popover.Trigger>
	<Popover.Content class="w-72 p-0" align="start">
		<Command.Root shouldFilter={false}>
			<Command.Input placeholder={searchLabel} bind:value={search} />
			<Command.List class="max-h-80">
				<Command.Empty>Nothing matches “{search}”.</Command.Empty>
				{#if multiple && value.length > 0}
					<Command.Item value="__clear" onSelect={() => onchange([])} class="text-muted-foreground">
						Clear selection
					</Command.Item>
				{/if}
				{#each matches.shown as item (item.value)}
					<Command.Item value={item.value} onSelect={() => choose(item.value)}>
						{#if item.swatch}
							<span class="size-2 shrink-0 rounded-full" style="background:{item.swatch}"></span>
						{/if}
						<span class="flex min-w-0 flex-col">
							<span class="truncate">{item.label}</span>
							{#if item.detail}<span class="truncate text-xs text-muted-foreground">{item.detail}</span>{/if}
						</span>
						<Check class="ml-auto {selected.has(item.value) ? 'opacity-100' : 'opacity-0'}" />
					</Command.Item>
				{/each}
				{#if matches.total > matches.shown.length}
					<div class="px-2 py-1.5 text-xs text-muted-foreground">
						{matches.total - matches.shown.length} more. Type to narrow the list.
					</div>
				{/if}
			</Command.List>
		</Command.Root>
	</Popover.Content>
</Popover.Root>
