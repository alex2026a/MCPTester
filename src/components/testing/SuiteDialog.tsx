import { useState, type ReactNode } from "react";
import { useMutation } from "@tanstack/react-query";
import {
  createTestSuite,
  updateTestSuite,
} from "../../api/tests";
import { toast } from "../../stores/toast";
import type { ServerSummary } from "../../types/models";
import type { TestSuiteSummary } from "../../types/testing";

interface Props {
  servers: ServerSummary[];
  suite?: TestSuiteSummary;
  defaultServerId: string;
  onClose: () => void;
  onSaved: (suiteId: string) => void;
}

/** 新建/编辑套件。编辑时不允许改服务器（服务器变更走 rebind，且仅空套件）。 */
export function SuiteDialog({
  servers,
  suite,
  defaultServerId,
  onClose,
  onSaved,
}: Props) {
  const editing = Boolean(suite);
  const [name, setName] = useState(suite?.name ?? "");
  const [description, setDescription] = useState("");
  const [serverId, setServerId] = useState(suite?.server_id ?? defaultServerId);
  const [tags, setTags] = useState("");

  const save = useMutation({
    mutationFn: async () => {
      if (editing && suite) {
        return updateTestSuite(suite.id, { name, description, tags: splitTags(tags) });
      }
      return createTestSuite({
        name,
        description,
        server_id: serverId,
        tags: splitTags(tags),
      });
    },
    onSuccess: (saved) => {
      toast.success(editing ? "套件已更新" : "套件已创建");
      onSaved(saved.id);
      onClose();
    },
    onError: (error) => toast.fromError(error, "保存套件失败"),
  });

  return (
    <Modal title={editing ? "编辑套件" : "新建套件"} onClose={onClose}>
      <div className="space-y-3 text-sm">
        <Field label="名称">
          <input
            className="w-full rounded border border-slate-200 px-2 py-1.5"
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="例如：文件系统工具回归"
          />
        </Field>
        <Field label="描述">
          <textarea
            className="h-20 w-full rounded border border-slate-200 px-2 py-1.5"
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            placeholder="可选"
          />
        </Field>
        <Field label="服务器">
          <select
            className="w-full rounded border border-slate-200 px-2 py-1.5 disabled:bg-slate-50"
            value={serverId}
            disabled={editing}
            onChange={(event) => setServerId(event.target.value)}
          >
            <option value="">请选择服务器</option>
            {servers.map((server) => (
              <option key={server.id} value={server.id}>
                {server.name}
              </option>
            ))}
          </select>
          {editing ? (
            <p className="mt-1 text-[11px] text-slate-400">
              套件已绑定服务器；如需更换，请新建套件（已有用例时不允许直接重绑）。
            </p>
          ) : null}
        </Field>
        <Field label="标签（逗号分隔）">
          <input
            className="w-full rounded border border-slate-200 px-2 py-1.5"
            value={tags}
            onChange={(event) => setTags(event.target.value)}
            placeholder="smoke, filesystem"
          />
        </Field>
        <div className="flex justify-end gap-2 pt-2">
          <button
            type="button"
            className="rounded border border-slate-200 px-3 py-1.5 text-xs hover:bg-slate-50"
            onClick={onClose}
          >
            取消
          </button>
          <button
            type="button"
            className="rounded bg-slate-800 px-3 py-1.5 text-xs text-white hover:bg-slate-700 disabled:opacity-50"
            disabled={save.isPending || !name.trim() || (!editing && !serverId)}
            onClick={() => save.mutate()}
          >
            {save.isPending ? "保存中…" : "保存"}
          </button>
        </div>
      </div>
    </Modal>
  );
}

export function Modal({
  title,
  onClose,
  children,
  wide,
  xwide,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  wide?: boolean;
  xwide?: boolean;
}) {
  return (
    <div className="fixed inset-0 z-40 flex items-start justify-center bg-slate-900/30 p-6">
      <div
        className={`max-h-[90vh] w-full overflow-y-auto rounded-lg bg-white p-4 shadow-xl ${
          xwide
            ? "max-w-[min(92vw,1400px)]"
            : wide
              ? "max-w-5xl"
              : "max-w-xl"
        }`}
      >
        <div className="mb-3 flex items-center justify-between">
          <h3 className="text-base font-semibold text-slate-800">{title}</h3>
          <button
            type="button"
            className="rounded px-2 py-1 text-sm text-slate-400 hover:bg-slate-100"
            onClick={onClose}
          >
            ×
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}

export function Field({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1 block text-xs text-slate-500">{label}</span>
      {children}
    </label>
  );
}

function splitTags(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}
