import { describe, it, expect } from 'vitest';
import { formatTimeRemaining } from '../src/lib/usage';

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
