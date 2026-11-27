// Colour for the charts and map. Every set here was run through the dataviz
// skill's validator (CVD and normal-vision separation, contrast) against the
// light (#fcfcfc) and dark (#181c20) chart surfaces in layout.css. Changing a
// colour or an order means re-validating it.

export type Mode = 'light' | 'dark';
type Pair = { light: string; dark: string };

// --- Fuel groups --------------------------------------------------------------
// BMRS has ~20 fuel codes. Charts fold them into eight groups (the categorical
// palette's limit). GB coal ended in 2024 and oil is negligible, so both join
// "Other". Groups are listed in stacking order, bottom to top: this order is
// the one that passes the adjacent-pair checks in both modes.

export interface FuelGroup {
	id: string;
	label: string;
	color: Pair;
}

export const FUEL_GROUPS: FuelGroup[] = [
	{ id: 'nuclear', label: 'Nuclear', color: { light: '#6250d6', dark: '#9085e9' } },
	{ id: 'biomass', label: 'Biomass', color: { light: '#e34948', dark: '#e66767' } },
	{ id: 'ps', label: 'Pumped storage', color: { light: '#1baf7a', dark: '#199e70' } },
	{ id: 'gas', label: 'Gas', color: { light: '#eb6834', dark: '#d95926' } },
	{ id: 'hydro', label: 'Hydro', color: { light: '#2a78d6', dark: '#3987e5' } },
	{ id: 'wind', label: 'Wind', color: { light: '#008300', dark: '#0ca30c' } },
	{ id: 'int', label: 'Interconnectors', color: { light: '#e87ba4', dark: '#d55181' } },
	{ id: 'other', label: 'Other', color: { light: '#eda100', dark: '#c98500' } }
];

const GROUP_BY_ID = new Map(FUEL_GROUPS.map((g) => [g.id, g]));

/** The chart group for a BMRS fuel code (fuelinst or bmu.bmrs_fuel_type). */
export function fuelGroup(fuel: string | null | undefined): FuelGroup {
	const f = (fuel ?? '').toUpperCase();
	let id = 'other';
	if (f === 'NUCLEAR') id = 'nuclear';
	else if (f === 'BIOMASS') id = 'biomass';
	else if (f === 'PS') id = 'ps';
	else if (f === 'CCGT' || f === 'OCGT') id = 'gas';
	else if (f === 'NPSHYD') id = 'hydro';
	else if (f === 'WIND') id = 'wind';
	else if (f.startsWith('INT')) id = 'int';
	return GROUP_BY_ID.get(id)!;
}

// Plain-language names for the fuel codes users pick from.
const FUEL_LABELS: Record<string, string> = {
	BIOMASS: 'Biomass',
	CCGT: 'Gas (CCGT)',
	OCGT: 'Gas (OCGT)',
	COAL: 'Coal',
	NPSHYD: 'Hydro',
	NUCLEAR: 'Nuclear',
	OIL: 'Oil',
	OTHER: 'Other',
	PS: 'Pumped storage',
	WIND: 'Wind',
	UNKNOWN: 'Unknown'
};

export function fuelLabel(fuel: string): string {
	if (FUEL_LABELS[fuel]) return FUEL_LABELS[fuel];
	if (fuel.startsWith('INT')) return `Interconnector ${fuel.slice(3)}`;
	return fuel;
}

// --- Dispatch metrics ---------------------------------------------------------
// One colour per metric across the dispatch and annual pages. PN / dispatched
// are categorical slots 1-2; turn-up / BOA curtailment / SO curtailment were
// validated as an all-pairs set (they share one chart, separated by sign).

export const METRIC = {
	pn: { light: '#2a78d6', dark: '#3987e5' },
	dispatched: { light: '#eb6834', dark: '#d95926' },
	turnup: { light: '#1baf7a', dark: '#199e70' },
	boaCurtailment: { light: '#e34948', dark: '#e66767' },
	soCurtailment: { light: '#6250d6', dark: '#9085e9' }
} satisfies Record<string, Pair>;

// Categorical slots in fixed order, for series with no fixed identity (BMUs of
// a station). A ninth series is folded into "Other", never a new hue.
export const CATEGORICAL: Pair[] = [
	{ light: '#2a78d6', dark: '#3987e5' },
	{ light: '#eb6834', dark: '#d95926' },
	{ light: '#1baf7a', dark: '#199e70' },
	{ light: '#eda100', dark: '#c98500' },
	{ light: '#e87ba4', dark: '#d55181' },
	{ light: '#008300', dark: '#0ca30c' },
	{ light: '#6250d6', dark: '#9085e9' },
	{ light: '#e34948', dark: '#e66767' }
];

// --- Chart chrome ---------------------------------------------------------------

export interface Chrome {
	surface: string;
	ink: string;
	ink2: string;
	muted: string;
	grid: string;
	axis: string;
	/** Midpoint of diverging scales: reads as "nothing". */
	neutral: string;
}

export const CHROME: Record<Mode, Chrome> = {
	light: {
		surface: '#fcfcfc',
		ink: '#14181c',
		ink2: '#4b555e',
		muted: '#7d8790',
		grid: '#e5e8eb',
		axis: '#c5ccd2',
		neutral: '#eef0f2'
	},
	dark: {
		surface: '#181c20',
		ink: '#eceff1',
		ink2: '#c3cad0',
		muted: '#8a959e',
		grid: '#262c31',
		axis: '#3a4249',
		neutral: '#2a3036'
	}
};

export const pick = (p: Pair, mode: Mode) => p[mode];

/**
 * A one-hue sequential ramp for magnitude: from (almost) the surface to the
 * full hue, so "near zero" recedes in both modes.
 */
export function sequential(p: Pair, mode: Mode): string[] {
	const hue = p[mode];
	return [mix(CHROME[mode].surface, hue, 0.12), mix(CHROME[mode].surface, hue, 0.55), hue];
}

/** Diverging: blue (negative) through a neutral grey to red (positive). */
export function diverging(mode: Mode): string[] {
	return [
		mode === 'light' ? '#2a78d6' : '#3987e5',
		CHROME[mode].neutral,
		mode === 'light' ? '#e34948' : '#e66767'
	];
}

function mix(a: string, b: string, t: number): string {
	const pa = parseInt(a.slice(1), 16);
	const pb = parseInt(b.slice(1), 16);
	const ch = (shift: number) => {
		const x = (pa >> shift) & 255;
		const y = (pb >> shift) & 255;
		return Math.round(x + (y - x) * t);
	};
	return `#${((ch(16) << 16) | (ch(8) << 8) | ch(0)).toString(16).padStart(6, '0')}`;
}
