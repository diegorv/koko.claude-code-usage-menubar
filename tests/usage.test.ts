import { describe, it, expect } from 'vitest';
import { barColor, COLOR_CRITICAL, COLOR_WARNING, formatTimeRemaining } from '../src/lib/usage';

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
});
