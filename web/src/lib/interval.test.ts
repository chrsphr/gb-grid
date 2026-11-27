import { describe, expect, it } from 'vitest';
import { pickStep } from './interval';

const H = 3_600_000;

describe('pickStep', () => {
	it('uses raw 5-minute buckets for a day on a wide chart', () => {
		expect(pickStep(24 * H, 1000).pg).toBe('5 minutes');
	});
	it('snaps up to the next width on the ladder', () => {
		// 7 days over 1000 px is ~10 min a pixel.
		expect(pickStep(7 * 24 * H, 1000).pg).toBe('15 minutes');
		expect(pickStep(30 * 24 * H, 1000).pg).toBe('1 hour');
	});
	it('reaches the daily tier for a year', () => {
		expect(pickStep(365 * 24 * H, 1000).pg).toBe('12 hours');
		expect(pickStep(365 * 24 * H, 500).pg).toBe('1 day');
	});
	it('tops out at a week', () => {
		expect(pickStep(10 * 365 * 24 * H, 100).pg).toBe('7 days');
	});
});
