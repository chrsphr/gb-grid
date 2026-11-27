import { describe, expect, it } from 'vitest';
import { describe as label, isLive, parseBound, resolve, shift, zoomOut } from './time';

const now = new Date('2026-05-14T12:00:00Z');

describe('parseBound', () => {
	it('resolves relative bounds against now', () => {
		expect(parseBound('now', now)?.toISOString()).toBe('2026-05-14T12:00:00.000Z');
		expect(parseBound('now-24h', now)?.toISOString()).toBe('2026-05-13T12:00:00.000Z');
		expect(parseBound('now-7d', now)?.toISOString()).toBe('2026-05-07T12:00:00.000Z');
		expect(parseBound('now-30m', now)?.toISOString()).toBe('2026-05-14T11:30:00.000Z');
	});
	it('handles calendar units', () => {
		expect(parseBound('now-1y', now)?.getFullYear()).toBe(2025);
		expect(parseBound('now-1M', now)?.getMonth()).toBe(3);
	});
	it('accepts ISO and epoch ms, rejects junk', () => {
		expect(parseBound('2026-05-01T00:00:00Z', now)?.getTime()).toBe(Date.UTC(2026, 4, 1));
		expect(parseBound(String(Date.UTC(2026, 4, 1)), now)?.getTime()).toBe(Date.UTC(2026, 4, 1));
		expect(parseBound('yesterday', now)).toBeNull();
	});
});

describe('resolve', () => {
	it('rejects empty or inverted ranges', () => {
		expect(resolve({ from: 'now', to: 'now-1h' }, now)).toBeNull();
		expect(resolve({ from: 'now-1h', to: 'now' }, now)).not.toBeNull();
	});
});

describe('shift', () => {
	it('moves by half the window', () => {
		const r = shift({ from: 'now-24h', to: 'now' }, -1, now);
		expect(r).toEqual({ from: '2026-05-13T00:00:00.000Z', to: '2026-05-14T00:00:00.000Z' });
	});
	it('never moves past now', () => {
		const r = shift({ from: 'now-24h', to: 'now' }, 1, now);
		expect(r.to).toBe(now.toISOString());
		expect(isLive(r)).toBe(false);
	});
});

describe('zoomOut', () => {
	it('keeps a live window live', () => {
		const r = zoomOut({ from: 'now-24h', to: 'now' }, now);
		expect(r.to).toBe('now');
		expect(r.from).toBe('2026-05-12T12:00:00.000Z');
	});
	it('doubles an absolute window around its centre', () => {
		const r = zoomOut({ from: '2026-05-01T00:00:00Z', to: '2026-05-03T00:00:00Z' }, now);
		expect(r).toEqual({ from: '2026-04-30T00:00:00.000Z', to: '2026-05-04T00:00:00.000Z' });
	});
});

describe('describe', () => {
	it('names quick ranges and other relative windows', () => {
		expect(label({ from: 'now-24h', to: 'now' }, now)).toBe('Last 24 hours');
		expect(label({ from: 'now-3d', to: 'now' }, now)).toBe('Last 3 days');
	});
});
