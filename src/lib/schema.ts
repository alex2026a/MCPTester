/** JSON Schema 基础校验（子集：required / type / enum；复杂关键字交由服务端） */

export type SchemaObject = {
  type?: string;
  properties?: Record<string, SchemaObject>;
  /** 标准 JSON Schema 为根级数组；兼容部分服务端在字段上使用 boolean 的写法 */
  required?: string[] | boolean;
  /** false = 明确禁止额外字段；缺省/true/对象 = 允许 */
  additionalProperties?: boolean | SchemaObject;
  enum?: unknown[];
  description?: string;
  items?: SchemaObject;
};

/** 字段是否必填（根级 required 数组 + 字段级 required: true） */
export function isFieldRequired(
  root: SchemaObject,
  key: string,
  field: SchemaObject,
): boolean {
  return (
    (Array.isArray(root.required) && root.required.includes(key)) ||
    field.required === true
  );
}

/** 读取嵌套 schema 的必填字段名（用于复杂类型提示） */
export function requiredKeysOf(schema: SchemaObject | undefined): string[] {
  return schema && Array.isArray(schema.required) ? schema.required : [];
}

/** 必填判空：undefined / null / 空字符串 */
export function isEmptyValue(value: unknown): boolean {
  return value === undefined || value === null || value === "";
}

export function validateAgainstSchema(
  schema: unknown,
  value: Record<string, unknown>,
): string[] {
  const errors: string[] = [];
  if (!schema || typeof schema !== "object") return errors;
  const root = schema as SchemaObject;
  const requiredKeys = new Set<string>(
    Array.isArray(root.required) ? root.required : [],
  );
  for (const [key, field] of Object.entries(root.properties ?? {})) {
    if (field.required === true) requiredKeys.add(key);
  }
  for (const key of requiredKeys) {
    if (isEmptyValue(value[key])) {
      errors.push(`缺少必填参数「${key}」`);
    }
  }
  for (const [key, field] of Object.entries(root.properties ?? {})) {
    const v = value[key];
    if (isEmptyValue(v)) continue;
    switch (field.type) {
      case "string":
        if (typeof v !== "string") errors.push(`参数「${key}」应为字符串`);
        break;
      case "number":
      case "integer":
        if (typeof v !== "number") errors.push(`参数「${key}」应为数字`);
        break;
      case "boolean":
        if (typeof v !== "boolean") errors.push(`参数「${key}」应为布尔值`);
        break;
      case "object":
        if (typeof v !== "object" || Array.isArray(v))
          errors.push(`参数「${key}」应为对象`);
        break;
      case "array":
        if (!Array.isArray(v)) errors.push(`参数「${key}」应为数组`);
        break;
      default:
        break;
    }
    if (field.enum && !field.enum.includes(v)) {
      errors.push(`参数「${key}」必须是以下值之一：${field.enum.join(" / ")}`);
    }
  }
  return errors;
}

/** 历史参数回填时的 Schema 漂移处理（只处理顶层字段）。 */
export function filterKnownTopLevel(
  schema: unknown,
  params: Record<string, unknown>,
): {
  value: Record<string, unknown>;
  dropped: number;
  unknown: number;
  disallowsAdditional: boolean;
} {
  const root =
    schema && typeof schema === "object" ? (schema as SchemaObject) : null;
  const properties = root?.properties;
  const paramsKeys = Object.keys(params);

  if (!properties) {
    return {
      value: params,
      dropped: 0,
      unknown: 0,
      disallowsAdditional: false,
    };
  }

  const propertyNames = Object.keys(properties);
  if (propertyNames.length === 0) {
    if (root?.additionalProperties === false && paramsKeys.length > 0) {
      return {
        value: params,
        dropped: 0,
        unknown: paramsKeys.length,
        disallowsAdditional: true,
      };
    }
    return {
      value: params,
      dropped: 0,
      unknown: 0,
      disallowsAdditional: false,
    };
  }

  if (root?.additionalProperties === false) {
    const value = Object.fromEntries(
      Object.entries(params).filter(([key]) => key in properties),
    );
    return {
      value,
      dropped: paramsKeys.length - Object.keys(value).length,
      unknown: 0,
      disallowsAdditional: false,
    };
  }

  return {
    value: params,
    dropped: 0,
    unknown: paramsKeys.filter((key) => !(key in properties)).length,
    disallowsAdditional: false,
  };
}

/** 字段类型是否支持自动控件（复杂类型降级为 JSON 编辑） */
export function simpleControlType(
  field: SchemaObject,
): "string" | "number" | "boolean" | "enum" | null {
  if (field.enum && field.enum.length > 0) return "enum";
  switch (field.type) {
    case "string":
      return "string";
    case "number":
    case "integer":
      return "number";
    case "boolean":
      return "boolean";
    default:
      return null;
  }
}
