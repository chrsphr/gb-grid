// Time ranges in the URL use Grafana's syntax so old dashboard links translate
// directly: `from=now-24h&to=now`, or absolute ISO timestamps / epoch ms.
// Relative bounds are re-resolved against the clock on every fetch, so an
// auto-refreshing page keeps sliding forward.

export interface TimeRange {
	from: string;
	to: string;
}

export interface Resolved {
	start: Date;
	end: Date;
}

const UNIT_MS: Record<string, number> = {
	s: 1_000,
	m: 60_000,
	h: 3_600_000,
	d: 86_400_000,
	w: 7 * 86_400_000
};

const RELATIVE = /^now(?:([+-])(\d+)([smhdwMy]))?$/;

export function isRelative(bound: string): boolean {
	return RELATIVE.test(bound.trim());
}

/** Resolve one bound (`now`, `now-7d`, ISO, or epoch ms) to a Date. */
export function parseBound(bound: string, now: Date = new Date()): Date | null {
	const s = bound.trim();
	const rel = RELATIVE.exec(s);
	if (rel) {
		const [, sign, nStr, unit] = rel;
		if (!sign) return new Date(now);
		const n = Number(nStr) * (sign === '-' ? -1 : 1);
		const d = new Date(now);
		if (unit === 'M') d.setMonth(d.getMonth() + n);
		else if (unit === 'y') d.setFullYear(d.getFullYear() + n);
		else d.setTime(d.getTime() + n * UNIT_MS[unit]);
		return d;
	}
	if (/^\d{10,}$/.test(s)) return new Date(Number(s));
	const d = new Date(s);
	return Number.isNaN(d.getTime()) ? null : d;
}

export function resolve(range: TimeRange, now: Date = new Date()): Resolved | null {
	const start = parseBound(range.from, now);
	const end = parseBound(range.to, now);
	if (!start || !end || start >= end) return null;
	return { start, end };
}

export function isLive(range: TimeRange): boolean {
	return range.to.trim() === 'now';
}

/** An absolute range, serialised compactly for the URL. */
export function absolute(start: Date | number, end: Date | number): TimeRange {
	return { from: new Date(start).toISOString(), to: new Date(end).toISOString() };
}

/**
 * Move the window by half its width (Grafana's ◀ ▶ step). Moving forward
 * never passes the present.
 */
export function shift(range: TimeRange, direction: -1 | 1, now: Date = new Date()): TimeRange {
	const r = resolve(range, now);
	if (!r) return range;
	const span = r.end.getTime() - r.start.getTime();
	let end = r.end.getTime() + (direction * span) / 2;
	if (end > now.getTime()) end = now.getTime();
	return absolute(end - span, end);
}

/** Double the window around its centre; a live window stays live. */
export function zoomOut(range: TimeRange, now: Date = new Date()): TimeRange {
	const r = resolve(range, now);
	if (!r) return range;
	const span = r.end.getTime() - r.start.getTime();
	if (isLive(range)) {
		return { from: new Date(now.getTime() - 2 * span).toISOString(), to: 'now' };
	}
	const mid = (r.start.getTime() + r.end.getTime()) / 2;
	const end = Math.min(mid + span, now.getTime());
	return absolute(end - 2 * span, end);
}

export interface QuickRange extends TimeRange {
	label: string;
}

export const QUICK_RANGES: QuickRange[] = [
	{ label: 'Last 6 hours', from: 'now-6h', to: 'now' },
	{ label: 'Last 24 hours', from: 'now-24h', to: 'now' },
	{ label: 'Last 2 days', from: 'now-2d', to: 'now' },
	{ label: 'Last 7 days', from: 'now-7d', to: 'now' },
	{ label: 'Last 30 days', from: 'now-30d', to: 'now' },
	{ label: 'Last 90 days', from: 'now-90d', to: 'now' },
	{ label: 'Last year', from: 'now-1y', to: 'now' }
];

const dayFmt = new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short', year: 'numeric' });
const dateTimeFmt = new Intl.DateTimeFormat('en-GB', {
	day: 'numeric',
	month: 'short',
	year: 'numeric',
	hour: '2-digit',
	minute: '2-digit'
});
const timeFmt = new Intl.DateTimeFormat('en-GB', { hour: '2-digit', minute: '2-digit' });

function isMidnight(d: Date): boolean {
	return d.getHours() === 0 && d.getMinutes() === 0 && d.getSeconds() === 0;
}

/** Human label for the picker button: a quick-range name or the two dates. */
export function describe(range: TimeRange, now: Date = new Date()): string {
	const quick = QUICK_RANGES.find((q) => q.from === range.from && q.to === range.to);
	if (quick) return quick.label;
	const rel = RELATIVE.exec(range.from.trim());
	if (rel && rel[1] === '-' && isLive(range)) {
		const units: Record<string, string> = {
			s: 'second', m: 'minute', h: 'hour', d: 'day', w: 'week', M: 'month', y: 'year'
		};
		const n = Number(rel[2]);
		return `Last ${n} ${units[rel[3]]}${n === 1 ? '' : 's'}`;
	}
	const r = resolve(range, now);
	if (!r) return 'Invalid range';
	if (isMidnight(r.start) && isMidnight(r.end)) {
		const lastDay = new Date(r.end.getTime() - 1);
		return lastDay.toDateString() === r.start.toDateString()
			? dayFmt.format(r.start)
			: `${dayFmt.format(r.start)} – ${dayFmt.format(lastDay)}`;
	}
	if (r.start.toDateString() === r.end.toDateString()) {
		return `${dateTimeFmt.format(r.start)} – ${timeFmt.format(r.end)}`;
	}
	return `${dateTimeFmt.format(r.start)} – ${dateTimeFmt.format(r.end)}`;
}
