import { describe, it, expect } from 'vitest';
import {
	DEFAULT_PROVIDER_SETTINGS,
	MAX_TRAY_PROVIDERS,
	normalizeProviderSettings,
	trayCount,
	withEnabled,
	withTray,
	type ProviderSettings
} from '../src/lib/providers';

const allOn: ProviderSettings = {
	claude: { enabled: true, tray: true },
	kimi: { enabled: true, tray: true },
	gpt: { enabled: true, tray: false }
};

describe('provider settings', () => {
	it('defaults to the pre-setting behaviour with GPT opt-in', () => {
		expect(trayCount(DEFAULT_PROVIDER_SETTINGS)).toBe(2);
		expect(DEFAULT_PROVIDER_SETTINGS.gpt.enabled).toBe(false);
	});

	it('refuses a third tray provider', () => {
		expect(trayCount(allOn)).toBe(MAX_TRAY_PROVIDERS);
		expect(withTray(allOn, 'gpt', true)).toBe(allOn);
	});

	it('lets a freed slot go to another provider', () => {
		const next = withTray(withTray(allOn, 'claude', false), 'gpt', true);
		expect(next.claude.tray).toBe(false);
		expect(next.gpt.tray).toBe(true);
		// Claude is still fetched and shown in the popup.
		expect(next.claude.enabled).toBe(true);
	});

	it('takes a disabled provider off the tray', () => {
		const next = withEnabled(allOn, 'kimi', false);
		expect(next.kimi).toEqual({ enabled: false, tray: false });
		expect(trayCount(next)).toBe(1);
		// Re-enabling does not silently put it back.
		expect(withEnabled(next, 'kimi', true).kimi.tray).toBe(false);
	});

	it('refuses the tray for a disabled provider', () => {
		const off = withEnabled(allOn, 'gpt', false);
		expect(withTray(off, 'gpt', true)).toBe(off);
	});

	it('falls back to defaults for a missing or malformed saved value', () => {
		expect(normalizeProviderSettings(undefined)).toEqual(DEFAULT_PROVIDER_SETTINGS);
		const partial = normalizeProviderSettings({ gpt: { enabled: true, tray: true }, kimi: 'x' });
		expect(partial.gpt).toEqual({ enabled: true, tray: true });
		expect(partial.kimi).toEqual(DEFAULT_PROVIDER_SETTINGS.kimi);
	});
});
