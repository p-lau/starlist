<script lang="ts">
	import type { Timeline } from './types.ts';
	import { group, GROUP_LABEL, num, tick, when, type Group } from './format.ts';

	let { data, height = 200 }: { data: Timeline; height?: number } = $props();

	// Errors sit on the baseline so they're the easiest to read, then warnings.
	const ORDER: Group[] = ['error', 'warn', 'info'];

	let width = $state(640);
	let hover: number | null = $state(null);

	const pad = { top: 22, right: 8, bottom: 22, left: 40 };

	const buckets = $derived.by(() => {
		const n = data.series[0]?.counts.length ?? 0;
		const out = Array.from({ length: n }, () => ({ error: 0, warn: 0, info: 0 }));
		for (const s of data.series) {
			const g = group(s.level);
			s.counts.forEach((c, i) => (out[i][g] += c));
		}
		return out;
	});
	const present = $derived(ORDER.filter((g) => buckets.some((b) => b[g] > 0)));
	const from = $derived(new Date(data.from).getTime());
	const stepMs = $derived(data.step_seconds * 1000);
	const max = $derived(niceMax(Math.max(1, ...buckets.map((b) => b.error + b.warn + b.info))));

	const plotW = $derived(Math.max(10, width - pad.left - pad.right));
	const plotH = $derived(height - pad.top - pad.bottom);
	const bw = $derived(plotW / Math.max(1, buckets.length));
	const gap = $derived(bw > 6 ? 2 : bw > 3 ? 1 : 0);

	function niceMax(v: number): number {
		const p = 10 ** Math.floor(Math.log10(v));
		for (const m of [1, 2, 2.5, 5, 10]) if (m * p >= v) return m * p;
		return 10 * p;
	}
	const y = (v: number) => pad.top + plotH - (v / max) * plotH;
	const xAt = (ms: number) => pad.left + ((ms - from) / stepMs) * bw;

	/** A bar with its top corners rounded, anchored at the bottom. */
	function bar(x: number, top: number, w: number, h: number, r: number): string {
		r = Math.min(r, w / 2, h);
		return `M${x},${top + h}V${top + r}Q${x},${top} ${x + r},${top}H${x + w - r}Q${x + w},${top} ${x + w},${top + r}V${top + h}Z`;
	}

	const segments = $derived(
		buckets.map((b, i) => {
			let base = 0;
			const segs: { g: Group; d: string }[] = [];
			const stack = ORDER.filter((g) => b[g] > 0);
			stack.forEach((g, k) => {
				const y0 = y(base);
				const y1 = y(base + b[g]);
				const top = k === stack.length - 1;
				// A 2px surface gap between stacked segments.
				const h = Math.max(1, y0 - y1 - (k > 0 ? gap : 0));
				const x = pad.left + i * bw + gap / 2;
				const w = Math.max(1, bw - gap);
				segs.push({ g, d: top ? bar(x, y1, w, h, 3) : `M${x},${y1}h${w}v${h}h${-w}Z` });
				base += b[g];
			});
			return segs;
		})
	);

	const xticks = $derived.by(() => {
		const n = Math.max(2, Math.floor(plotW / 110));
		const every = Math.max(1, Math.ceil(buckets.length / n));
		const out: { x: number; label: string }[] = [];
		for (let i = 0; i < buckets.length; i += every) {
			out.push({ x: pad.left + i * bw, label: tick(new Date(from + i * stepMs), data.step_seconds) });
		}
		return out;
	});

	// Newest markers are labelled first; an older label that would collide is
	// left out (its line and tooltip stay).
	const markers = $derived.by(() => {
		const ms = data.markers
			.map((m) => ({ ...m, x: xAt(new Date(m.at).getTime()), label: true, end: false }))
			.filter((m) => m.x >= pad.left && m.x <= pad.left + plotW)
			.sort((a, b) => b.x - a.x);
		let lastX = Infinity;
		for (const m of ms) {
			m.end = m.x > pad.left + plotW - 70;
			const room = m.end ? lastX - m.x : lastX - m.x - 70;
			m.label = lastX === Infinity || room > 74;
			if (m.label) lastX = m.end ? m.x - 70 : m.x;
		}
		return ms;
	});

	function onmove(e: PointerEvent) {
		const r = (e.currentTarget as SVGElement).getBoundingClientRect();
		const i = Math.floor((e.clientX - r.left - pad.left) / bw);
		hover = i >= 0 && i < buckets.length ? i : null;
	}
	const tip = $derived.by(() => {
		if (hover === null) return null;
		const b = buckets[hover];
		const start = new Date(from + hover * stepMs);
		const x = pad.left + (hover + 0.5) * bw;
		return { b, start, end: new Date(start.getTime() + stepMs), left: x > width / 2 };
	});
</script>

