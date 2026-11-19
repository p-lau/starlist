// The MCP tools, with an example call and the answer starlist gave for it. The
// answers come from the demo (`cargo run --example demo_log`): a checkout
// service with a bad deploy (v1.4.2) and a fix (v1.4.3). Long samples are cut.

export type Arg = { name: string; text: string; required?: boolean };

export type Tool = {
	name: string;
	group: 'find' | 'dig' | 'fix' | 'tidy';
	summary: string;
	when: string;
	args: Arg[];
	call: Record<string, unknown>;
	answer: unknown;
	notes?: string[];
	/** Not in the latest release yet. */
	unreleased?: boolean;
};

const trace =
	'ERROR Unhandled error in POST /api/checkout cart=1942\nTraceback (most recent call last):\n  File "/app/checkout.py", line 88, in place_order\n    total = price_cart(cart)\n  File "/app/pricing.py", line 41, in price_cart\n    currency = cart.meta[\'currency\']\nKeyError: \'currency\'';
const keyError = 'ERROR Unhandled error in POST /api/checkout cart=<NUM> | KeyError';
const timeout = 'ERROR upstream payments timeout after <NUM> (attempt <NUM>)';

export const groups: Record<Tool['group'], string> = {
	find: 'Find what is wrong',
	dig: 'Dig into one problem',
	fix: 'Check a fix and track releases',
	tidy: 'Keep patterns tidy'
};

