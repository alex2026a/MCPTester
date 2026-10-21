/**
 * Markdown 渲染（v0.1.3）
 *
 * 安全边界：不启用 rehype-raw（原始 HTML 不解析）；链接经 react-markdown 的
 * urlTransform 过滤后，仅 http/https 通过系统浏览器打开，不在 WebView 内导航。
 */

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";

export function Markdown({ text, className = "" }: { text: string; className?: string }) {
  return (
    <div className={`max-w-none break-words text-xs leading-6 text-slate-700 ${className}`}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          h1: ({ children }) => (
            <h1 className="mb-1 mt-3 text-sm font-semibold text-slate-800">{children}</h1>
          ),
          h2: ({ children }) => (
            <h2 className="mb-1 mt-2 text-[13px] font-semibold text-slate-800">{children}</h2>
          ),
          h3: ({ children }) => (
            <h3 className="mb-1 mt-2 text-xs font-semibold text-slate-800">{children}</h3>
          ),
          p: ({ children }) => <p className="my-1">{children}</p>,
          ul: ({ children }) => <ul className="my-1 list-disc pl-5">{children}</ul>,
          ol: ({ children }) => <ol className="my-1 list-decimal pl-5">{children}</ol>,
          li: ({ children }) => <li className="my-0.5">{children}</li>,
          blockquote: ({ children }) => (
            <blockquote className="my-2 border-l-2 border-slate-300 pl-3 text-slate-500">
              {children}
            </blockquote>
          ),
          hr: () => <hr className="my-3 border-slate-200" />,
          a: ({ href, children }) => (
            <a
              href={href}
              className="text-blue-600 underline decoration-blue-300 hover:text-blue-700"
              onClick={(event) => {
                event.preventDefault();
                if (href && /^https?:\/\//i.test(href)) {
                  void openUrl(href).catch(() => {});
                }
              }}
            >
              {children}
            </a>
          ),
          code: ({ className, children }) => {
            const fenced = typeof className === "string" && className.includes("language-");
            if (fenced) {
              return <code className="font-mono text-[11px] text-slate-700">{children}</code>;
            }
            return (
              <code className="rounded bg-slate-100 px-1 py-0.5 font-mono text-[11px] text-rose-600">
                {children}
              </code>
            );
          },
          pre: ({ children }) => (
            <pre className="my-2 overflow-auto rounded border border-slate-200 bg-slate-50 p-2 font-mono text-[11px] leading-5 [&_code]:bg-transparent [&_code]:p-0">
              {children}
            </pre>
          ),
          table: ({ children }) => (
            <div className="my-2 overflow-auto">
              <table className="min-w-full border-collapse text-[11px]">{children}</table>
            </div>
          ),
          th: ({ children }) => (
            <th className="border border-slate-200 bg-slate-50 px-2 py-1 text-left font-medium text-slate-600">
              {children}
            </th>
          ),
          td: ({ children }) => (
            <td className="border border-slate-200 px-2 py-1 align-top">{children}</td>
          ),
          strong: ({ children }) => (
            <strong className="font-semibold text-slate-800">{children}</strong>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}
