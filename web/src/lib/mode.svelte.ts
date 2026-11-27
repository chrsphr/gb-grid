import { mode } from 'mode-watcher';
import type { Mode } from './palette';

/** The resolved colour mode for charts (ModeWatcher sets it before first paint). */
export function currentMode(): Mode {
	return mode.current === 'light' ? 'light' : 'dark';
}
