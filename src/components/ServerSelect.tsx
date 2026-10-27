import { groupServers, serverOptionLabel } from "../lib/serverOrder";
import type { ServerSummary } from "../types/models";

interface ServerSelectProps {
  servers: ServerSummary[];
  value: string;
  onChange: (id: string) => void;
  /** 空选项文本（如“全部服务器”“选择服务器”）；不传则没有空选项 */
  placeholder?: string;
  /** 是否按状态分组（默认 true）；历史筛选等场景可关闭 */
  groupByStatus?: boolean;
  className?: string;
  ariaLabel?: string;
  disabled?: boolean;
  /** 是否允许选择 enabled=false 的服务器（历史筛选等场景） */
  allowDisabled?: boolean;
}

const DEFAULT_CLASS = "rounded border border-slate-200 px-2 py-1 text-xs";

/** 服务器选择器：状态字形前置 + 原生 optgroup（已连接优先） */
export function ServerSelect({
  servers,
  value,
  onChange,
  placeholder,
  groupByStatus = true,
  className,
  ariaLabel = "选择服务器",
  disabled,
  allowDisabled = false,
}: ServerSelectProps) {
  const groups = groupByStatus ? groupServers(servers) : null;

  return (
    <select
      className={className ?? DEFAULT_CLASS}
      value={value}
      onChange={(event) => onChange(event.target.value)}
      aria-label={ariaLabel}
      disabled={disabled}
    >
      {placeholder ? <option value="">{placeholder}</option> : null}
      {groups
        ? groups.map((group) => (
            <optgroup
              key={group.key}
              label={`${group.label}（${group.servers.length}）`}
            >
              {group.servers.map((server) => (
                <option
                  key={server.id}
                  value={server.id}
                  disabled={!server.enabled && !allowDisabled}
                >
                  {serverOptionLabel(server, true)}
                </option>
              ))}
            </optgroup>
          ))
        : servers.map((server) => (
            <option
              key={server.id}
              value={server.id}
              disabled={!server.enabled && !allowDisabled}
            >
              {serverOptionLabel(server, false)}
            </option>
          ))}
    </select>
  );
}
