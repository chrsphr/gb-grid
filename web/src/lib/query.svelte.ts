// Small helpers shared by pages: an auto-refresh interval from the URL, and a
// plot width that drives bucket size.
import type { TimeRange } from './time';
import { isLive } from './time';

const REFRESH_MS: Record<string, number> = { '30s': 30_000, '1m': 60_000, '5m': 300_000 };

/** Refetch interval for a query: only live ("…to now") windows refresh. */
export function refetchEvery(range: TimeRange, refresh: string): number | false {
	return isLive(range) ? (REFRESH_MS[refresh] ?? false) : false;
}
