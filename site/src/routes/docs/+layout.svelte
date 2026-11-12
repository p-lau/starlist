<script lang="ts">
	import { page } from '$app/state';
	import { afterNavigate } from '$app/navigation';
	import Starfield from '#lib/Starfield.svelte';
	import Logo from '#lib/Logo.svelte';
	import { docsNav } from '#lib/docs/nav.ts';
	import '#lib/docs/docs.css';

	let { children } = $props();
	const repo = 'https://github.com/p-lau/starlist';

	let active = $state('');
	let menu: HTMLDetailsElement | undefined = $state();

	const current = $derived(docsNav.find((p) => p.href === page.url.pathname) ?? docsNav[0]);

	// Light up the sidebar entry of the section being read.
	let io: IntersectionObserver | undefined;
	afterNavigate(() => {
		io?.disconnect();
		const seen = new Map<string, boolean>();
		io = new IntersectionObserver(
			(entries) => {
				for (const e of entries) seen.set(e.target.id, e.isIntersecting);
				const first = current.sections.find((s) => seen.get(s.id));
				if (first) active = first.id;
			},
			{ rootMargin: '-90px 0px -55% 0px' }
		);
		for (const s of current.sections) {
			const el = document.getElementById(s.id);
			if (el) io.observe(el);
		}
		active = location.hash.slice(1) || current.sections[0]?.id;
		if (menu) menu.open = false;
	});
	$effect(() => () => io?.disconnect());
</script>

<Starfield />

<div class="docs">
	<header class="nav">
		<a class="brand" href="/" aria-label="Starlist home"><Logo /> Starlist</a>
		<span class="crumb">Docs</span>
		<nav aria-label="Docs">
			{#each docsNav as p}
				<a href={p.href} aria-current={p.href === current.href ? 'page' : undefined}>{p.title}</a>
			{/each}
		</nav>
		<a class="gh" href={repo} rel="noopener">GitHub</a>
	</header>

	<div class="frame">
		<aside>
			<details class="menu" bind:this={menu}>
				<summary>On this page</summary>
				{@render toc()}
			</details>
			<div class="side">{@render toc()}</div>
		</aside>

		<div class="content">
			{@render children()}
			<footer>
				<span>Starlist · MIT licensed</span>
				<span class="links">
					<a href="{repo}/tree/main/docs" rel="noopener">Reference docs</a>
					<a href="{repo}/issues" rel="noopener">Report an issue</a>
					<a href="https://crates.io/crates/starlist" rel="noopener">crates.io</a>
				</span>
			</footer>
		</div>
	</div>
</div>

{#snippet toc()}
	{#each docsNav as p}
		<p class="group">{p.title}</p>
		<ul>
			{#each p.sections as s}
				{@const here = p.href === current.href}
				<li>
					<a href="{here ? '' : p.href}#{s.id}" class:on={here && active === s.id}>{s.title}</a>
				</li>
			{/each}
		</ul>
	{/each}
{/snippet}

<style>
	.docs {
		position: relative;
		z-index: 1;
		overflow-x: clip;
	}
	.nav {
		position: sticky;
		top: 10px;
		z-index: 10;
		display: flex;
		align-items: center;
		gap: 18px;
		max-width: 1240px;
		margin: 10px auto 0;
		padding: 10px 14px 10px 22px;
		border-radius: 999px;
		border: 1px solid var(--line);
		background: #090b1a;
		box-shadow: 0 10px 30px -18px rgba(0, 0, 0, 0.9);
	}
	.brand {
		display: flex;
		align-items: center;
		gap: 10px;
		font: 700 1.15rem var(--font-display);
		text-decoration: none;
		letter-spacing: -0.02em;
	}
	.crumb {
		font: 500 0.75rem var(--font-mono);
		color: var(--sat);
		letter-spacing: 0.12em;
		text-transform: uppercase;
		padding-left: 16px;
		border-left: 1px solid var(--line-strong);
	}
	nav {
		display: flex;
		gap: 22px;
		margin-left: auto;
	}
	nav a {
		color: var(--muted);
		text-decoration: none;
		font-size: 0.92rem;
	}
	nav a:hover,
	nav a[aria-current='page'] {
		color: var(--text);
	}
	nav a[aria-current='page'] {
		text-decoration: underline;
		text-decoration-color: var(--star);
		text-underline-offset: 6px;
	}
	.gh {
		padding: 7px 14px;
		border-radius: 999px;
		border: 1px solid var(--line-strong);
		text-decoration: none;
		font-size: 0.88rem;
		background: rgba(255, 255, 255, 0.03);
	}
	.gh:hover {
		border-color: rgba(159, 231, 255, 0.5);
	}

	.frame {
		display: grid;
		grid-template-columns: 230px minmax(0, 1fr);
		gap: 56px;
		max-width: 1240px;
		margin: 0 auto;
		padding: 0 24px;
	}
	aside {
		position: relative;
	}
	.side {
		position: sticky;
		top: 90px;
		max-height: calc(100vh - 110px);
		overflow-y: auto;
		padding: 40px 4px 40px 0;
		scrollbar-width: thin;
	}
	.menu {
		display: none;
	}
	.group {
		margin: 22px 0 8px;
		font: 600 0.72rem var(--font-mono);
		letter-spacing: 0.14em;
		text-transform: uppercase;
		color: var(--faint);
	}
	.group:first-child {
		margin-top: 0;
	}
	ul {
		list-style: none;
		margin: 0;
		padding: 0 0 0 2px;
		border-left: 1px solid var(--line);
	}
	li a {
		display: block;
		margin-left: -3px;
		padding: 5px 0 5px 14px;
		border-left: 2px solid transparent;
		font-size: 0.89rem;
		color: var(--muted);
		text-decoration: none;
		line-height: 1.35;
	}
	li a:hover {
		color: var(--text);
	}
	li a.on {
		color: var(--star);
		border-left-color: var(--star);
	}

	.content {
		min-width: 0;
		max-width: 800px;
		padding-top: 40px;
	}
	footer {
		display: flex;
		flex-wrap: wrap;
		justify-content: space-between;
		gap: 12px;
		margin-top: 80px;
		padding: 28px 0 40px;
		border-top: 1px solid var(--line);
		color: var(--faint);
		font-size: 0.88rem;
	}
	footer a {
		color: var(--muted);
	}
	.links {
		display: flex;
		flex-wrap: wrap;
		gap: 18px;
	}

	@media (max-width: 900px) {
		.frame {
			grid-template-columns: minmax(0, 1fr);
			gap: 0;
		}
		.side {
			display: none;
		}
		.menu {
			display: block;
			position: sticky;
			top: 74px;
			z-index: 5;
			margin-top: 16px;
			border-radius: 14px;
			border: 1px solid var(--line);
			background: #090b1a;
		}
		.menu summary {
			padding: 10px 16px;
			cursor: pointer;
			font: 500 0.85rem var(--font-mono);
			color: var(--muted);
		}
		.menu[open] {
			max-height: calc(100vh - 100px);
			overflow-y: auto;
			padding-bottom: 14px;
		}
		.menu[open] > :global(:not(summary)) {
			margin-left: 16px;
			margin-right: 16px;
		}
		.content {
			padding-top: 24px;
		}
	}
	@media (max-width: 600px) {
		.nav {
			margin: 10px 10px 0;
			padding: 8px 10px 8px 16px;
			gap: 12px;
		}
		.crumb,
		.gh {
			display: none;
		}
		nav {
			gap: 16px;
		}
		.frame {
			padding: 0 16px;
		}
	}
</style>
