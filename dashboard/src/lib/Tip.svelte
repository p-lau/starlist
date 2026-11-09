<script lang="ts">
	// A small "?" that explains a term. Hover or focus shows it on a desktop;
	// a tap toggles it on a phone. Fixed positioning keeps it clear of cards
	// that clip their overflow.
	let { text, label = 'What is this?' }: { text: string; label?: string } = $props();

	const id = `tip-${Math.random().toString(36).slice(2, 9)}`;
	let open = $state(false);
	let btn: HTMLButtonElement;
	let pos = $state({ left: 0, top: 0 });

	function show() {
		const r = btn.getBoundingClientRect();
		const w = Math.min(280, window.innerWidth - 32);
		pos = {
			left: Math.max(16, Math.min(r.left + r.width / 2 - w / 2, window.innerWidth - w - 16)),
			top: r.bottom + 6
		};
		open = true;
	}
	const hide = () => (open = false);
	const hovers = () => matchMedia('(hover: hover)').matches;
</script>

<svelte:window
	onkeydown={(e) => e.key === 'Escape' && hide()}
	onscroll={hide}
	onresize={hide}
	onpointerdown={(e) => open && !btn.contains(e.target as Node) && hide()}
/>

<button
	bind:this={btn}
	type="button"
	class="tip"
	aria-label={label}
	aria-expanded={open}
	aria-describedby={open ? id : undefined}
	onclick={() => (open && !hovers() ? hide() : show())}
	onmouseenter={() => hovers() && show()}
	onmouseleave={() => hovers() && hide()}
	onfocus={() => btn.matches(':focus-visible') && show()}
	onblur={hide}>?</button
>
{#if open}
	<span {id} role="tooltip" class="pop" style:left="{pos.left}px" style:top="{pos.top}px">{text}</span>
{/if}

<style>
	.tip {
		display: inline-grid;
		place-items: center;
		width: 18px;
		height: 18px;
		padding: 0;
		margin-left: 6px;
		border-radius: 50%;
		border: 1px solid var(--line-strong);
		background: transparent;
		color: var(--muted);
		font-size: 0.7rem;
		font-weight: 700;
		line-height: 1;
		vertical-align: 0.15em;
		cursor: help;
	}
	.tip:hover,
	.tip[aria-expanded='true'] {
		color: var(--accent);
		border-color: var(--accent);
	}
	.pop {
		position: fixed;
		z-index: 50;
		width: min(280px, calc(100vw - 32px));
		padding: 8px 10px;
		border-radius: 8px;
		border: 1px solid var(--line-strong);
		background: var(--surface);
		color: var(--text);
		box-shadow: 0 6px 20px rgb(0 0 0 / 0.18);
		font-size: 0.82rem;
		font-weight: 400;
		line-height: 1.45;
		letter-spacing: normal;
		text-transform: none;
		text-align: left;
		white-space: normal;
	}
</style>
