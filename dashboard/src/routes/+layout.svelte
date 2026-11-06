<script lang="ts">
	import '../app.css';
	import { page } from '$app/state';
	import { get, session, setToken, startLive } from '#lib/api.svelte.ts';
	import { ago } from '#lib/format.ts';
	import type { Status } from '#lib/types.ts';
	import { onMount } from 'svelte';

	let { children } = $props();
	let status: Status | null = $state(null);
	let tokenInput = $state('');

	onMount(startLive);

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
	{#if status}starlist {status.version} · {status.patterns} patterns{#if status.raw.oldest} · raw lines since {ago(status.raw.oldest)}{/if}{/if}
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
	@media (max-width: 560px) {
		header .inner {
			gap: 10px;
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
