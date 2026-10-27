/**
 * 敏感字段检测（v0.4.2）
 *
 * 用于复制菜单提示；键名采用精确/白名单，避免把 `max_tokens`、`token_count`
 * 这类模型参数误报成凭据。
 */

export interface SensitiveHit {
  /** JSONPath，如 `$.arguments.token` */
  path: string;
  /** 命中的键名 */
  key: string;
}

const SENSITIVE_KEYS = new Set([
  "authorization",
  "proxyauthorization",
  "cookie",
  "cookies",
  "setcookie",
  "token",
  "accesstoken",
  "refreshtoken",
  "idtoken",
  "authtoken",
  "apitoken",
  "xauthtoken",
  "xaccesstoken",
  "xapikey",
  "apikey",
  "secret",
  "clientsecret",
  "password",
  "credential",
  "credentials",
]);

const SECRET_VALUE_PATTERNS = [
  /^Bearer\s+\S+/i,
  /^sk-[A-Za-z0-9_-]{8,}$/,
  /^ghp_[A-Za-z0-9]{20,}$/,
  /^github_pat_[A-Za-z0-9_]{20,}$/,
  /^xox[bp]-[A-Za-z0-9-]{10,}$/,
  /^eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/,
];

function normalizeKey(key: string): string {
  return key.replace(/[^a-z0-9]/gi, "").toLowerCase();
}

function isSensitiveKey(key: string): boolean {
  return SENSITIVE_KEYS.has(normalizeKey(key));
}

export function looksLikeSecret(value: string): boolean {
  const text = value.trim();
  return SECRET_VALUE_PATTERNS.some((pattern) => pattern.test(text));
}

function childPath(path: string, key: string): string {
  return `${path}.${key}`;
}

function indexPath(path: string, index: number): string {
  return `${path}[${index}]`;
}

/**
 * 递归查找敏感字段。
 *
 * - 键名命中优先记一次，不再重复按值记；
 * - 数组按 `[i]` 记录路径；
 * - 只返回命中，不修改原值。
 */
export function findSensitiveFields(value: unknown): SensitiveHit[] {
  const hits: SensitiveHit[] = [];

  function visit(node: unknown, path: string): void {
    if (Array.isArray(node)) {
      node.forEach((item, index) => visit(item, indexPath(path, index)));
      return;
    }
    if (node === null || typeof node !== "object") return;

    for (const [key, item] of Object.entries(node as Record<string, unknown>)) {
      const nextPath = childPath(path, key);
      if (isSensitiveKey(key)) {
        hits.push({ path: nextPath, key });
        continue;
      }
      if (typeof item === "string" && looksLikeSecret(item)) {
        hits.push({ path: nextPath, key });
        continue;
      }
      visit(item, nextPath);
    }
  }

  visit(value, "$");
  return hits;
}

/** 深拷贝并替换常见敏感字段；返回替换数量 */
export function redactSensitive(value: unknown): {
  value: unknown;
  count: number;
} {
  let count = 0;

  function visit(node: unknown): unknown {
    if (Array.isArray(node)) {
      return node.map(visit);
    }
    if (node === null || typeof node !== "object") {
      if (typeof node === "string" && looksLikeSecret(node)) {
        count += 1;
        return "****";
      }
      return node;
    }

    const output = Object.create(null) as Record<string, unknown>;
    for (const [key, item] of Object.entries(node as Record<string, unknown>)) {
      if (isSensitiveKey(key)) {
        output[key] = "****";
        count += 1;
        continue;
      }
      if (typeof item === "string" && looksLikeSecret(item)) {
        output[key] = "****";
        count += 1;
        continue;
      }
      output[key] = visit(item);
    }
    return output;
  }

  return { value: visit(value), count };
}
