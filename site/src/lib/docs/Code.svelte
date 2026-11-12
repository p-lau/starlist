<script lang="ts">
	/** A code block with a title bar and a copy button. Lines starting with "#" in
	    shell blocks are drawn as comments. */
	let {
		code,
		title = '',
		lang = 'sh',
		wrap = false
	}: { code: string; title?: string; lang?: string; wrap?: boolean } = $props();
	let copied = $state(false);

	const lines = $derived(code.replace(/^\n+|\s+$/g, '').split('\n'));
	const commentMark = $derived(lang === 'sh' || lang === 'yaml' || lang === 'toml' ? '#' : lang === 'groovy' ? '//' : '');

	async function copy() {
		try {
			await navigator.clipboard.writeText(lines.join('\n'));
			copied = true;
			setTimeout(() => (copied = false), 1600);
		} catch {
			/* clipboard blocked: the text is still selectable */
		}
	}
</script>

<figure class="code">
	<figcaption>
		<span class="title">{title || lang}</span>
		<button type="button" onclick={copy} aria-label="Copy {title || 'code'}">{copied ? 'Copied' : 'Copy'}</button>
	</figcaption>
	<pre class:wrap><code>{#each lines as line, i}{#if commentMark && line.trimStart().startsWith(commentMark)}<span class="c">{line}</span>{:else}{line}{/if}{#if i < lines.length - 1}{'\n'}{/if}{/each}</code></pre>
</figure>

<style>
	.code {
		margin: 18px 0;
		border-radius: 12px;
		border: 1px solid var(--line);
		background: rgba(8, 10, 24, 0.8);
		overflow: hidden;
		min-width: 0;
	}
	figcaption {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		padding: 7px 10px 7px 14px;
		border-bottom: 1px solid var(--line);
		background: rgba(255, 255, 255, 0.02);
	}
	.title {
		font: 500 0.72rem var(--font-mono);
		color: var(--faint);
		letter-spacing: 0.04em;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	button {
		flex: none;
		font: 500 0.72rem var(--font-mono);
		color: var(--muted);
		background: rgba(255, 255, 255, 0.04);
		border: 1px solid var(--line);
		border-radius: 7px;
		padding: 3px 9px;
		cursor: pointer;
	}
	button:hover {
		color: var(--text);
		border-color: var(--line-strong);
	}
	pre {
		margin: 0;
		padding: 14px 16px;
		overflow-x: auto;
		font-size: 0.8rem;
		line-height: 1.65;
		color: var(--text);
		tab-size: 2;
		font-variant-ligatures: none;
	}
	pre.wrap {
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}
	.c {
		color: var(--faint);
	}
</style>
