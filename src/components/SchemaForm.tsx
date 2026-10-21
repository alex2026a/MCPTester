import {
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type Ref,
} from "react";
import {
  isFieldRequired,
  isEmptyValue,
  requiredKeysOf,
  simpleControlType,
  validateAgainstSchema,
  type SchemaObject,
} from "../lib/schema";

export type SchemaValidation = {
  /** 当前表单/JSON 文本对应的参数 */
  value: Record<string, unknown>;
  /** Schema 校验错误（JSON 解析失败时为空） */
  errors: string[];
  /** JSON 模式解析失败信息 */
  parseError: string | null;
};

export type SchemaFormHandle = {
  /** 提交前校验：展示错误、聚焦第一个错误字段，并返回当前参数 */
  validate: () => SchemaValidation;
  /** 只读取当前参数，不触发错误展示（供「仍要发送」使用） */
  readValue: () => SchemaValidation;
};

function parseArguments(text: string): {
  value: Record<string, unknown> | null;
  error: string | null;
} {
  try {
    const parsed = JSON.parse(text || "{}") as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return { value: null, error: "参数必须是 JSON 对象" };
    }
    return { value: parsed as Record<string, unknown>, error: null };
  } catch (e) {
    return { value: null, error: `JSON 解析失败：${String(e)}` };
  }
}

/** 必填空值时的边框/底色 */
function missingClass(missing: boolean): string {
  return missing ? "border-red-300 bg-red-50/40" : "border-slate-200";
}

