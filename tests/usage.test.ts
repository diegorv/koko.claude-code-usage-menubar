import { describe, it, expect } from 'vitest';
import {
	barColor,
	COLOR_CRITICAL,
	COLOR_WARNING,
	formatTimeRemaining,
	type ProviderUsage,
	type UsageData
} from '../src/lib/usage';

describe('barColor', () => {
	const base = '#6b7fe0';

	it('keeps the identity color below the warning threshold', () => {
		expect(barColor(0, base)).toBe(base);
		expect(barColor(79, base)).toBe(base);
	});

	it('warns from 80%', () => {
		expect(barColor(80, base)).toBe(COLOR_WARNING);
		expect(barColor(94, base)).toBe(COLOR_WARNING);
	});

	it('goes critical from 95%', () => {
		expect(barColor(95, base)).toBe(COLOR_CRITICAL);
		expect(barColor(100, base)).toBe(COLOR_CRITICAL);
	});
});

describe('formatTimeRemaining', () => {
	it('returns empty string for undefined', () => {
		expect(formatTimeRemaining(undefined)).toBe('');
	});

	it('returns "Now" for past dates', () => {
		const pastDate = new Date(Date.now() - 60000).toISOString();
		expect(formatTimeRemaining(pastDate)).toBe('Now');
	});

	it('formats minutes only', () => {
		const future = new Date(Date.now() + 25 * 60000).toISOString();
		const result = formatTimeRemaining(future);
		expect(result).toMatch(/^\d+m$/);
	});

	it('formats hours and minutes', () => {
		const future = new Date(Date.now() + 2.5 * 3600000).toISOString();
		const result = formatTimeRemaining(future);
		expect(result).toMatch(/^\d+h \d+m$/);
	});

	it('formats days and hours', () => {
		const future = new Date(Date.now() + 50 * 3600000).toISOString();
		const result = formatTimeRemaining(future);
		expect(result).toMatch(/^\d+d \d+h$/);
	});

	// Anthropic sends millisecond precision; Kimi's resetTime carries six
	// fractional digits ("2026-08-04T11:59:17.868440Z"). Both reach this
	// function unparsed, straight off the wire.
	it('parses the microsecond precision Kimi sends', () => {
		const future = new Date(Date.now() + 2.5 * 3600000)
			.toISOString()
			.replace(/\.(\d{3})Z$/, '.$1440Z');
		expect(future).toMatch(/\.\d{6}Z$/);
		expect(formatTimeRemaining(future)).toMatch(/^\d+h \d+m$/);
	});

	it('returns empty string for an unparseable timestamp', () => {
		expect(formatTimeRemaining('not a date')).toBe('');
	});
});

// The payload shape is pinned on the Rust side by
// serialized_payload_has_expected_shape in parser.rs / kimi_parser.rs. These
// hold the frontend's half of that contract: if ProviderExtra or ProviderUsage
// drift away from what Rust serializes, this file stops type-checking.
describe('provider payload contract', () => {
	const claude: ProviderUsage = {
		id: 'claude',
		title: 'Claude Usage',
		status: 'ok',
		sessionPercent: 45,
		sessionResetsAt: '2026-07-30T00:00:00Z',
		weeklyPercent: 67,
		models: [{ name: 'Fable', percent: 30, resetsAt: '2026-08-04T00:00:00Z' }],
		extra: { kind: 'extra_usage', enabled: false, percent: 0 }
	};

	const kimi: ProviderUsage = {
		id: 'kimi',
		title: 'Kimi Usage',
		status: 'ok',
		sessionPercent: 96,
		sessionResetsAt: '2026-07-29T22:59:17.868440Z',
		weeklyPercent: 19,
		weeklyResetsAt: '2026-08-04T11:59:17.868440Z',
		// Kimi reports no per-model breakdown.
		models: [],
		extra: { kind: 'parallel', used: 6, limit: 30 }
	};

	it('narrows extra by kind', () => {
		// The popup switches on `kind` to pick between a percentage bar and a
		// used/limit count; each arm must only see its own fields.
		const describe_ = (p: ProviderUsage): string =>
			p.extra.kind === 'parallel'
				? `${p.extra.used}/${p.extra.limit}`
				: p.extra.kind === 'extra_usage'
					? `${p.extra.percent}%`
					: '';

		const gpt: ProviderUsage = { ...kimi, id: 'gpt', title: 'GPT Usage', extra: { kind: 'none' } };

		expect(describe_(kimi)).toBe('6/30');
		expect(describe_(claude)).toBe('0%');
		expect(describe_(gpt)).toBe('');
	});

	it('carries both providers in one payload, Claude first', () => {
		const payload: UsageData = {
			providers: [claude, kimi],
			lastUpdatedAt: Date.UTC(2026, 6, 30, 12, 0, 0)
		};
		// The tray reads providers[0].
		expect(payload.providers[0].id).toBe('claude');
		expect(payload.providers.map((p) => p.title)).toEqual(['Claude Usage', 'Kimi Usage']);
	});

	it('treats a keyless install as a Claude-only payload', () => {
		const payload: UsageData = { providers: [claude], lastUpdatedAt: 0 };
		expect(payload.providers).toHaveLength(1);
	});
});
