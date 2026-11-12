<script lang="ts">
	let { size = 26 }: { size?: number } = $props();
</script>

<svg viewBox="0 0 32 32" width={size} height={size} aria-hidden="true">
	<circle cx="16" cy="16" r="6.5" fill="url(#logo-star)" />
	<ellipse cx="16" cy="16" rx="13" ry="5" fill="none" stroke="#7aa2ff" stroke-width="1.4" transform="rotate(-25 16 16)" />
	<circle cx="27.2" cy="10.9" r="2.2" fill="#9fe7ff" />
	<defs>
		<radialGradient id="logo-star"><stop offset="0" stop-color="#fff6d8" /><stop offset=".6" stop-color="#ffc857" /><stop offset="1" stop-color="#ff8a3d" /></radialGradient>
	</defs>
</svg>
