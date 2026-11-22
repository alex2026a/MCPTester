import { describe, expect, it } from "vitest";
import {
  equalShares,
  formatPercentValue,
  formatShareDraft,
  formatUnitsAsPercent,
  isValidPercentTotal,
  normalizeShares,
  parseShareDraft,
  sharesToPercents,
  sumShares,
} from "./loadAllocation";

function sum(values: number[]): number {
  return values.reduce((total, value) => total + value, 0);
}

describe("loadAllocation", () => {
  it("sums shares", () => {
    expect(sumShares([])).toBe(0);
    expect(sumShares([1, 2, 3])).toBe(6);
  });

  it("splits equally with the remainder on earlier rows", () => {
    expect(equalShares(0)).toEqual([]);
    expect(equalShares(1)).toEqual([1000]);
    expect(equalShares(3)).toEqual([334, 333, 333]);
    expect(equalShares(4)).toEqual([250, 250, 250, 250]);
    expect(equalShares(50)).toEqual(Array(50).fill(20));
    expect(equalShares(999)).toEqual([2, ...Array(998).fill(1)]);
    expect(equalShares(1000)).toEqual(Array(1000).fill(1));
    expect(equalShares(1001)).toEqual([]);
    expect(equalShares(-1)).toEqual([]);
    expect(equalShares(2.5)).toEqual([]);
    expect(equalShares(Number.NaN)).toEqual([]);
    expect(equalShares(Number.POSITIVE_INFINITY)).toEqual([]);
  });

  it("normalizes the documented boundary cases", () => {
    expect(normalizeShares([])).toEqual([]);
    expect(normalizeShares([1, 1, 1])).toEqual([334, 333, 333]);
    expect(normalizeShares([700, 300])).toEqual([700, 300]);
    expect(normalizeShares([700, 500])).toEqual([583, 417]);
    expect(normalizeShares([500, 500, 1])).toEqual([500, 499, 1]);
    expect(normalizeShares([1, 1, 9998])).toEqual([1, 1, 998]);
    expect(normalizeShares([1000, 0, 0])).toEqual([998, 1, 1]);
    expect(normalizeShares([1, 0, 0])).toEqual([998, 1, 1]);
    expect(normalizeShares([0, 1000])).toEqual([1, 999]);
    expect(normalizeShares([1, 1, 0])).toEqual([499, 500, 1]);
    expect(normalizeShares([1, 1, 1, 1, 1, 1])).toEqual([
      167, 167, 167, 167, 166, 166,
    ]);
    expect(normalizeShares(Array(7).fill(1))).toEqual([
      143, 143, 143, 143, 143, 143, 142,
    ]);
    expect(normalizeShares(Array(999).fill(1))).toEqual([
      2,
      ...Array(998).fill(1),
    ]);
    expect(normalizeShares(Array(1000).fill(1))).toEqual(Array(1000).fill(1));
    expect(normalizeShares([0, 0, 0])).toEqual([334, 333, 333]);
    expect(normalizeShares([Number.NaN, Number.POSITIVE_INFINITY, -1, 2])).toEqual([
      1, 1, 1, 997,
    ]);
    expect(normalizeShares([1.4, 2.6])).toEqual([250, 750]);
    expect(normalizeShares(Array(1001).fill(1))).toEqual([]);
  });

  it("does not mutate the input array", () => {
    const input = [700, 500];
    const copy = [...input];
    const result = normalizeShares(input);
    expect(result).not.toBe(input);
    expect(input).toEqual(copy);
  });

  it("keeps the length and invariants for arbitrary inputs", () => {
    let seed = 20261114;
    const next = () => {
      seed = (seed * 1664525 + 1013904223) >>> 0;
      return seed;
    };

    for (let length = 1; length <= 50; length += 1) {
      const values = Array.from({ length }, () => next() % 5001);
      const result = normalizeShares(values);
      expect(result).toHaveLength(length);
      expect(result.every((value) => value >= 1)).toBe(true);
      expect(sum(result)).toBe(1000);
      expect(isValidPercentTotal(result)).toBe(true);
    }
  });

  it("converts shares to percentages", () => {
    expect(sharesToPercents([])).toEqual([]);
    expect(sharesToPercents([0, 0])).toEqual([0, 0]);
    expect(sharesToPercents([1, 3])).toEqual([25, 75]);
  });

  it("formats units and percentage values", () => {
    expect(formatUnitsAsPercent(0)).toBe("0.0%");
    expect(formatUnitsAsPercent(1)).toBe("0.1%");
    expect(formatUnitsAsPercent(999)).toBe("99.9%");
    expect(formatUnitsAsPercent(1000)).toBe("100.0%");
    expect(formatPercentValue(0)).toBe("0.0%");
    expect(formatPercentValue(100)).toBe("100.0%");
    expect(formatPercentValue(58.34)).toBe("58.3%");
  });

  it("validates a 100.0% allocation", () => {
    expect(isValidPercentTotal([700, 300])).toBe(true);
    expect(isValidPercentTotal([1000])).toBe(true);
    expect(isValidPercentTotal([700, 299])).toBe(false);
    expect(isValidPercentTotal([])).toBe(false);
    expect(isValidPercentTotal([0, 1000])).toBe(false);
    expect(isValidPercentTotal([1000.5])).toBe(false);
    expect(isValidPercentTotal([1000, 1])).toBe(false);
  });

  it("parses and formats raw drafts without clamping", () => {
    expect(parseShareDraft("70", "percent")).toBe(700);
    expect(parseShareDraft("0.1", "percent")).toBe(1);
    expect(parseShareDraft("100.5", "percent")).toBe(1005);
    expect(parseShareDraft("", "percent")).toBe(0);
    expect(parseShareDraft("abc", "percent")).toBe(0);
    expect(parseShareDraft("12.34", "percent")).toBe(123);
    expect(parseShareDraft("70", "weight")).toBe(70);
    expect(parseShareDraft("12.6", "weight")).toBe(13);
    expect(parseShareDraft("abc", "weight")).toBe(0);

    expect(formatShareDraft(700, "percent")).toBe("70.0");
    expect(formatShareDraft(1, "percent")).toBe("0.1");
    expect(formatShareDraft(0, "percent")).toBe("0.0");
    expect(formatShareDraft(700, "weight")).toBe("700");
    expect(formatShareDraft(0, "weight")).toBe("0");
  });
});
