// Typed client for the PostgREST `api` schema (src/gb_grid/migrations/post-apply.api.sql).
// Every endpoint is a read-only function called with GET /api/rpc/<name>?args.

export class ApiError extends Error {
	constructor(
		message: string,
		readonly status: number,
		readonly hint?: string
	) {
		super(message);
	}
}

type Param = string | number | boolean | Date | string[] | null | undefined;

/** A Postgres array literal for PostgREST, quoting elements that need it. */
function pgArray(values: string[]): string {
	const items = values.map((v) =>
		/^[A-Za-z0-9_.+-]+$/.test(v) ? v : `"${v.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`
	);
	return `{${items.join(',')}}`;
}

function encode(params: Record<string, Param>): string {
	const q = new URLSearchParams();
	for (const [k, v] of Object.entries(params)) {
		if (v === null || v === undefined) continue;
		if (Array.isArray(v)) q.set(k, pgArray(v));
		else if (v instanceof Date) q.set(k, v.toISOString());
		else q.set(k, String(v));
	}
	return q.toString();
}

export async function rpc<T>(
	fn: string,
	params: Record<string, Param> = {},
	signal?: AbortSignal
): Promise<T> {
	const qs = encode(params);
	const res = await fetch(`/api/rpc/${fn}${qs ? `?${qs}` : ''}`, {
		headers: { Accept: 'application/json' },
		signal
	});
	if (!res.ok) {
		let message = `${fn} failed with HTTP ${res.status}`;
		let hint: string | undefined;
		try {
			const body = await res.json();
			if (body?.message) message = body.message;
			hint = body?.hint ?? body?.details ?? undefined;
		} catch {
			// Not a PostgREST error body (e.g. the proxy is down).
		}
		throw new ApiError(message, res.status, hint);
	}
	return res.json() as Promise<T>;
}

// --- Response shapes, one per api function ---------------------------------

export interface Station {
	station: string;
	name: string | null;
	latitude: number | null;
	longitude: number | null;
	fuel_type: string;
	bmus: number;
	has_dispatch: boolean;
}

export interface DispatchRow {
	ts: string;
	bmu: string;
	tier: string;
	pn_mw: number | null;
	boa_level_mw: number | null;
	mel_mw: number | null;
	so_turnup_mw: number | null;
	boa_curtailment_mw: number | null;
	so_curtailment_mw: number | null;
}

export interface B1610Row {
	ts: string;
	tier: string;
	b1610_mw: number | null;
}

export interface DispatchTotals {
	pn_mwh: number | null;
	b1610_mwh: number | null;
	turnup_mwh: number | null;
	turndown_mwh: number | null;
	pn_minus_b1610_mwh: number | null;
}

export interface CalendarDay {
	day: string;
	pn: number | null;
	dispatched: number | null;
	b1610: number | null;
	so_turnup: number | null;
	boa_curtailment: number | null;
}

export interface FuelSeries {
	fuel_type: string;
	ts: number[];
	mw: (number | null)[];
}

export interface GridRow {
	station: string;
	rank: number;
	ts: number[];
	value: number[];
}

export interface ConstraintGroup {
	ord: number;
	label: string;
	constraint_group: string;
}

export interface ConstraintSeries {
	constraint_group: string;
	ts: number[];
	flow_mw: (number | null)[];
	avg_24h_mw: (number | null)[];
	daily_max_mw: (number | null)[];
	daily_min_mw: (number | null)[];
	limit_mw: (number | null)[];
}

// --- Calls ------------------------------------------------------------------

export const api = {
	stations: (signal?: AbortSignal) => rpc<Station[]>('stations', {}, signal),
	fuelTypes: (signal?: AbortSignal) =>
		rpc<{ fuel_type: string }[]>('fuel_types', {}, signal).then((r) => r.map((x) => x.fuel_type)),
	dispatchSeries: (station: string, start: Date, end: Date, step: string, signal?: AbortSignal) =>
		rpc<DispatchRow[]>('dispatch_series', { station, start_ts: start, end_ts: end, step }, signal),
	b1610Series: (station: string, start: Date, end: Date, step: string, signal?: AbortSignal) =>
		rpc<B1610Row[]>('b1610_series', { station, start_ts: start, end_ts: end, step }, signal),
	dispatchTotals: (station: string, start: Date, end: Date, signal?: AbortSignal) =>
		rpc<DispatchTotals[]>('dispatch_totals', { station, start_ts: start, end_ts: end }, signal).then(
			(r) => r[0]
		),
	dailyCalendar: (
		args: { start: Date; end: Date; fuels: string[] | null; stations: string[] | null; pct: boolean },
		signal?: AbortSignal
	) =>
		rpc<CalendarDay[]>(
			'daily_calendar',
			{
				start_ts: args.start,
				end_ts: args.end,
				fuels: args.fuels,
				stations: args.stations,
				pct: args.pct
			},
			signal
		),
	fuelMix: (start: Date, end: Date, step: string, signal?: AbortSignal) =>
		rpc<FuelSeries[]>('fuel_mix', { start_ts: start, end_ts: end, step }, signal),
	stationGrid: (fuel: string, start: Date, end: Date, pct: boolean, signal?: AbortSignal) =>
		rpc<GridRow[]>('station_grid', { fuel, start_ts: start, end_ts: end, pct }, signal),
	constraintGroups: (start: Date, end: Date, signal?: AbortSignal) =>
		rpc<ConstraintGroup[]>('constraint_groups', { start_ts: start, end_ts: end }, signal),
	constraintSeries: (groups: string[], start: Date, end: Date, signal?: AbortSignal) =>
		rpc<ConstraintSeries[]>('constraint_series', { groups, start_ts: start, end_ts: end }, signal)
};
