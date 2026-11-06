// Talks to the starlist API on the same origin. With a bearer token (starlist
// listening beyond localhost) the user enters it once; it is kept for this tab.

const TOKEN_KEY = 'starlist-token';

function readToken(): string {
	try {
		return sessionStorage.getItem(TOKEN_KEY) ?? '';
	} catch {
		return '';
	}
}

export const session = $state({
	token: readToken(),
	/** The server answered 401: ask for a token. */
	needsToken: false,
	/** Bumped on every refresh; pages reload their data when it changes. */
	tick: 0,
	/** Time of the last successful refresh. */
	updated: null as Date | null,
	/** The API can't be reached. */
	offline: false,
	/** The latest alert's summary, shown until dismissed. */
	alert: null as string | null
});

export function setToken(t: string) {
	session.token = t.trim();
	try {
		sessionStorage.setItem(TOKEN_KEY, session.token);
	} catch {
		// Private windows may refuse storage; the token still works for this page.
	}
	session.needsToken = false;
	refresh();
}

export class ApiError extends Error {
	constructor(
		public status: number,
		message: string
	) {
		super(message);
	}
}

type Params = Record<string, string | number | boolean | null | undefined>;

function url(path: string, params?: Params): string {
	const q = new URLSearchParams();
	for (const [k, v] of Object.entries(params ?? {})) {
		if (v !== undefined && v !== null && v !== '') q.set(k, String(v));
	}
	const s = q.toString();
	return `/api/v1${path}${s ? `?${s}` : ''}`;
}

async function call<T>(path: string, init: RequestInit, params?: Params): Promise<T> {
	const headers = new Headers(init.headers);
	if (session.token) headers.set('authorization', `Bearer ${session.token}`);
	let res: Response;
	try {
		res = await fetch(url(path, params), { ...init, headers });
	} catch {
		session.offline = true;
		throw new ApiError(0, 'Could not reach starlist.');
	}
	session.offline = false;
	if (res.status === 401) {
		session.needsToken = true;
		throw new ApiError(401, 'This server needs a token.');
	}
	const body = await res.json().catch(() => ({}));
	if (!res.ok) throw new ApiError(res.status, body.error ?? res.statusText);
	session.updated = new Date();
	return body as T;
}

export function get<T>(path: string, params?: Params): Promise<T> {
	return call<T>(path, { method: 'GET' }, params);
}

export function post<T>(path: string, body: unknown): Promise<T> {
	return call<T>(path, {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(body)
	});
}

export function refresh() {
	session.tick++;
}

let started = false;

/**
 * Refresh every 10 seconds while the tab is visible, and at once when an
 * alert arrives on the event stream.
 */
export function startLive() {
	if (started) return;
	started = true;
	setInterval(() => {
		if (document.visibilityState === 'visible' && !session.needsToken) refresh();
	}, 10_000);
	document.addEventListener('visibilitychange', () => {
		if (document.visibilityState === 'visible') refresh();
	});
	listenForAlerts();
}

/** Read the server-sent alert stream with fetch, so the token header can be sent. */
async function listenForAlerts() {
	for (let delay = 2000; ; delay = Math.min(delay * 2, 60_000)) {
		try {
			const headers = new Headers({ accept: 'text/event-stream' });
			if (session.token) headers.set('authorization', `Bearer ${session.token}`);
			const res = await fetch('/api/v1/alerts/stream', { headers });
			// Alerts off (404) or not allowed: stop trying.
			if (res.status === 404 || res.status === 403) return;
			if (res.ok && res.body) {
				delay = 2000;
				const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
				let buf = '';
				for (;;) {
					const { value, done } = await reader.read();
					if (done) break;
					buf += value;
					let end;
					while ((end = buf.indexOf('\n\n')) >= 0) {
						const block = buf.slice(0, end);
						buf = buf.slice(end + 2);
						const data = block
							.split('\n')
							.filter((l) => l.startsWith('data:'))
							.map((l) => l.slice(5).trim())
							.join('\n');
						if (block.includes('event: alert') && data) {
							try {
								session.alert = JSON.parse(data).summary ?? 'New errors.';
							} catch {
								session.alert = 'New errors.';
							}
							refresh();
						}
					}
				}
			}
		} catch {
			// Network hiccup: retry below.
		}
		await new Promise((r) => setTimeout(r, delay));
	}
}
