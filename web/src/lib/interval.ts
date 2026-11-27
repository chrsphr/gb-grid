// The equivalent of Grafana's $__interval: pick a bucket width so a chart gets
// about one point per pixel. The database functions choose their source tier
// from this width (raw 5-min below an hour, the hourly rollup below a day, the
// daily rollup above), so it has to come from the same ladder of widths.

export interface Step {
	/** Bucket width in milliseconds. */
	ms: number;
	/** The same width as a Postgres interval literal. */
	pg: string;
	/** Short label for display, e.g. "30 min". */
	label: string;
}

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

export const STEPS: Step[] = [
	{ ms: 5 * MIN, pg: '5 minutes', label: '5 min' },
	{ ms: 10 * MIN, pg: '10 minutes', label: '10 min' },
	{ ms: 15 * MIN, pg: '15 minutes', label: '15 min' },
	{ ms: 30 * MIN, pg: '30 minutes', label: '30 min' },
	{ ms: HOUR, pg: '1 hour', label: '1 hour' },
	{ ms: 2 * HOUR, pg: '2 hours', label: '2 hours' },
	{ ms: 3 * HOUR, pg: '3 hours', label: '3 hours' },
	{ ms: 6 * HOUR, pg: '6 hours', label: '6 hours' },
	{ ms: 12 * HOUR, pg: '12 hours', label: '12 hours' },
	{ ms: DAY, pg: '1 day', label: '1 day' },
	{ ms: 7 * DAY, pg: '7 days', label: '7 days' }
];

/**
 * The smallest step that keeps the number of buckets across `spanMs` at or
 * below `maxPoints` (typically the plot's pixel width).
 */
export function pickStep(spanMs: number, maxPoints: number): Step {
	const target = spanMs / Math.max(1, maxPoints);
	return STEPS.find((s) => s.ms >= target) ?? STEPS[STEPS.length - 1];
}
