/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        // 连接状态色（设计文档 01-需求与范围 7.2）
        conn: {
          ok: "#22c55e",
          connecting: "#3b82f6",
          degraded: "#eab308",
          lost: "#ef4444",
          idle: "#94a3b8",
        },
      },
    },
  },
  plugins: [],
};
