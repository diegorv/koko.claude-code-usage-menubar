<script lang="ts">
	let { percent = 0, color = '#6b7fe0' }: { percent: number; color?: string } = $props();

	let clampedPercent = $derived(Math.min(100, Math.max(0, percent)));
</script>

<div class="progress-track">
	<div
		class="progress-fill"
		style="width: {clampedPercent}%; background-color: {color};"
	></div>
</div>

<style>
	.progress-track {
		width: 100%;
		height: 8px;
		background: var(--track-bg, rgba(120, 120, 128, 0.2));
		border-radius: 4px;
		overflow: hidden;
	}

	.progress-fill {
		height: 100%;
		border-radius: 4px;
		transition: width 0.3s ease, background-color 0.3s ease;
	}
</style>
