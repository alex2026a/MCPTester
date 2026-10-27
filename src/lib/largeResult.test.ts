import { afterEach, describe, expect, it, vi } from "vitest";
import { LARGE_COPY_BYTES, confirmLargeResultRead } from "./largeResult";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("confirmLargeResultRead", () => {
  it("非截断或小于阈值时直接允许", () => {
    const confirm = vi.spyOn(window, "confirm");
    expect(
      confirmLargeResultRead({ truncated: false, size_bytes: 20 * 1024 * 1024 }),
    ).toBe(true);
    expect(
      confirmLargeResultRead({
        truncated: true,
        size_bytes: LARGE_COPY_BYTES - 1,
      }),
    ).toBe(true);
    expect(confirm).not.toHaveBeenCalled();
  });

  it("大于等于阈值时由用户确认", () => {
    const confirm = vi
      .spyOn(window, "confirm")
      .mockReturnValueOnce(false)
      .mockReturnValueOnce(true);
    const record = { truncated: true, size_bytes: LARGE_COPY_BYTES };
    expect(confirmLargeResultRead(record)).toBe(false);
    expect(confirmLargeResultRead(record)).toBe(true);
    expect(confirm).toHaveBeenCalledTimes(2);
  });
});
