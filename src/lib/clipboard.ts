/** 写入系统剪贴板；不可用时抛出可读错误（Tauri WebView 与浏览器共用）。 */
export async function writeClipboardText(text: string): Promise<void> {
  const clipboard = navigator.clipboard;
  if (!clipboard?.writeText) {
    throw new Error("当前环境不支持剪贴板写入");
  }
  await clipboard.writeText(text);
}
