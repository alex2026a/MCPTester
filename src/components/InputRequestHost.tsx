/**
 * 交互输入 Host（v0.6.0）
 *
 * - 顶层监听 `mcp://input`；
 * - 挂载时恢复 HMR/刷新前未处理的输入；
 * - 所有 pending 对话框保持挂载，最小化只隐藏。
 */

import { useEffect } from "react";
import { cancelInput, listPendingInputs, listenInputEvents, respondInput } from "../api/input";
import { InputRequestDialog } from "./InputRequestDialog";
import { useInputRequests } from "../stores/inputRequests";
import type { InputAnswer } from "../types/input";

export function InputRequestHost() {
  const pending = useInputRequests((state) => state.pending);
  const activeId = useInputRequests((state) => state.activeId);
  const open = useInputRequests((state) => state.open);
  const close = useInputRequests((state) => state.close);
  const minimize = useInputRequests((state) => state.minimize);
  const restore = useInputRequests((state) => state.restore);

  useEffect(() => {
    let disposed = false;
    void listPendingInputs()
      .then((events) => {
        if (disposed) return;
        for (const event of events) {
          if (event.type === "opened") open(event);
        }
      })
      .catch(() => {});
    const subscription = listenInputEvents((event) => {
      if (event.type === "opened") {
        open(event);
      } else {
        close(event.request_id, event.reason);
      }
    });
    return () => {
      disposed = true;
      subscription
        .then((unlisten) => unlisten())
        .catch(() => {});
    };
  }, [open, close]);

  const minimized = pending.filter((item) => item.minimized);

  async function handleRespond(requestId: string, answers: InputAnswer[]) {
    await respondInput(requestId, answers);
    close(requestId, "answered");
  }

  async function handleCancel(requestId: string) {
    await cancelInput(requestId);
    close(requestId, "canceled");
  }

  return (
    <>
      {pending.map((item) => (
        <InputRequestDialog
          key={item.requestId}
          item={item}
          visible={activeId === item.requestId}
          onRespond={(answers) => handleRespond(item.requestId, answers)}
          onCancel={() => handleCancel(item.requestId)}
          onMinimize={() => minimize(item.requestId)}
        />
      ))}
      {minimized.length > 0 ? (
        <button
          type="button"
          className="fixed bottom-5 right-5 z-40 rounded-full bg-amber-600 px-4 py-2 text-xs font-medium text-white shadow-lg hover:bg-amber-500"
          onClick={() => restore(minimized[0].requestId)}
        >
          待输入 {minimized.length}
        </button>
      ) : null}
    </>
  );
}
