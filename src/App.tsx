import { useEffect } from "react";
import { NavLink, Route, Routes, HashRouter } from "react-router-dom";
import { QueryClientProvider, useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { queryClient } from "./lib/queryClient";
import { getAppInfo } from "./api/app";
import { Servers } from "./pages/Servers";
import { Explorer } from "./pages/Explorer";
import { History } from "./pages/History";
import { SettingsPage } from "./pages/Settings";
import { ToastContainer } from "./components/Toast";
import { zh } from "./i18n/zh";

/** 监听 Rust 侧事件：连接状态变化驱动服务器列表刷新 */
function TauriEvents() {
  const client = useQueryClient();
  useEffect(() => {
    const subscription = listen("mcp://connection-state", () => {
      void client.invalidateQueries({ queryKey: ["servers"] });
    });
    return () => {
      subscription.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [client]);
  return null;
}

/** 侧栏底部版本号（M1：打通 IPC 链路的最小展示） */
function VersionBadge() {
  const appInfo = useQuery({ queryKey: ["appInfo"], queryFn: getAppInfo });
  if (!appInfo.data) return null;
  return (
    <div className="px-1 text-xs text-slate-400">v{appInfo.data.version}</div>
  );
}

function NavItem({ to, label }: { to: string; label: string }) {
  return (
    <NavLink
      to={to}
      className={({ isActive }) =>
        `block rounded-md px-3 py-2 text-sm ${
          isActive
            ? "bg-slate-800 text-white"
            : "text-slate-600 hover:bg-slate-100"
        }`
      }
    >
      {label}
    </NavLink>
  );
}

function Layout() {
  return (
    <div className="flex h-full">
      <aside className="flex w-44 shrink-0 flex-col border-r border-slate-200 bg-white p-4">
        <div className="mb-6 px-1">
          <h1 className="text-lg font-semibold text-slate-800">
            {zh.appName}
          </h1>
          <p className="text-xs text-slate-400">{zh.appSlogan}</p>
        </div>
        <nav className="space-y-1">
          <NavItem to="/" label={zh.nav.servers} />
          <NavItem to="/explorer" label={zh.nav.explorer} />
          <NavItem to="/history" label={zh.nav.history} />
          <NavItem to="/settings" label={zh.nav.settings} />
        </nav>
        <div className="mt-auto pt-4">
          <VersionBadge />
        </div>
      </aside>
      <main className="flex-1 overflow-y-auto bg-slate-50 p-6">
        <Routes>
          <Route path="/" element={<Servers />} />
          <Route path="/explorer" element={<Explorer />} />
          <Route path="/history" element={<History />} />
          <Route path="/settings" element={<SettingsPage />} />
        </Routes>
      </main>
    </div>
  );
}

function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <HashRouter>
        <TauriEvents />
        <Layout />
        <ToastContainer />
      </HashRouter>
    </QueryClientProvider>
  );
}

export default App;