/** JSON Schema 驱动表单：基础类型自动控件；数组/对象/嵌套降级为 JSON 编辑 */
export function SchemaForm({
  schema,
  onChange,
  initial,
  ref,
}: {
  schema: unknown;
  onChange: (value: Record<string, unknown>) => void;
  /** 预填参数（重放场景） */
  initial?: Record<string, unknown> | null;
  ref?: Ref<SchemaFormHandle>;
}) {
  const root = useMemo(() => (schema ?? {}) as SchemaObject, [schema]);
  const fields = useMemo(() => Object.entries(root.properties ?? {}), [root]);
  const [values, setValues] = useState<Record<string, unknown>>(initial ?? {});
  const [jsonMode, setJsonMode] = useState(false);
  const [jsonText, setJsonText] = useState(
    JSON.stringify(initial ?? {}, null, 2),
  );
  const [jsonError, setJsonError] = useState<string | null>(null);
  const [touched, setTouched] = useState<Set<string>>(() => new Set());
  const [submitted, setSubmitted] = useState(false);
  const [jsonTouched, setJsonTouched] = useState(false);
  const [validateNonce, setValidateNonce] = useState(0);
  const containerRef = useRef<HTMLDivElement | null>(null);

  const requiredNames = useMemo(
    () =>
      fields
        .filter(([key, field]) => isFieldRequired(root, key, field))
        .map(([key]) => key),
    [fields, root],
  );

  const errors = useMemo(() => {
    if (jsonMode && jsonError) return [jsonError];
    return validateAgainstSchema(schema, values);
  }, [jsonError, jsonMode, schema, values]);

  function update(key: string, value: unknown) {
    const next = { ...values, [key]: value };
    setValues(next);
    onChange(next);
  }

  function touch(key: string) {
    setTouched((prev) => {
      if (prev.has(key)) return prev;
      const next = new Set(prev);
      next.add(key);
      return next;
    });
  }

  const readValue = useCallback((): SchemaValidation => {
    if (!jsonMode) {
      return {
        value: values,
        errors: validateAgainstSchema(schema, values),
        parseError: null,
      };
    }
    const parsed = parseArguments(jsonText);
    if (!parsed.value) {
      return { value: {}, errors: [], parseError: parsed.error };
    }
    return {
      value: parsed.value,
      errors: validateAgainstSchema(schema, parsed.value),
      parseError: null,
    };
  }, [jsonMode, jsonText, schema, values]);

  const validate = useCallback((): SchemaValidation => {
    setSubmitted(true);
    setValidateNonce((value) => value + 1);
    return readValue();
  }, [readValue]);

  useImperativeHandle(ref, () => ({ validate, readValue }), [
    readValue,
    validate,
  ]);

  // 提交失败后聚焦第一个错误控件（等本次渲染把 aria-invalid 刷到 DOM 上）
  useEffect(() => {
    if (validateNonce === 0) return;
    const node = containerRef.current?.querySelector<HTMLElement>(
      '[aria-invalid="true"]',
    );
    node?.focus();
    node?.scrollIntoView?.({ block: "nearest" });
  }, [validateNonce]);

  function switchToJson() {
    setJsonText(JSON.stringify(values, null, 2));
    setJsonError(null);
    setJsonTouched(false);
    setSubmitted(false);
    setTouched(new Set());
    setJsonMode(true);
  }

  function switchToForm() {
    const parsed = parseArguments(jsonText);
    if (!parsed.value) {
      setJsonError(parsed.error);
      setJsonTouched(true);
      return;
    }
    setValues(parsed.value);
    onChange(parsed.value);
    setJsonError(null);
    setSubmitted(false);
    setTouched(new Set());
    setJsonMode(false);
  }

  return (
    <div ref={containerRef} className="space-y-3">
      <div className="flex items-center justify-between">
        <span className="text-xs text-slate-400">
          参数
          {requiredNames.length > 0 ? (
            <span className="ml-2 text-red-500">
              {requiredNames.length} 个必填
            </span>
          ) : null}
        </span>
        <button
          type="button"
          className="rounded border border-slate-200 px-2 py-0.5 text-xs text-slate-500 hover:bg-slate-50"
          onClick={jsonMode ? switchToForm : switchToJson}
        >
          {jsonMode ? "表单模式" : "JSON 模式"}
        </button>
      </div>

      {jsonMode ? (
        <div className="space-y-1">
          {requiredNames.length > 0 ? (
            <div className="text-[11px] text-red-500">
              必填字段：{requiredNames.join("、")}
            </div>
          ) : null}
          <textarea
            aria-label="参数 JSON"
            aria-invalid={jsonError ? true : undefined}
            className="h-40 w-full rounded border border-slate-200 p-2 font-mono text-xs"
            value={jsonText}
            onChange={(e) => {
              const text = e.target.value;
              setJsonText(text);
              const parsed = parseArguments(text);
              if (!parsed.value) {
                setJsonError(parsed.error);
                return;
              }
              setJsonError(null);
              setValues(parsed.value);
              onChange(parsed.value);
            }}
            onBlur={() => setJsonTouched(true)}
          />
        </div>
      ) : fields.length === 0 ? (
        <div className="text-xs text-slate-400">该接口无参数</div>
      ) : (
        fields.map(([key, field]) => {
          const required = isFieldRequired(root, key, field);
          const control = simpleControlType(field);
          const current = values[key];
          const missing = required && isEmptyValue(current);
          const showMissing = missing && (submitted || touched.has(key));
          const nestedRequired = requiredKeysOf(field);
          const itemRequired =
            field.type === "array" ? requiredKeysOf(field.items) : [];
          return (
            <div key={key} className="space-y-1">
              <label className="flex flex-wrap items-center gap-1 text-xs text-slate-600">
                {required ? (
                  <span className="text-red-500" aria-hidden="true">
                    *
                  </span>
                ) : null}
                <span
                  className={
                    required ? "font-medium text-slate-800" : undefined
                  }
                >
                  {key}
                </span>
                {required ? (
                  <span className="rounded bg-red-50 px-1 py-0.5 text-[10px] font-medium text-red-600">
                    必填
                  </span>
                ) : null}
                <span className="text-slate-400">{field.type ?? ""}</span>
              </label>
              {field.description ? (
                <div className="text-xs text-slate-400">
                  {field.description}
                </div>
              ) : null}
              {nestedRequired.length > 0 ? (
                <div className="text-[11px] text-red-500">
                  子字段必填：{nestedRequired.join("、")}
                </div>
              ) : null}
              {itemRequired.length > 0 ? (
                <div className="text-[11px] text-red-500">
                  数组元素必填：{itemRequired.join("、")}
                </div>
              ) : null}
              {control === "boolean" ? (
                <span
                  className={
                    showMissing
                      ? "inline-flex rounded border border-red-300 p-0.5"
                      : "inline-flex"
                  }
                >
                  <input
                    type="checkbox"
                    aria-label={key}
                    aria-required={required || undefined}
                    aria-invalid={showMissing || undefined}
                    checked={current === true}
                    onChange={(e) => update(key, e.target.checked)}
                    onBlur={() => touch(key)}
                  />
                </span>
              ) : control === "enum" ? (
                <select
                  aria-label={key}
                  aria-required={required || undefined}
                  aria-invalid={showMissing || undefined}
                  className={`w-full rounded border px-2 py-1 text-sm ${missingClass(showMissing)}`}
                  value={typeof current === "string" ? current : ""}
                  onChange={(e) => update(key, e.target.value)}
                  onBlur={() => touch(key)}
                >
                  <option value="">（请选择）</option>
                  {(field.enum ?? []).map((option) => (
                    <option key={String(option)} value={String(option)}>
                      {String(option)}
                    </option>
                  ))}
                </select>
              ) : control === "number" ? (
                <input
                  type="number"
                  aria-label={key}
                  aria-required={required || undefined}
                  aria-invalid={showMissing || undefined}
                  className={`w-full rounded border px-2 py-1 text-sm ${missingClass(showMissing)}`}
                  value={typeof current === "number" ? current : ""}
                  onChange={(e) =>
                    update(
                      key,
                      e.target.value === ""
                        ? undefined
                        : Number(e.target.value),
                    )
                  }
                  onBlur={() => touch(key)}
                />
              ) : control === "string" ? (
                <input
                  type="text"
                  aria-label={key}
                  aria-required={required || undefined}
                  aria-invalid={showMissing || undefined}
                  className={`w-full rounded border px-2 py-1 text-sm ${missingClass(showMissing)}`}
                  value={typeof current === "string" ? current : ""}
                  onChange={(e) => update(key, e.target.value)}
                  onBlur={() => touch(key)}
                />
              ) : (
                <textarea
                  aria-label={key}
                  aria-required={required || undefined}
                  aria-invalid={showMissing || undefined}
                  placeholder="复杂类型：粘贴 JSON"
                  className={`h-20 w-full rounded border p-2 font-mono text-xs ${missingClass(showMissing)}`}
                  value={
                    typeof current === "string"
                      ? current
                      : JSON.stringify(current ?? "")
                  }
                  onChange={(e) => {
                    try {
                      update(key, JSON.parse(e.target.value));
                    } catch {
                      // 输入过程中允许非 JSON 文本，提交前由校验兜底
                      update(key, e.target.value);
                    }
                  }}
                  onBlur={() => touch(key)}
                />
              )}
              {missing && touched.has(key) && !submitted ? (
                <div className="text-[11px] text-red-500">此参数必填</div>
              ) : null}
            </div>
          );
        })
      )}

      {submitted || (jsonMode && jsonTouched && jsonError) ? (
        errors.length > 0 ? (
          <ul role="alert" className="space-y-0.5 text-xs text-red-500">
            {errors.map((error) => (
              <li key={error}>{error}</li>
            ))}
          </ul>
        ) : null
      ) : null}
    </div>
  );
}
