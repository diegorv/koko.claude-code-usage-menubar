<script lang="ts">
	const SEGMENTS = 10;

	let { percent = 0, color = '#6b7fe0' }: { percent: number; color?: string } = $props();

	let filled = $derived(Math.round((Math.min(100, Math.max(0, percent)) / 100) * SEGMENTS));
</script>

<div class="segments">
	{#each Array(SEGMENTS) as _, i}
		<div
			class="segment"
			class:on={i < filled}
			style="--on-color: {color};"
		></div>
	{/each}
</div>

<style>
	.segments {
		display: flex;
		gap: 2px;
		width: 100%;
		height: 8px;
	}

	.segment {
		flex: 1;
		background: var(--segment-off, rgba(120, 120, 128, 0.5));
		border-radius: 2px;
		transition: background-color 0.3s ease;
	}

	.segment.on {
		background: var(--on-color);
	}
</style>