export const tools: Tool[] = [
	{
		name: 'diagnose',
		group: 'find',
		summary:
			'The likely causes of what is wrong right now, ranked, in one call. Each cause says why it stands out (new, spiking, slowly rising), the deploy it started with, its code location and sample lines.',
		when: 'Start here when you don’t know what is wrong yet: "checkout is broken", "errors went up".',
		args: [
			{ name: 'window', text: 'Recent period, like "15m" or "1h". Default: since the latest deploy if it was in the last 6 hours, else "1h".' },
			{ name: 'min_level', text: 'DEBUG, INFO, WARN, ERROR or FATAL.' },
			{ name: 'limit', text: 'Causes to explain. Default 3.' }
		],
		call: { window: '1h', limit: 1 },
		answer: {
			window: 'last 60m',
			verdict:
				'Most likely a new ERROR pattern at /app/pricing.py:41 in price_cart (last 60m). 2 new, 1 returning, 0 spiking, 0 gone quiet. 655 events across 8 patterns in the window.',
			causes: [
				{
					pattern_id: 'p6c8d499767',
					kind: 'new',
					level: 'ERROR',
					template: keyError,
					why: 'New: 27 in 60m (27/h), first seen 50m ago.',
					started_with: 'v1.4.2',
					location: '/app/pricing.py:41 in price_cart',
					samples: [trace]
				}
			],
			also_changed: [
				{ pattern_id: 'p05f0d09d52', kind: 'new', template: timeout },
				{ pattern_id: 'p0632225b80', kind: 'returning', template: 'INFO Starting checkout-service version <ID> on port <NUM>' }
			],
			next: 'Read the code at the location. get_pattern or get_request for more; after a fix, verify_fix.'
		},
		notes: [
			'With nothing new in the window, it says so and lists the busiest warnings and errors instead.',
			'The top cause comes with the lines of one request that hit it, when the logs carry request IDs.'
		]
	},
	{
		name: 'whats_new',
		group: 'find',
		summary:
			'Every pattern that appeared for the first time, came back, spiked, is slowly rising, or went quiet in a window compared with a baseline. Errors first.',
		when: 'After a deploy, during an incident, or as a morning check: "what changed in the last hour?"',
		args: [
			{ name: 'window', text: 'Recent period. Default: since a deploy in the last 6 hours, else "1h".' },
			{ name: 'baseline', text: 'Period before it taken as normal. Default "24h".' },
			{ name: 'min_level', text: 'Leave out lower levels.' },
			{ name: 'limit', text: 'Default 10. Pass cursor back to page.' }
		],
		call: { window: '1h', min_level: 'WARN' },
		answer: {
			window: 'last 60m vs prior 24h',
			summary: '2 new, 0 returning, 0 spiking, 0 gone quiet. 654 events across 8 patterns in the window.',
			traffic: { note: 'Overall log volume is 5.3x its baseline.' },
			current_release: 'v1.4.3',
			markers_in_window: [
				{ name: 'v1.4.3', at: '15m ago' },
				{ name: 'v1.4.2', at: '50m ago' }
			],
			findings: [
				{
					pattern_id: 'p6c8d499767',
					level: 'ERROR',
					template: keyError,
					first_release: 'v1.4.2',
					location: '/app/pricing.py:41 in price_cart',
					last_seen: '16m ago',
					kind: 'new',
					count_in_window: 27,
					per_hour: '27 vs 0 baseline',
					sample: 'ERROR Unhandled error in POST /api/checkout cart=1942\nTraceback (most recent call last): …\nKeyError: \'currency\''
				},
				{
					pattern_id: 'p05f0d09d52',
					level: 'ERROR',
					template: timeout,
					first_release: 'v1.4.2',
					last_seen: '16m ago',
					kind: 'new',
					count_in_window: 9,
					per_hour: '9 vs 0 baseline'
				}
			]
		},
		notes: [
			'Spikes are judged against overall traffic, so a busy hour alone is not flagged.',
			'A spike that matches the same time on earlier days (a nightly job) is labelled usual and listed last instead of hidden.',
			'first_release answers "which deploy started this?" without a second call.'
		]
	},
	{
		name: 'list_patterns',
		group: 'find',
		summary: 'Search the patterns by text or level, with counts. Paged with next_cursor.',
		when: 'When you know roughly what you are looking for: "any errors about payments today?"',
		args: [
			{ name: 'contains', text: 'Text the template contains (any case).' },
			{ name: 'min_level', text: 'Leave out lower levels.' },
			{ name: 'since', text: 'Default "24h".' },
			{ name: 'limit', text: 'Default 20.' }
		],
		call: { min_level: 'WARN', limit: 3 },
		answer: {
			since: '24h ago',
			patterns: [
				{ pattern_id: 'p6c8d499767', level: 'ERROR', template: keyError, first_release: 'v1.4.2', location: '/app/pricing.py:41 in price_cart', last_seen: '16m ago', count: 27 },
				{ pattern_id: 'p05f0d09d52', level: 'ERROR', template: timeout, first_release: 'v1.4.2', last_seen: '16m ago', count: 9 },
				{ pattern_id: 'p6f3fac74d0', level: 'WARN', template: 'WARN slow query on carts took <NUM>', first_release: 'v1.4.1', last_seen: '16m ago', count: 231 }
			]
		}
	},
	{
		name: 'get_pattern',
		group: 'dig',
		summary:
			'Everything about one pattern: counts, a 24-hour histogram, the markers around it, its state, every code location it was raised from, and redacted samples with their stack traces.',
		when: 'Once a finding looks like the culprit and you want its shape over time.',
		args: [
			{ name: 'pattern_id', text: 'From any other tool.', required: true },
			{ name: 'samples', text: '1 to 5. Default 1 (brief) or 3 (full).' }
		],
		call: { pattern_id: 'p6c8d499767' },
		answer: {
			pattern_id: 'p6c8d499767',
			level: 'ERROR',
			template: keyError,
			first_seen: '2026-11-12T03:40:46Z (50m ago)',
			first_release: 'v1.4.2',
			location: '/app/pricing.py:41 in price_cart',
			last_seen: '2026-11-12T04:14:32Z (16m ago)',
			exception: 'KeyError',
			state: 'open',
			total_seen: 27,
			count_last_hour: 27,
			count_last_24h: 27,
			hourly_last_24h: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 27],
			markers_last_24h: [
				{ name: 'v1.4.3', at: '15m ago' },
				{ name: 'v1.4.2', at: '50m ago' }
			],
			samples: [{ at: '16m ago', text: trace }]
		},
		notes: [
			'Stack traces are trimmed to your app’s own frames plus the exception line in brief answers; library frames are counted, not shown.',
			'If the app runs from another path than your checkout (say /app in a container), start starlist with --source-root /app so locations match the repository.'
		]
	},
	{
		name: 'get_context',
		group: 'dig',
		summary:
			'The redacted lines just before and after an occurrence of a pattern, or around any time, so the agent sees what led up to an error without opening the log file.',
		when: 'When the error line alone doesn’t explain itself.',
		args: [
			{ name: 'pattern_id', text: 'Center on its latest occurrence (before at, if given).' },
			{ name: 'at', text: 'Center on this time instead.' },
			{ name: 'before / after', text: 'Lines on each side, 0 to 200. Default 20.' }
		],
		call: { pattern_id: 'p6c8d499767', before: 3, after: 2 },
		answer: {
			anchor: { pattern_id: 'p6c8d499767', at: '16m ago' },
			note: 'Offsets are from the anchor line, marked >>.',
			lines: [
				'-10s INFO GET /api/cart/6588 200 in 58ms user=<EMAIL>',
				'-6s INFO GET /api/cart/1797 200 in 70ms user=<EMAIL>',
				'-4s INFO POST /api/checkout 201 in 81ms order=ord_e46a6a',
				'>> 0s ERROR Unhandled error in POST /api/checkout cart=1942\nTraceback (most recent call last): …\nKeyError: \'currency\'',
				'+4s INFO GET /api/cart/6218 200 in 32ms user=<EMAIL>',
				'+8s INFO GET /api/cart/6365 200 in 77ms user=<EMAIL>'
			]
		},
		notes: ['Reads the raw-line buffer (--raw-size, 20 MB by default). With --no-raw it is unavailable.']
	},
	{
		name: 'get_request',
		group: 'dig',
		summary:
			'Every event of one request, in order: the path that led to an error. Findings carry an example_request ID to start from; a W3C traceparent works too.',
		when: 'When an error depends on what happened earlier in the same request.',
		args: [{ name: 'request_id', text: 'Like a finding’s example_request.', required: true }],
		call: { request_id: 'req-7f3a' },
		answer: {
			request_id: 'req-7f3a',
			started: '11m ago',
			duration: '3s',
			events: 5,
			patterns: 5,
			lines: [
				'0s INFO request_id=req-7f3a POST /api/checkout started',
				'0s INFO request_id=req-7f3a cart loaded items=3',
				'0s WARN request_id=req-7f3a payments slow, retrying attempt=2',
				'+3s ERROR request_id=req-7f3a upstream payments timeout after 3000ms (attempt 2)',
				'+3s INFO request_id=req-7f3a POST /api/checkout 502 in 3512ms'
			]
		},
		notes: [
			'Request IDs are read from a traceparent and fields like trace_id, request_id, x-request-id and correlation_id, in JSON or key=value text. Add your own with --request-key order_ref.',
			'On a hub, leaving out service follows one request through every service in time order.'
		]
	},
	{
		name: 'verify_fix',
		group: 'fix',
		summary:
			'Did this error stop after the fix? Compares the pattern’s rate before and after a marker, deploy or time, and gives a verdict: fixed, reduced, still_occurring, worse, too_early (with minutes to wait), appeared_after or not_active_before.',
		when: 'After shipping a fix, instead of watching the logs and guessing.',
		args: [
			{ name: 'pattern_id', text: 'The pattern that should have stopped.', required: true },
			{ name: 'since', text: 'When the fix went live: a marker, an ISO time or "20m".', required: true },
			{ name: 'before', text: 'Period before it to compare with. Default: as long as since the split, at least 1h.' }
		],
		call: { pattern_id: 'p6c8d499767', since: 'v1.4.3' },
		answer: {
			pattern_id: 'p6c8d499767',
			template: keyError,
			split: { at: '15m ago', marker: 'v1.4.3' },
			last_seen: '16m ago',
			before: '27 in 35m (46.2/h)',
			after: '0 in 15m (0/h)',
			confidence: 'high',
			caveat: 'Overall log volume dropped to 12% of its earlier rate after the split; low traffic can hide errors.',
			verdict: 'fixed',
			explanation: 'At the old rate about 12 would have occurred since the split; none did.'
		},
		notes: [
			'Pattern IDs are a hash of the template, so the ID from before the fix still works after a restart or on another host.',
			'An error that keeps to certain hours isn’t called fixed until those hours have come round.'
		]
	},
	{
		name: 'compare_deploy',
		group: 'fix',
		summary:
			'Everything that changed around a deploy (the latest one by default): new patterns, which are likely regressions, then what increased, stopped and decreased.',
		when: 'Right after a release goes out, or when deciding whether to roll back.',
		args: [
			{ name: 'at', text: 'Marker, ISO time or "30m". Default: the latest deploy.' },
			{ name: 'before', text: 'Period before the split to compare with.' },
			{ name: 'limit', text: 'Default 10. Paged with cursor.' }
		],
		call: { at: 'v1.4.2', limit: 3 },
		answer: {
			split: { at: '50m ago', marker: 'v1.4.2' },
			compared: '60m before vs 50m after',
			summary: '2 new since the split, 1 increased, 0 stopped, 0 decreased.',
			traffic_per_hour: '118 before, 763.4 after',
			findings: [
				{ pattern_id: 'p6c8d499767', level: 'ERROR', template: keyError, location: '/app/pricing.py:41 in price_cart', kind: 'new_after', before: '0 in 60m (0/h)', after: '27 in 50m (32.3/h)' },
				{ pattern_id: 'p05f0d09d52', level: 'ERROR', template: timeout, kind: 'new_after', before: '0 in 60m (0/h)', after: '9 in 50m (10.8/h)' },
				{ pattern_id: 'pdf468c3cb1', level: 'INFO', template: 'INFO auth refreshed for <EMAIL> Authorization: <REDACTED> <TOKEN>', kind: 'increased', before: '1 in 60m (1/h)', after: '15 in 50m (17.9/h)' }
			]
		}
	},
	{
		name: 'get_release',
		group: 'fix',
		summary:
			'A release’s report card: the patterns it introduced, the ones that regressed and the ones it resolved, and its error rate against the previous release.',
		when: 'For a release review, or to see whether the new version is healthier than the last.',
		args: [{ name: 'name', text: 'Release name. Default: the live release.' }],
		call: { name: 'v1.4.2' },
		answer: {
			release: 'v1.4.2',
			live: '50m ago to 15m ago',
			events_per_hour: 1042.5,
			errors_per_hour: 61.6,
			vs_previous: '61.6/h errors; none in v1.4.1',
			new: {
				total: 2,
				patterns: [
					{ pattern_id: 'p6c8d499767', level: 'ERROR', template: keyError, location: '/app/pricing.py:41 in price_cart' },
					{ pattern_id: 'p05f0d09d52', level: 'ERROR', template: timeout }
				]
			},
			regressed: { total: 0 },
			resolved: { total: 0 }
		}
	},
	{
		name: 'add_marker',
		group: 'fix',
		summary:
			'Name a point in time, like a fix going live, so other tools can use it ("since": "fix-currency"). With deploy: true it records a release.',
		when: 'When the agent ships a fix itself, or a deploy doesn’t log its version.',
		args: [
			{ name: 'name', text: 'Like "fix-currency" or "v1.4.3".', required: true },
			{ name: 'note', text: 'Free text.' },
			{ name: 'at', text: 'If not now: an ISO time or "5m".' },
			{ name: 'deploy', text: 'true for a release going live.' }
		],
		call: { name: 'fix-currency', note: 'default currency when cart.meta is empty' },
		answer: { marker: 'fix-currency', kind: 'manual', at: '2026-11-12T04:30:49Z (0s ago)' },
		notes: [
			'Deploys are also detected from lines like "Starting api version 1.4.2", from STARLIST_VERSION at startup, from `starlist mark`, and from POST /api/v1/markers.'
		]
	},
	{
		name: 'list_markers',
		group: 'fix',
		summary: 'Deploys and other markers, newest first.',
		when: 'To find the name of the deploy to compare against.',
		args: [{ name: 'limit', text: 'Default 20.' }],
		call: { limit: 3 },
		answer: {
			markers: [
				{ name: 'fix-currency', kind: 'manual', at: '2026-11-12T04:30:49Z (0s ago)', note: 'default currency when cart.meta is empty' },
				{ name: 'v1.4.3', kind: 'deploy', at: '2026-11-12T04:15:40Z (15m ago)' },
				{ name: 'v1.4.2', kind: 'deploy', at: '2026-11-12T03:40:37Z (50m ago)' }
			]
		}
	},
	{
		name: 'set_pattern_state',
		group: 'tidy',
		summary:
			'Resolve a pattern after a verified fix (it comes back as regressed if it occurs again), resolve it from the next release on, ignore it for a while or for a number of events, or reopen it. timing marks it as expected at set times (usual) or never usual (always_alert).',
		when: 'After verify_fix says fixed, or to quiet a known, harmless pattern.',
		args: [
			{ name: 'pattern_id', text: 'The pattern.', required: true },
			{ name: 'state', text: 'resolved, resolved_next, ignored or open.' },
			{ name: 'timing', text: 'usual, always_alert or auto (the default).' },
			{ name: 'ignore_for / ignore_count', text: 'With ignored: "24h", or a number of events.' },
			{ name: 'note', text: 'Why.' }
		],
		call: { pattern_id: 'p6c8d499767', state: 'resolved', note: 'fixed in v1.4.3' },
		answer: { pattern_id: 'p6c8d499767', template: keyError, state: 'resolved', timing: 'auto', note: 'Resolved 0s ago.' }
	},
	{
		name: 'merge_patterns',
		group: 'tidy',
		summary:
			'Merge patterns that are really one issue. The first ID is kept, and the others keep resolving to it, so IDs an agent already holds still work.',
		when: 'When the same error shows up as several patterns because a word in it varies.',
		args: [{ name: 'pattern_ids', text: 'The first is kept.', required: true }],
		call: { pattern_ids: ['p82c5f3b07d', 'pcb8b4537d1'] },
		answer: {
			pattern_id: 'p82c5f3b07d',
			template: 'WARN login failed for user alice: bad password',
			merged: 1,
			total_seen: 6,
			note: 'The other IDs now resolve to this one.'
		},
		notes: [
			'starlist already merges patterns that differ in one word once that word has taken 5 values (--merge-after N, 0 turns it off).'
		]
	},
	{
		name: 'split_pattern',
		unreleased: true,
		group: 'tidy',
		summary: 'Undo a merge: patterns merged by mistake count on their own again and aren’t merged automatically again.',
		when: 'When an automatic merge lumped together things you want to track apart.',
		args: [
			{ name: 'pattern_id', text: 'The merged pattern, or an ID merged into it.', required: true },
			{ name: 'pattern_ids', text: 'Merged-in IDs to split out. Default: all of them.' }
		],
		call: { pattern_id: 'p82c5f3b07d' },
		answer: {
			pattern_id: 'p82c5f3b07d',
			template: 'WARN login failed for user alice: bad password',
			total_seen: 6,
			still_merged: 0,
			split: [
				{ pattern_id: 'pcb8b4537d1', template: 'WARN login failed for user bob: bad password' },
				{ pattern_id: 'p5f4e47140a', template: 'WARN login failed for user carol: bad password' }
			],
			note: 'The split IDs count on their own from now on and won’t be merged automatically again. What was counted while merged stays with this pattern.'
		}
	}
];

export const json = (v: unknown) => JSON.stringify(v, null, 2);
