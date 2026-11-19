<script lang="ts">
	import Code from './Code.svelte';
	import { json, type Tool } from './tools.ts';

	let { tool }: { tool: Tool } = $props();
</script>

<article class="tool" id={tool.name}>
	<h3>
		<a href="#{tool.name}"><code>{tool.name}</code></a>
		{#if tool.unreleased}<span class="kind soon">next release</span>{/if}
	</h3>
	<p>{tool.summary}</p>
	<p class="when"><span>Reach for it</span> {tool.when}</p>

	<div class="args">
		<h4>Arguments</h4>
		<dl>
			{#each tool.args as a}
				<div>
					<dt><code>{a.name}</code>{#if a.required}<em>required</em>{/if}</dt>
					<dd>{a.text}</dd>
				</div>
			{/each}
		</dl>
	</div>

	<div class="io">
		<Code lang="json" title="call {tool.name}" code={json(tool.call)} wrap />
		<Code lang="json" title="answer" code={json(tool.answer)} wrap />
	</div>

	{#if tool.notes}
		<ul class="notes">
			{#each tool.notes as n}<li>{n}</li>{/each}
		</ul>
	{/if}
</article>

<style>
	.tool {
		scroll-margin-top: 90px;
		margin: 26px 0;
		padding: 24px 24px 10px;
		border-radius: var(--radius);
		border: 1px solid var(--line);
		background: var(--panel);
	}
	h3 {
		margin: 0 0 10px;
	}
	h3 a {
		text-decoration: none;
	}
	h3 code {
		font-size: 1.05rem;
		color: var(--star);
		background: none;
		border: 0;
		padding: 0;
	}
	.when {
		font-size: 0.94rem;
	}
	.when span {
		font: 600 0.68rem var(--font-mono);
		letter-spacing: 0.1em;
		text-transform: uppercase;
		color: var(--sat);
		margin-right: 6px;
	}
	dl {
		margin: 0;
		display: grid;
		gap: 6px;
	}
	dl div {
		display: grid;
		grid-template-columns: minmax(130px, auto) 1fr;
		gap: 14px;
		font-size: 0.9rem;
	}
	dt {
		white-space: nowrap;
	}
	dt em {
		margin-left: 6px;
		font: 500 0.68rem var(--font-mono);
		font-style: normal;
		color: var(--star-hot);
	}
	dd {
		margin: 0;
	}
	.io :global(.code + .code) {
		margin-top: -6px;
	}
	.io :global(pre) {
		max-height: 420px;
	}
	.notes {
		font-size: 0.92rem;
	}
	@media (max-width: 760px) {
		.tool {
			padding: 18px 16px 6px;
		}
		dl div {
			grid-template-columns: 1fr;
			gap: 0;
		}
	}
</style>
