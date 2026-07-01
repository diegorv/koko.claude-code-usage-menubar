<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { invoke } from '@tauri-apps/api/core';
	import { appState } from '$lib/store.svelte';
	import { formatTimeRemaining, type UsageData } from '$lib/usage';
	import ProgressBar from './ProgressBar.svelte';

	let usage = $state<UsageData | null>(null);
	let refreshing = $state(false);
	let cooldown = $state(false);
	let cooldownTimer: ReturnType<typeof setTimeout> | null = null;
	let unlisten: UnlistenFn | null = null;

	const intervalOptions = [
		{ value: 120, label: '2m' },
		{ value: 300, label: '5m' },
		{ value: 600, label: '10m' }
	];

	let selectedInterval = $state(120);

	async function handleIntervalChange() {
		await appState.saveInterval(selectedInterval);
		await invoke('start_auto_refresh', { intervalSecs: selectedInterval });
	}

	function startCooldown() {
		if (cooldownTimer) clearTimeout(cooldownTimer);
		cooldown = true;
		cooldownTimer = setTimeout(() => { cooldown = false; }, 30_000);
	}

	async function handleRefresh() {
		refreshing = true;
		try {
			usage = await invoke<UsageData>('trigger_refresh');
			startCooldown();
		} catch {
			// invoke failed — usage stays as-is, will get data via event listener
		} finally {
			refreshing = false;
		}
	}

	async function handleQuit() {
		await invoke('quit_app');
	}

	function handleKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			invoke('hide_popup');
		}
	}

	onMount(async () => {
		document.body.classList.add('popup-window');

		await appState.loadSettings();
		selectedInterval = appState.intervalSeconds;

		// Listen for usage updates from Rust-side polling
		unlisten = await listen<UsageData>('usage_updated', (event) => {
			usage = event.payload;
		});

		// Fetch fresh data when popup opens
		handleRefresh();
	});

	onDestroy(() => {
		unlisten?.();
		if (cooldownTimer) clearTimeout(cooldownTimer);
	});
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="popup-container">
	<header>
		<h1>Claude Usage</h1>
		{#if usage?.lastUpdatedAt}
			<span class="timestamp">
				{new Date(usage.lastUpdatedAt).toLocaleTimeString()}
			</span>
		{/if}
	</header>

	{#if !usage}
		<p class="loading">Loading...</p>
	{:else if usage.status !== 'ok'}
		<div class="error">{usage.errorMessage}</div>
	{:else}
		<section class="usage-section">
			<div class="usage-row">
				<div class="usage-label">
					<span>Session (5h)</span>
					<span class="percent">{usage.sessionPercent}%</span>
				</div>
				<ProgressBar percent={usage.sessionPercent} color="#6b7fe0" />
				{#if usage.sessionResetsAt}
					<span class="reset-time">
						Resets in {formatTimeRemaining(usage.sessionResetsAt)}
					</span>
				{/if}
			</div>

			<div class="usage-row">
				<div class="usage-label">
					<span>Weekly</span>
					<span class="percent">{usage.weeklyPercent}%</span>
				</div>
				<ProgressBar percent={usage.weeklyPercent} color="#c060d0" />
				{#if usage.weeklyResetsAt}
					<span class="reset-time">
						Resets in {formatTimeRemaining(usage.weeklyResetsAt)}
					</span>
				{/if}
			</div>

			<div class="usage-row">
				<div class="usage-label">
					<span>Extra Usage</span>
					{#if usage.extraUsageEnabled}
						<span class="percent">{usage.extraUsagePercent}%</span>
					{:else}
						<span class="disabled-label">Disabled</span>
					{/if}
				</div>
				{#if usage.extraUsageEnabled}
					<ProgressBar percent={usage.extraUsagePercent} color="#4db6a0" />
				{/if}
			</div>
		</section>

		{#if usage.models.length > 0}
			<section class="models-section">
				<h2>Models</h2>
				{#each usage.models as model}
					<div class="usage-row">
						<div class="usage-label">
							<span>{model.name}</span>
							<span class="percent">{model.percent}%</span>
						</div>
						<ProgressBar percent={model.percent} />
					</div>
				{/each}
			</section>
		{/if}
	{/if}

	<footer>
		<div class="interval-setting">
			<select bind:value={selectedInterval} onchange={handleIntervalChange}>
				{#each intervalOptions as opt}
					<option value={opt.value}>{opt.label}</option>
				{/each}
			</select>
		</div>
		<button class="action-btn" onclick={handleRefresh} disabled={refreshing || cooldown}>
			{#if refreshing}
				...
			{:else if cooldown}
				<span class="dots">
				<span class="dot"></span><span class="dot"></span><span class="dot"></span>
			</span>
			{:else}
				Refresh
			{/if}
		</button>
		<button class="action-btn quit" onclick={handleQuit}>Quit</button>
	</footer>
</div>

<style>
	:global(html),
	:global(body) {
		margin: 0;
		padding: 0;
		overflow: hidden;
		background: transparent !important;
		font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
		-webkit-user-select: none;
		user-select: none;
	}

	@media (prefers-color-scheme: dark) {
		:global(body.popup-window) {
			--popup-border: rgba(255, 255, 255, 0.15);
			--text-primary: #ffffff;
			--text-secondary: #d0d0d5;
			--segment-off: rgba(255, 255, 255, 0.2);
		}
	}

	@media (prefers-color-scheme: light) {
		:global(body.popup-window) {
			--popup-border: rgba(0, 0, 0, 0.12);
			--text-primary: #000000;
			--text-secondary: #505055;
			--segment-off: rgba(0, 0, 0, 0.2);
		}
	}

	.popup-container {
		padding: 16px;
		color: var(--text-primary);
	}

	header {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		margin-bottom: 16px;
	}

	h1 {
		font-size: 16px;
		font-weight: 600;
		margin: 0;
	}

	.timestamp {
		font-size: 11px;
		color: var(--text-secondary);
	}

	.loading {
		color: var(--text-secondary);
		font-size: 13px;
	}

	.error {
		background: #ff3b3015;
		border: 1px solid #ff3b30;
		border-radius: 8px;
		padding: 12px;
		color: #ff3b30;
		font-size: 13px;
	}

	.usage-section {
		display: flex;
		flex-direction: column;
		gap: 16px;
		margin-bottom: 16px;
	}

	.models-section {
		margin-bottom: 16px;
	}

	.models-section h2 {
		font-size: 12px;
		font-weight: 500;
		color: var(--text-secondary);
		margin: 0 0 10px;
		text-transform: uppercase;
		letter-spacing: 0.5px;
	}

	.models-section .usage-row + .usage-row {
		margin-top: 12px;
	}

	.usage-row {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.usage-label {
		display: flex;
		justify-content: space-between;
		align-items: baseline;
		font-size: 13px;
	}

	.percent {
		font-weight: 600;
		font-size: 14px;
	}

	.reset-time {
		font-size: 11px;
		color: var(--text-secondary);
	}

	.disabled-label {
		font-size: 12px;
		color: var(--text-secondary);
	}

	footer {
		display: flex;
		gap: 8px;
		margin-top: 16px;
		padding-top: 12px;
		border-top: 1px solid var(--popup-border);
	}

	.action-btn {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 6px 12px;
		border-radius: 6px;
		border: 1px solid var(--popup-border);
		background: transparent;
		color: var(--text-primary);
		font-size: 12px;
		cursor: pointer;
		transition: background 0.15s;
	}

	.action-btn:hover:not(:disabled) {
		background: rgba(120, 120, 128, 0.1);
	}

	.action-btn:disabled {
		opacity: 0.5;
		cursor: default;
	}

	.action-btn:disabled:has(.dots) {
		opacity: 1;
	}

	.action-btn.quit {
		color: #ff3b30;
	}

	.dots {
		display: flex;
		width: 100%;
		align-items: center;
		justify-content: center;
		gap: 4px;
	}

	.dot {
		width: 4px;
		height: 4px;
		border-radius: 50%;
		background: var(--text-primary);
		animation: bounce 1.2s ease-in-out infinite;
	}

	.dot:nth-child(2) {
		animation-delay: 0.2s;
	}

	.dot:nth-child(3) {
		animation-delay: 0.4s;
	}

	@keyframes bounce {
		0%, 60%, 100% { transform: translateY(0); }
		30% { transform: translateY(-4px); }
	}

	.interval-setting {
		flex: 1;
		display: flex;
	}

	select {
		flex: 1;
		padding: 6px 12px;
		border-radius: 6px;
		border: 1px solid var(--popup-border);
		background: transparent;
		color: var(--text-primary);
		font-size: 12px;
		cursor: pointer;
		box-sizing: border-box;
		height: 100%;
	}
</style>
