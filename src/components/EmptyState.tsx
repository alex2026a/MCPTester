/** 空态展示组件：页面占位与异常态统一使用 */
export function EmptyState({
  title,
  description,
}: {
  title: string;
  description?: string;
}) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
      <div className="text-base font-medium text-slate-600">{title}</div>
      {description ? (
        <div className="max-w-md text-sm text-slate-400">{description}</div>
      ) : null}
    </div>
  );
}
