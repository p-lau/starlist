import type { Level } from './types.ts';

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto', style: 'short' });

/** "3 min ago", "in 2 hr". */
export function ago(iso: string | null | undefined, now = Date.now()): string {
	if (!iso) return '';
	const s = (new Date(iso).getTime() - now) / 1000;
	const a = Math.abs(s);
	if (a < 45) return 'just now';
	if (a < 3600) return rtf.format(Math.round(s / 60), 'minute');
	if (a < 86400 * 2) return rtf.format(Math.round(s / 3600), 'hour');
	return rtf.format(Math.round(s / 86400), 'day');
}

/** Local date and time, for tooltips and detail views. */
export function when(iso: string | null | undefined): string {
	if (!iso) return '';
	return new Date(iso).toLocaleString(undefined, {
		dateStyle: 'medium',
		timeStyle: 'medium'
	});
}

/** Short axis label for a time, depending on the step. */
export function tick(d: Date, stepSeconds: number): string {
	if (stepSeconds >= 86400) return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
	return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
}

const nf = new Intl.NumberFormat();
export function num(n: number | null | undefined): string {
	return n === null || n === undefined ? '' : nf.format(n);
}

export function rate(perHour: number): string {
	if (perHour >= 100) return `${nf.format(Math.round(perHour))}/h`;
	return `${perHour.toFixed(1).replace(/\.0$/, '')}/h`;
}

/** Errors, warnings, and everything else: the three groups charts use. */
export type Group = 'error' | 'warn' | 'info';

export function group(level: Level | string): Group {
	if (level === 'ERROR' || level === 'FATAL') return 'error';
	if (level === 'WARN') return 'warn';
	return 'info';
}

export const GROUP_LABEL: Record<Group, string> = {
	error: 'Errors',
	warn: 'Warnings',
	info: 'Info and debug'
};

export function duration(ms: number): string {
	if (ms < 1000) return `${Math.round(ms)} ms`;
	if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
	return `${Math.round(ms / 60_000)} min`;
}

const sf = new Intl.NumberFormat(undefined, { maximumSignificantDigits: 3 });

/** A number to three significant digits, for estimates like percentiles. */
export function approx(n: number | null | undefined): string {
	return n === null || n === undefined ? '' : sf.format(n);
}
