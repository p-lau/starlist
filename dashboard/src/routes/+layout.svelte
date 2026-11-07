<script lang="ts">
	import '../app.css';
	import { page } from '$app/state';
	import { get, session, setService, setToken, startLive } from '#lib/api.svelte.ts';
	import { ago, approx, bytes, num } from '#lib/format.ts';
	import type { Status } from '#lib/types.ts';
	import { onMount } from 'svelte';

	let { children } = $props();
	let status = $state<Status | null>(null);
	let tokenInput = $state('');

	// Many new patterns an hour means lines aren't grouping, and the pattern table grows for good.
	const MANY_NEW_PATTERNS = 1000;
	let health = $derived(status?.health);
	let healthWarn = $derived(
		!!health && (health.hub_dropped > 0 || health.new_patterns_last_hour >= MANY_NEW_PATTERNS)
	);

	onMount(startLive);

	// A link can name the service, like a pattern from a request on a hub.
	$effect(() => {
		const s = page.url.searchParams.get('service');
		if (s && session.services.includes(s)) setService(s);
	});

	$effect(() => {
		void session.tick;
		get<Status>('/status').then((s) => (status = s), () => {});
	});

	const nav = [
		{ href: '/', label: 'Overview' },
		{ href: '/patterns', label: 'Patterns' },
		{ href: '/deploys', label: 'Deploys' }
	];
	const active = (href: string) =>
		href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(href);
</script>

<header>
	<div class="inner">
		<a class="brand" href="/">
			<img src="/favicon.svg" alt="" width="22" height="22" />
			starlist
		</a>
		<nav>
			{#each nav as n (n.href)}
				<a href={n.href} class:active={active(n.href)} aria-current={active(n.href) ? 'page' : undefined}
					>{n.label}</a
				>
			{/each}
		</nav>
		<div class="state">
			{#if session.services.length}
				<select
					class="svc"
					aria-label="Service"
					value={session.service}
					onchange={(e) => setService(e.currentTarget.value)}
				>
					{#each session.services as s (s)}<option value={s}>{s}</option>{/each}
				</select>
			{/if}
			{#if status?.current_release}
				<span class="release" title="Live since {status.current_release.at}">{status.current_release.name}</span>
			{/if}
			<span class="live" class:off={session.offline} title={session.updated ? `Updated ${session.updated.toLocaleTimeString()}` : ''}>
				<i></i>{session.offline ? 'Offline' : 'Live'}
			</span>
		</div>
	</div>
</header>

{#if session.alert}
	<div class="alert" role="status">
		<div class="inner row">
			<strong>Alert</strong>
			<span>{session.alert}</span>
			<button onclick={() => (session.alert = null)}>Dismiss</button>
		</div>
	</div>
{/if}

<main>
	{#if session.needsToken}
		<form
			class="card token"
			onsubmit={(e) => {
				e.preventDefault();
				setToken(tokenInput);
			}}
		>
			<h1>Token needed</h1>
			<p class="muted">
				This starlist listens beyond localhost, so it needs its bearer token: the value of
				<code>--token</code>, <code>STARLIST_TOKEN</code> or the <code>.token</code> file next to its database.
				It is kept in this tab only.
			</p>
			<div class="row">
				<input type="password" bind:value={tokenInput} placeholder="Token" autocomplete="off" aria-label="Token" />
				<button class="primary" type="submit" disabled={!tokenInput.trim()}>Continue</button>
			</div>
		</form>
	{:else}
		{@render children()}
	{/if}
</main>

<footer class="faint">
	{#if status}starlist {status.version} · {status.patterns} patterns{#if status.raw?.oldest}{' · '}raw lines since {ago(status.raw.oldest)}{/if}{/if}
	{#if health}
		<div class="health" class:warn={healthWarn}>
			{#if health.db_bytes != null}<span title="Database on disk, with its write-ahead log">database {bytes(health.db_bytes)}</span>{/if}
			{#if health.memory_bytes != null}<span title="Memory starlist is using now">memory {bytes(health.memory_bytes)}</span>{/if}
			<span title="Log lines read per second, over the last minute">{approx(health.events_per_sec)} lines/s</span>
			<span
				title="Patterns first seen in the last hour. A steady stream of new ones means lines aren't grouping: add --group rules or use --merge-after."
				>{num(health.new_patterns_last_hour)} new patterns/h</span
			>
			{#if health.hub_dropped > 0}<span
					title="Batches dropped because the hub was unreachable for longer than --hub-buffer could hold"
					>{num(health.hub_dropped)} batches dropped</span
				>{/if}
		</div>
	{/if}
</footer>

<style>
	header {
		background: var(--surface);
		border-bottom: 1px solid var(--line);
		position: sticky;
		top: 0;
		z-index: 10;
	}
	.inner {
		max-width: var(--max);
		margin: 0 auto;
		padding: 0 16px;
	}
	header .inner {
		display: flex;
		align-items: center;
		gap: 20px;
		height: 56px;
	}
	.brand {
		display: flex;
		align-items: center;
		gap: 8px;
		font-weight: 700;
		color: var(--text);
		font-size: 1.05rem;
	}
	.brand:hover {
		text-decoration: none;
	}
	nav {
		display: flex;
		gap: 4px;
		overflow-x: auto;
		min-width: 0;
	}
	nav a {
		color: var(--muted);
		padding: 6px 10px;
		border-radius: 8px;
		white-space: nowrap;
	}
	nav a:hover {
		text-decoration: none;
		background: var(--surface-2);
	}
	nav a.active {
		color: var(--text);
		background: var(--accent-soft);
	}
	.state {
		margin-left: auto;
		display: flex;
		align-items: center;
		gap: 10px;
		font-size: 0.82rem;
	}
	.svc {
		max-width: 160px;
		padding: 3px 6px;
		font-size: 0.82rem;
	}
	.release {
		font-family: var(--mono);
		padding: 2px 8px;
		border: 1px solid var(--line-strong);
		border-radius: 999px;
	}
	.live {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		color: var(--muted);
	}
	.live i {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--good);
	}
	.live.off i {
		background: var(--error);
	}
	.alert {
		background: var(--error-soft);
		border-bottom: 1px solid var(--error);
		padding: 8px 0;
	}
	.alert span {
		flex: 1;
	}
	main {
		max-width: var(--max);
		margin: 0 auto;
		padding: 20px 16px 40px;
	}
	.token {
		max-width: 520px;
		margin: 40px auto;
		display: grid;
		gap: 12px;
	}
	.token input {
		flex: 1;
	}
	footer {
		max-width: var(--max);
		margin: 0 auto;
		padding: 0 16px 24px;
		font-size: 0.8rem;
	}
	.health {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 14px;
		margin-top: 4px;
	}
	.health span {
		cursor: help;
	}
	.health.warn {
		color: var(--warn);
	}
	@media (max-width: 560px) {
		header .inner {
			gap: 10px;
		}
		/* On a hub the service picker needs the room: give it its own row. */
		header .inner:has(.svc) {
			flex-wrap: wrap;
			height: auto;
			padding-top: 10px;
			padding-bottom: 10px;
			row-gap: 6px;
		}
		header .inner:has(.svc) .state {
			flex: 1 0 100%;
			margin-left: 0;
		}
		.brand {
			font-size: 0;
			gap: 0;
		}
		.release {
			display: none;
		}
	}
</style>
