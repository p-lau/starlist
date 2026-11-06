// The shapes the starlist JSON API returns (docs/api.md).

export type Level = 'DEBUG' | 'INFO' | 'WARN' | 'ERROR' | 'FATAL';

export interface PatternState {
	state: 'open' | 'resolved' | 'resolved_next' | 'ignored' | 'regressed';
	since: string | null;
	note: string | null;
	until?: string | null;
	remaining?: number | null;
	resolved_at?: string;
	regressed_at?: string;
	regressed_in?: string | null;
}

export interface Pattern {
	id: string;
	level: Level;
	template: string;
	exception: string | null;
	location: string | null;
	first_seen: string;
	last_seen: string;
	first_release: string | null;
	total: number;
	example_request: string | null;
	state: PatternState;
}

export interface Marker {
	name: string;
	kind: 'deploy' | 'manual' | string;
	at: string;
	note: string | null;
}

export interface Status {
	version: string;
	now: string;
	service: string | null;
	env: string | null;
	current_release: { name: string; at: string } | null;
	patterns: number;
	last_hour: { events: number; errors: number };
	raw: { events: number; oldest: string | null };
	alerts: boolean;
}

export interface Timeline {
	from: string;
	to: string;
	step_seconds: number;
	series: { level: Level; pattern_id?: string; counts: number[] }[];
	markers: Marker[];
}

export interface Sample {
	at: string;
	text: string;
}

export interface Finding {
	kind: 'new' | 'regressed' | 'returning' | 'spike' | 'gone_quiet';
	note: string | null;
	count_window: number;
	count_baseline: number;
	per_hour_window: number;
	per_hour_baseline: number;
	pattern: Pattern;
	sample: Sample | null;
}

export interface WhatsNew {
	window: { from: string; to: string };
	baseline: { from: string; to: string };
	summary: string;
	counts: { new: number; regressed: number; returning: number; spike: number; gone_quiet: number };
	traffic: { events_window: number; events_baseline: number; ratio: number };
	current_release: string | null;
	markers: Marker[];
	total: number;
	findings: Finding[];
}

export interface PatternList {
	since: string;
	trend_step_seconds: number | null;
	total: number;
	offset: number;
	patterns: (Pattern & { count: number; trend?: number[] })[];
}

export interface PatternDetail extends Pattern {
	locations: { location: string; count: number }[];
	count_last_hour: number;
	count_last_24h: number;
	samples: Sample[];
	merged_from?: string;
}

export interface Verify {
	pattern: Pattern;
	verdict:
		| 'fixed'
		| 'reduced'
		| 'still_occurring'
		| 'worse'
		| 'too_early'
		| 'appeared_after'
		| 'not_active_before';
	explanation: string;
	split: { at: string; marker: string | null };
	before: { from: string; to: string; count: number; per_hour: number } | null;
	after: { from: string; to: string; count: number; per_hour: number };
	expected_after: number | null;
	confidence: string | null;
	wait_minutes: number | null;
	caveat: string | null;
}

export interface Change {
	kind: 'new_after' | 'increased' | 'stopped' | 'decreased';
	before: { count: number; per_hour: number };
	after: { count: number; per_hour: number };
	pattern: Pattern;
}

export interface Compare {
	split: { at: string; marker: string | null };
	before: { from: string; to: string; events: number };
	after: { from: string; to: string; events: number };
	summary: string;
	counts: { new_after: number; increased: number; stopped: number; decreased: number };
	total: number;
	changes: Change[];
}

export interface Line {
	id: number;
	at: string;
	pattern_id: string;
	text: string;
	offset_ms?: number;
}

export interface Context {
	anchor: Line;
	lines: Line[];
}

export interface RequestTrace {
	request_id: string;
	service: string | null;
	started: string;
	duration_ms: number;
	events: number;
	truncated: boolean;
	lines: Line[];
}

export interface Release {
	name: string;
	at: string;
	until: string | null;
	current: boolean;
	previous: string | null;
	env: string | null;
	events_per_hour: number;
	errors_per_hour: number;
	previous_errors_per_hour: number | null;
	new: Pattern[];
	regressed: (Pattern & { regressed_at: string })[];
	resolved: (Pattern & { resolved_at: string })[];
}