<div class="chart" bind:clientWidth={width}>
	<div class="legend" aria-hidden="true">
		{#each present as g (g)}
			<span><i class={g}></i>{GROUP_LABEL[g]}</span>
		{/each}
		{#if markers.length}<span><i class="mk"></i>Deploy or marker</span>{/if}
	</div>
	<svg
		{width}
		{height}
		role="img"
		aria-label="Events per {data.step_seconds / 60} minutes by level"
		onpointermove={onmove}
		onpointerleave={() => (hover = null)}
	>
		{#each [0, 0.5, 1] as f (f)}
			<line x1={pad.left} x2={pad.left + plotW} y1={y(max * f)} y2={y(max * f)} class="grid" />
			<text x={pad.left - 6} y={y(max * f) + 4} class="axis" text-anchor="end">{num(max * f)}</text>
		{/each}
		{#each xticks as t (t.x)}
			<text x={t.x} y={height - 6} class="axis">{t.label}</text>
		{/each}
		{#if hover !== null}
			<rect x={pad.left + hover * bw} y={pad.top} width={bw} height={plotH} class="hover" />
		{/if}
		{#each segments as segs, i (i)}
			{#each segs as s (s.g)}
				<path d={s.d} class={s.g} />
			{/each}
		{/each}
		{#each markers as m (m.name + m.at)}
			<line x1={m.x} x2={m.x} y1={pad.top - 6} y2={pad.top + plotH} class="marker"><title>{m.name}, {when(m.at)}</title></line>
			{#if m.label}
				<text x={m.end ? m.x - 4 : m.x + 4} y={pad.top - 8} class="mlabel" text-anchor={m.end ? 'end' : 'start'}>{m.name}</text>
			{/if}
		{/each}
	</svg>
	{#if tip}
		<div class="tip" style="{tip.left ? 'right' : 'left'}: {tip.left ? width - pad.left - (hover! * bw) + 8 : pad.left + (hover! + 1) * bw + 8}px">
			<div class="faint">{when(tip.start.toISOString())}</div>
			{#each ORDER as g (g)}
				<div class="tiprow"><i class={g}></i>{GROUP_LABEL[g]}<b>{num(tip.b[g])}</b></div>
			{/each}
		</div>
	{/if}
	<details>
		<summary>Show as table</summary>
		<div class="tablewrap">
			<table>
				<thead>
					<tr><th>From</th>{#each ORDER as g (g)}<th class="num">{GROUP_LABEL[g]}</th>{/each}</tr>
				</thead>
				<tbody>
					{#each buckets as b, i (i)}
						{#if b.error + b.warn + b.info > 0}
							<tr>
								<td>{when(new Date(from + i * stepMs).toISOString())}</td>
								{#each ORDER as g (g)}<td class="num">{num(b[g])}</td>{/each}
							</tr>
						{/if}
					{/each}
				</tbody>
			</table>
		</div>
	</details>
</div>

<style>
	.chart {
		position: relative;
		min-width: 0;
		width: 100%;
	}
	svg {
		max-width: 100%;
	}
	svg {
		display: block;
		touch-action: pan-y;
	}
	.legend {
		display: flex;
		flex-wrap: wrap;
		gap: 14px;
		font-size: 0.8rem;
		color: var(--muted);
		margin-bottom: 4px;
	}
	.legend span,
	.tiprow {
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}
	i {
		display: inline-block;
		width: 10px;
		height: 10px;
		border-radius: 3px;
	}
	i.mk {
		width: 2px;
		height: 12px;
		border-radius: 0;
		background: var(--star);
	}
	.error {
		fill: var(--error);
		background: var(--error);
	}
	.warn {
		fill: var(--warn);
		background: var(--warn);
	}
	.info {
		fill: var(--info);
		background: var(--info);
	}
	.grid {
		stroke: var(--line);
		stroke-width: 1;
	}
	.axis {
		fill: var(--faint);
		font-size: 11px;
		font-variant-numeric: tabular-nums;
	}
	.hover {
		fill: var(--surface-2);
	}
	.marker {
		stroke: var(--star);
		stroke-width: 1.5;
		stroke-dasharray: 3 3;
	}
	.mlabel {
		fill: var(--muted);
		font-size: 11px;
		font-weight: 600;
	}
	.tip {
		position: absolute;
		top: 30px;
		pointer-events: none;
		background: var(--surface);
		border: 1px solid var(--line-strong);
		border-radius: 8px;
		padding: 8px 10px;
		font-size: 0.8rem;
		box-shadow: 0 6px 20px rgb(0 0 0 / 0.12);
		min-width: 170px;
		z-index: 2;
	}
	.tiprow {
		display: flex;
		width: 100%;
	}
	.tiprow b {
		margin-left: auto;
		font-variant-numeric: tabular-nums;
	}
	details {
		margin-top: 6px;
		font-size: 0.8rem;
	}
	summary {
		cursor: pointer;
		color: var(--muted);
	}
	.tablewrap {
		max-height: 260px;
		overflow: auto;
		margin-top: 6px;
	}
</style>
