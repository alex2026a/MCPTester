/**
 * 压测操作占比换算。
 *
 * 百分比模式下 1 unit = 0.1%，1000 units = 100%；
 * 权重模式下 unit 只是相对值，总和不要求 1000。
 * 常量与 src-tauri/src/loadtest/models.rs 的 MAX_WEIGHT / MAX_OPERATIONS 手工同步。
 */
export const SHARE_UNITS = 1000;
export const MIN_SHARE_UNITS = 1;
export const MAX_LOAD_OPERATIONS = 50;

export type ShareInputMode = "percent" | "weight";

export function sumShares(values: number[]): number {
  return values.reduce((sum, value) => sum + value, 0);
}

export function equalShares(count: number): number[] {
  if (!Number.isInteger(count) || count <= 0 || count > SHARE_UNITS) {
    return [];
  }
  const base = Math.floor(SHARE_UNITS / count);
  const remainder = SHARE_UNITS - base * count;
  return Array.from(
    { length: count },
    (_, index) => base + (index < remainder ? 1 : 0),
  );
}

/** 按当前比例缩放到 1000，每项至少 1；全 0 时均分。 */
export function normalizeShares(values: number[]): number[] {
  if (values.length === 0 || values.length > SHARE_UNITS) {
    return [];
  }
  const safe = values.map((value) =>
    Number.isFinite(value) ? Math.max(0, Math.round(value)) : 0,
  );
  const total = sumShares(safe);
  if (total <= 0) {
    return equalShares(safe.length);
  }

  const raw = safe.map((value) => (value / total) * SHARE_UNITS);
  const shares = raw.map((value) =>
    Math.max(MIN_SHARE_UNITS, Math.floor(value)),
  );
  let diff = SHARE_UNITS - sumShares(shares);

  if (diff > 0) {
    const candidates = raw
      .map((value, index) => ({
        index,
        remainder: value - Math.floor(value),
        notClamped: value >= MIN_SHARE_UNITS,
      }))
      .filter((item) => item.notClamped);
    const pool =
      candidates.length > 0
        ? candidates
        : raw.map((value, index) => ({
            index,
            remainder: value - Math.floor(value),
            notClamped: true,
          }));
    pool.sort(
      (left, right) =>
        right.remainder - left.remainder || left.index - right.index,
    );
    for (let offset = 0; offset < diff; offset += 1) {
      shares[pool[offset % pool.length].index] += 1;
    }
    diff = 0;
  }

  while (diff < 0) {
    let target = -1;
    for (let index = 0; index < shares.length; index += 1) {
      if (shares[index] <= MIN_SHARE_UNITS) {
        continue;
      }
      if (target === -1 || shares[index] > shares[target]) {
        target = index;
      }
    }
    if (target === -1) {
      break;
    }
    shares[target] -= 1;
    diff += 1;
  }

  return shares;
}

export function sharesToPercents(values: number[]): number[] {
  const total = sumShares(values);
  if (total <= 0) {
    return values.map(() => 0);
  }
  return values.map((value) => (value / total) * 100);
}

/** 只用于“1000 units = 100%”的百分比模式/归一化结果。 */
export function formatUnitsAsPercent(units: number): string {
  const value = Number.isFinite(units) ? (units / SHARE_UNITS) * 100 : 0;
  return `${value.toFixed(1)}%`;
}

export function formatPercentValue(value: number): string {
  return `${(Number.isFinite(value) ? value : 0).toFixed(1)}%`;
}

export function isValidPercentTotal(values: number[]): boolean {
  if (values.length === 0 || values.length > SHARE_UNITS) {
    return false;
  }
  return (
    values.every(
      (value) =>
        Number.isInteger(value) &&
        value >= MIN_SHARE_UNITS &&
        value <= SHARE_UNITS,
    ) && sumShares(values) === SHARE_UNITS
  );
}

/** 输入草稿解析；不做范围夹取，越界由界面校验处理。 */
export function parseShareDraft(
  draft: string,
  mode: ShareInputMode,
): number {
  const value = Number(draft);
  if (!Number.isFinite(value)) {
    return 0;
  }
  return mode === "percent" ? Math.round(value * 10) : Math.round(value);
}

export function formatShareDraft(units: number, mode: ShareInputMode): string {
  const safe = Number.isFinite(units) ? units : 0;
  return mode === "percent"
    ? (safe / 10).toFixed(1)
    : String(Math.round(safe));
}
