<script lang="ts">
	import { onMount, onDestroy, tick } from 'svelte';
	import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { invoke } from '@tauri-apps/api/core';
	import { appState } from '$lib/store.svelte';
	import { barColor, formatTimeRemaining, type UsageData } from '$lib/usage';
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

	let settingsOpen = $state(false);
	let hasKimiKey = $state(false);
	let kimiKeyInput = $state('');
	let kimiKeyBusy = $state(false);
	let kimiKeyError = $state('');

	async function toggleSettings() {
		settingsOpen = !settingsOpen;
		kimiKeyError = '';
		if (settingsOpen) {
			try {
				hasKimiKey = await invoke<boolean>('has_kimi_key');
			} catch {
				hasKimiKey = false;
			}
		}
	}

	async function handleSaveKimiKey() {
		const key = kimiKeyInput.trim();
		if (!key || kimiKeyBusy) return;
		kimiKeyBusy = true;
		kimiKeyError = '';
		try {
			await invoke('save_kimi_key', { key });
			hasKimiKey = true;
			kimiKeyInput = '';
		} catch (e) {
			kimiKeyError = String(e);
		} finally {
			kimiKeyBusy = false;
		}
	}

	async function handleRemoveKimiKey() {
		if (kimiKeyBusy) return;
		kimiKeyBusy = true;
		kimiKeyError = '';
		try {
			await invoke('delete_kimi_key');
			hasKimiKey = false;
		} catch (e) {
			kimiKeyError = String(e);
		} finally {
			kimiKeyBusy = false;
		}
	}

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

	// The popup has no scrollbar, so anything taller than the window is simply
	// cut off — the footer buttons were. Content height varies: the models
	// section grows with however many scoped limits the API reports, and the
	// shape warning appears only sometimes. Measure and resize instead of
	// guessing a fixed height.
	const POPUP_WIDTH = 320;

	async function fitWindowToContent() {
		await tick();
		const el = document.querySelector('.popup-container');
		if (!el) return;
		const height = Math.ceil(el.getBoundingClientRect().height);
		try {
			await getCurrentWindow().setSize(new LogicalSize(POPUP_WIDTH, height));
		} catch {
			// Resize not permitted — keep the size configured at build time.
		}
	}

	$effect(() => {
		// Re-fit whenever what we render changes.
		void usage;
		void settingsOpen;
		void kimiKeyError;
		fitWindowToContent();
	});

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
	{#if !usage}
		<p class="loading">Loading...</p>
	{:else}
		{#each usage.providers as provider (provider.id)}
			<header>
				<h1>{provider.title}</h1>
				{#if usage.lastUpdatedAt}
					<span class="timestamp">
						{new Date(usage.lastUpdatedAt).toLocaleTimeString()}
					</span>
				{/if}
			</header>

			{#if provider.status === 'ok'}
				{#if provider.shapeWarning}
					<div class="shape-warning">{provider.shapeWarning}</div>
				{/if}

				<section class="usage-section">
					<div class="usage-row">
						<div class="usage-label">
							<span>Session (5h)</span>
							<span class="percent">{provider.sessionPercent}%</span>
						</div>
						<ProgressBar percent={provider.sessionPercent} color={barColor(provider.sessionPercent, '#6b7fe0')} />
						{#if provider.sessionResetsAt}
							<span class="reset-time">
								Resets in {formatTimeRemaining(provider.sessionResetsAt)}
							</span>
						{/if}
					</div>

					<div class="usage-row">
						<div class="usage-label">
							<span>Weekly</span>
							<span class="percent">{provider.weeklyPercent}%</span>
						</div>
						<ProgressBar percent={provider.weeklyPercent} color={barColor(provider.weeklyPercent, '#c060d0')} />
						{#if provider.weeklyResetsAt}
							<span class="reset-time">
								Resets in {formatTimeRemaining(provider.weeklyResetsAt)}
							</span>
						{/if}
					</div>

					{#if provider.extra.kind === 'extra_usage'}
						{@const extra = provider.extra}
						<div class="usage-row">
							<div class="usage-label">
								<span>Extra Usage</span>
								{#if extra.enabled}
									<span class="percent">{extra.percent}%</span>
								{:else}
									<span class="disabled-label">Disabled</span>
								{/if}
							</div>
							{#if extra.enabled}
								<ProgressBar percent={extra.percent} color={barColor(extra.percent, '#4db6a0')} />
							{/if}
						</div>
					{/if}
				</section>

				{#if provider.models.length > 0}
					<section class="models-section">
						<h2>Models</h2>
						{#each provider.models as model}
							<div class="usage-row">
								<div class="usage-label">
									<span>{model.name}</span>
									<span class="percent">{model.percent}%</span>
								</div>
								<ProgressBar percent={model.percent} color={barColor(model.percent, '#3fa0c9')} />
								{#if model.resetsAt}
									<span class="reset-time">
										Resets in {formatTimeRemaining(model.resetsAt)}
									</span>
								{/if}
							</div>
						{/each}
					</section>
				{/if}
			{:else if provider.status !== 'disabled'}
				<div class="error">{provider.errorMessage}</div>
			{/if}
		{/each}
	{/if}

	<footer>
		{#if settingsOpen}
			<div class="settings-panel">
				<div class="settings-header">
					<span>Kimi API Key</span>
					<span class="key-status" class:saved={hasKimiKey}>
						{hasKimiKey ? 'Saved' : 'Not saved'}
					</span>
				</div>
				<input
					type="password"
					class="key-input"
					placeholder="Paste key here"
					bind:value={kimiKeyInput}
				/>
				{#if kimiKeyError}
					<span class="settings-error">{kimiKeyError}</span>
				{/if}
				<div class="settings-actions">
					<button
						class="action-btn"
						onclick={handleSaveKimiKey}
						disabled={kimiKeyBusy || kimiKeyInput.trim() === ''}
					>
						Save
					</button>
					<button
						class="action-btn"
						onclick={handleRemoveKimiKey}
						disabled={kimiKeyBusy || !hasKimiKey}
					>
						Remove
					</button>
				</div>
			</div>
		{/if}
		<div class="footer-row">
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
			<button class="action-btn" onclick={toggleSettings}>Settings</button>
			<button class="action-btn quit" onclick={handleQuit}>Quit</button>
		</div>
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

	.shape-warning {
		background: #e0a03018;
		border: 1px solid #e0a030;
		border-radius: 6px;
		padding: 8px 10px;
		margin-bottom: 14px;
		color: #e0a030;
		font-size: 11px;
		line-height: 1.4;
	}

	.disabled-label {
		font-size: 12px;
		color: var(--text-secondary);
	}

	footer {
		display: flex;
		flex-direction: column;
		gap: 10px;
		margin-top: 16px;
		padding-top: 12px;
		border-top: 1px solid var(--popup-border);
	}

	.footer-row {
		display: flex;
		gap: 8px;
	}

	.settings-panel {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.settings-header {
		display: flex;
		justify-content: space-between;
		align-items: baseline;
		font-size: 12px;
		font-weight: 500;
	}

	.key-status {
		font-size: 11px;
		color: var(--text-secondary);
	}

	.key-status.saved {
		color: #34c759;
	}

	.key-input {
		width: 100%;
		box-sizing: border-box;
		padding: 6px 12px;
		border-radius: 6px;
		border: 1px solid var(--popup-border);
		background: transparent;
		color: var(--text-primary);
		font-size: 12px;
		-webkit-user-select: text;
		user-select: text;
	}

	.settings-error {
		color: #ff3b30;
		font-size: 11px;
		line-height: 1.4;
	}

	.settings-actions {
		display: flex;
		gap: 8px;
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
