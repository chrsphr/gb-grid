// Page state lives in the URL query string, so every view can be bookmarked
// or shared as a link. Reads are reactive through `page.url`.
import { goto } from '$app/navigation';
import { page } from '$app/state';
import type { TimeRange } from './time';

type Value = string | string[] | null | undefined;

export function param(name: string): string | null {
	return page.url.searchParams.get(name);
}

/** A comma-separated list parameter; null when absent (meaning "all"). */
export function listParam(name: string): string[] | null {
	const v = page.url.searchParams.get(name);
	if (v === null) return null;
	return v === '' ? [] : v.split(',');
}

export function timeRange(defaults: TimeRange): TimeRange {
	return {
		from: page.url.searchParams.get('from') ?? defaults.from,
		to: page.url.searchParams.get('to') ?? defaults.to
	};
}

/** Merge `updates` into the query string (null removes a key) and navigate. */
export function setParams(updates: Record<string, Value>, opts: { replace?: boolean } = {}) {
	const url = new URL(page.url.href);
	for (const [k, v] of Object.entries(updates)) {
		if (v === null || v === undefined) url.searchParams.delete(k);
		else url.searchParams.set(k, Array.isArray(v) ? v.join(',') : v);
	}
	if (url.search === page.url.search) return;
	goto(url, { replace: opts.replace ?? false, reset: false });
}

export function setRange(range: TimeRange) {
	setParams({ from: range.from, to: range.to });
}
