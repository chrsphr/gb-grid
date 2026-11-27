// Shared chart chrome: recessive hairline axes and grid, ink-coloured text,
// a crosshair tooltip. Pages build their series on top of these pieces.
import { CHROME, type Mode } from '../palette';

export const FONT = "Barlow, system-ui, sans-serif";

const nf0 = new Intl.NumberFormat('en-GB', { maximumFractionDigits: 0 });
const nf1 = new Intl.NumberFormat('en-GB', { maximumFractionDigits: 1 });

export function fmt(v: number | null | undefined, unit = ''): string {
	if (v === null || v === undefined || Number.isNaN(v)) return '–';
	const s = Math.abs(v) >= 100 ? nf0.format(v) : nf1.format(v);
	return unit ? `${s} ${unit}` : s;
}

const tipTime = new Intl.DateTimeFormat('en-GB', {
	weekday: 'short',
	day: 'numeric',
	month: 'short',
	hour: '2-digit',
	minute: '2-digit'
});

export function fmtTime(ms: number): string {
	return tipTime.format(new Date(ms));
}

export function base(mode: Mode) {
	const c = CHROME[mode];
	return {
		backgroundColor: 'transparent',
		animation: false,
		textStyle: { fontFamily: FONT, color: c.ink2, fontSize: 12 },
		tooltip: {
			backgroundColor: c.surface,
			borderColor: c.axis,
			borderWidth: 1,
			padding: [8, 10],
			textStyle: { color: c.ink, fontFamily: FONT, fontSize: 12 },
			extraCssText: 'border-radius: 6px; box-shadow: 0 4px 16px rgb(0 0 0 / 0.12);'
		}
	};
}

export function timeAxis(mode: Mode, min?: number, max?: number, show = true) {
	const c = CHROME[mode];
	return {
		type: 'time' as const,
		min,
		max,
		axisLine: { show, lineStyle: { color: c.axis } },
		axisTick: { show: false },
		splitLine: { show: false },
		axisLabel: {
			show,
			color: c.muted,
			hideOverlap: true,
			formatter: {
				year: '{yyyy}',
				month: '{MMM}',
				day: '{d} {MMM}',
				hour: '{HH}:{mm}',
				minute: '{HH}:{mm}',
				second: '{HH}:{mm}:{ss}'
			}
		},
		axisPointer: {
			lineStyle: { color: c.muted, width: 1 },
			label: { show: false }
		}
	};
}

export function valueAxis(mode: Mode, unit: string, opts: { min?: number; max?: number } = {}) {
	const c = CHROME[mode];
	return {
		type: 'value' as const,
		min: opts.min,
		max: opts.max,
		splitNumber: 4,
		axisLine: { show: false },
		axisTick: { show: false },
		splitLine: { lineStyle: { color: c.grid, width: 1 } },
		axisLabel: {
			color: c.muted,
			formatter: (v: number) => `${nf0.format(v)}${unit ? ` ${unit}` : ''}`
		}
	};
}

export function legend(mode: Mode) {
	const c = CHROME[mode];
	return {
		bottom: 0,
		left: 0,
		icon: 'roundRect',
		itemWidth: 12,
		itemHeight: 4,
		itemGap: 16,
		textStyle: { color: c.ink2, fontFamily: FONT, fontSize: 12 },
		inactiveColor: c.axis
	};
}

interface AxisParam {
	axisValue: number;
	seriesName: string;
	color: string;
	value: [number, number | null] | number | null;
	data?: unknown;
}

/**
 * Crosshair tooltip: the time, then one row per series with a colour key,
 * the name in secondary ink and the value in primary ink.
 */
export function axisTooltip(mode: Mode, unit: string, opts: { hideZero?: boolean } = {}) {
	const c = CHROME[mode];
	return {
		trigger: 'axis' as const,
		axisPointer: { type: 'line' as const, lineStyle: { color: c.muted, width: 1 } },
		formatter: (raw: AxisParam | AxisParam[]) => {
			const params = Array.isArray(raw) ? raw : [raw];
			if (!params.length) return '';
			const head = `<div style="color:${c.muted};margin-bottom:4px">${fmtTime(Number(params[0].axisValue))}</div>`;
			const rows = params
				.map((p) => {
					const v = Array.isArray(p.value) ? p.value[1] : p.value;
					if (v === null || v === undefined || (opts.hideZero && v === 0)) return '';
					return `<div style="display:flex;align-items:center;gap:8px;line-height:1.6">
						<span style="width:10px;height:3px;border-radius:2px;background:${p.color}"></span>
						<span style="color:${c.ink2};flex:1">${p.seriesName}</span>
						<span style="color:${c.ink};font-variant-numeric:tabular-nums;margin-left:12px">${fmt(Number(v), unit)}</span>
					</div>`;
				})
				.join('');
			return head + rows;
		}
	};
}
