import { describe, expect, it } from 'vitest';
import { fmtAge, fmtAmount, fmtDate, fmtMoneyShort, fmtPct, signed } from './fmt';

describe('fmtMoneyShort (same cases as fmt.rs)', () => {
  it('scales with the same digit rules', () => {
    expect(fmtMoneyShort(1_234_567_890)).toBe('1.23bn');
    expect(fmtMoneyShort(45_600_000)).toBe('45.6m');
    expect(fmtMoneyShort(198_000_000)).toBe('198m');
    expect(fmtMoneyShort(1_200)).toBe('1.20k');
    expect(fmtMoneyShort(999)).toBe('999');
    expect(fmtMoneyShort(0)).toBe('0');
    expect(fmtMoneyShort(-3_010_000)).toBe('-3.01m');
  });
  it('signs', () => {
    expect(signed(542_000)).toBe('+542k');
    expect(signed(-125_000)).toBe('-125k');
  });
});

describe('fmtAmount (Decimal strings)', () => {
  it('groups and normalises', () => {
    expect(fmtAmount('1234567.5')).toBe('1,234,567.5');
    expect(fmtAmount('-94000000')).toBe('-94,000,000');
    expect(fmtAmount('1000000.00')).toBe('1,000,000');
    expect(fmtAmount('0.37600')).toBe('0.376');
    expect(fmtAmount('111749850.24')).toBe('111,749,850.24');
    expect(fmtAmount('0')).toBe('0');
  });
});

describe('small formats', () => {
  it('percent, age, date', () => {
    expect(fmtPct(222.65)).toBe('222.7%');
    expect(fmtPct(null)).toBe('—');
    expect(fmtAge(45)).toBe('45s');
    expect(fmtAge(1_900_000)).toBe('21d');
    expect(fmtDate('2026-11-04')).toBe('Wed 04 Nov');
  });
});
