<script lang="ts">
	import { onMount, onDestroy, tick } from 'svelte';
	import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { invoke } from '@tauri-apps/api/core';
	import { appState } from '$lib/store.svelte';
	import { barColor, formatTimeRemaining, type UsageData } from '$lib/usage';
	import {
		MAX_TRAY_PROVIDERS,
		PROVIDER_IDS,
		PROVIDER_NAMES,
		trayCount,
		withEnabled,
		withTray,
		type ProviderSettings
	} from '$lib/providers';
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
	// Three states, not two: a keychain that won't answer is not the same as
	// a key that isn't there, and telling the user "Not saved" about a stored
	// key sends them to paste it again — the one action that cannot help.
	let keyStatus = $state<'saved' | 'none' | 'unknown'>('none');
	let kimiKeyInput = $state('');
	let kimiKeyBusy = $state(false);
	let kimiKeyError = $state('');

	const keyStatusLabel = { saved: 'Saved', none: 'Not saved', unknown: 'Unknown' };

	let providerSettings = $state<ProviderSettings>(appState.providerSettings);
	let providerSettingsError = $state('');
	const trayFull = $derived(trayCount(providerSettings) >= MAX_TRAY_PROVIDERS);

	async function applyProviderSettings(next: ProviderSettings) {
		providerSettings = next;
		providerSettingsError = '';
		const snapshot = $state.snapshot(next);
		await appState.saveProviderSettings(snapshot);
		try {
			await invoke('set_provider_settings', { settings: snapshot });
		} catch (e) {
			providerSettingsError = String(e);
		}
	}

	async function refreshKeyStatus() {
		try {
			keyStatus = (await invoke<boolean>('has_kimi_key')) ? 'saved' : 'none';
		} catch {
			keyStatus = 'unknown';
		}
	}

	async function toggleSettings() {
		settingsOpen = !settingsOpen;
		kimiKeyError = '';
		if (!settingsOpen) {
			// Drop an unsubmitted draft so a pasted key doesn't linger in the DOM.
			kimiKeyInput = '';
			return;
		}
		await refreshKeyStatus();
	}

	async function handleSaveKimiKey() {
		const key = kimiKeyInput.trim();
		if (!key || kimiKeyBusy) return;
		kimiKeyBusy = true;
		kimiKeyError = '';
		try {
			await invoke('save_kimi_key', { key });
			keyStatus = 'saved';
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
			keyStatus = 'none';
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
		void keyStatus;
		void providerSettings;
		void providerSettingsError;
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
		providerSettings = appState.providerSettings;

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
		<!-- One timestamp, outside the loop: lastUpdatedAt belongs to the fetch,
		     not to a provider, and repeating it under every section implied each
		     one had its own. -->
		{#if usage.lastUpdatedAt}
			<header>
				<span class="timestamp">
					Updated {new Date(usage.lastUpdatedAt).toLocaleTimeString()}
				</span>
			</header>
		{/if}

		{#each usage.providers as provider (provider.id)}
			<h2 class="provider-title">{provider.title}</h2>

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
					{:else if provider.extra.kind === 'parallel'}
						{@const extra = provider.extra}
						<div class="usage-row">
							<div class="usage-label">
								<span>Parallel Sessions</span>
								<span class="percent">{extra.used}/{extra.limit}</span>
							</div>
						</div>
					{/if}
				</section>

				{#if provider.models.length > 0}
					<section class="models-section">
						<h3>Models</h3>
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
					<span>Providers</span>
					<span class="key-status">Tray {trayCount(providerSettings)}/{MAX_TRAY_PROVIDERS}</span>
				</div>
				{#each PROVIDER_IDS as id (id)}
					{@const toggle = providerSettings[id]}
					<div class="provider-toggle">
						<span class="provider-name">{PROVIDER_NAMES[id]}</span>
						<label>
							<input
								type="checkbox"
								checked={toggle.enabled}
								onchange={(e) =>
									applyProviderSettings(withEnabled(providerSettings, id, e.currentTarget.checked))}
							/>
							On
						</label>
						<label>
							<input
								type="checkbox"
								checked={toggle.tray}
								disabled={!toggle.enabled || (!toggle.tray && trayFull)}
								onchange={(e) =>
									applyProviderSettings(withTray(providerSettings, id, e.currentTarget.checked))}
							/>
							Tray
						</label>
					</div>
				{/each}
				{#if providerSettings.gpt.enabled}
					<span class="settings-hint">GPT reads your Codex CLI login (~/.codex/auth.json).</span>
				{/if}
				{#if providerSettingsError}
					<span class="settings-error">{providerSettingsError}</span>
				{/if}
				<div class="settings-header settings-section">
					<span>Kimi API Key</span>
					<span
						class="key-status"
						class:saved={keyStatus === 'saved'}
						class:unknown={keyStatus === 'unknown'}
					>
						{keyStatusLabel[keyStatus]}
					</span>
				</div>
				{#if keyStatus === 'unknown'}
					<span class="settings-hint">
						The keychain didn't answer, so a stored key can't be confirmed.
						Pasting again is safe — it replaces whatever is there.
					</span>
				{/if}
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
						disabled={kimiKeyBusy || keyStatus === 'none'}
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
		justify-content: flex-end;
		margin-bottom: 10px;
	}

	.provider-title {
		font-size: 16px;
		font-weight: 600;
		margin: 0 0 12px;
	}

	/* Sections after the first need air between them. */
	.provider-title ~ .provider-title {
		margin-top: 4px;
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

	.models-section h3 {
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

	.settings-section {
		margin-top: 4px;
		padding-top: 10px;
		border-top: 1px solid var(--popup-border);
	}

	.provider-toggle {
		display: flex;
		align-items: center;
		gap: 12px;
		font-size: 12px;
	}

	.provider-name {
		flex: 1;
	}

	.provider-toggle label {
		display: flex;
		align-items: center;
		gap: 4px;
		cursor: pointer;
	}

	.provider-toggle label:has(input:disabled) {
		opacity: 0.5;
		cursor: default;
	}

	.key-status {
		font-size: 11px;
		color: var(--text-secondary);
	}

	.key-status.saved {
		color: #34c759;
	}

	.key-status.unknown {
		color: #e0a030;
	}

	.settings-hint {
		font-size: 11px;
		line-height: 1.4;
		color: var(--text-secondary);
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
