/**
 * 结构视图的 DOM 寻址（v0.1.3）
 *
 * react18-json-view 不暴露节点路径，但渲染结构稳定：
 *   .json-view--pair > [.json-view--property | .json-view--index] + 值 + .jv-indent > .json-view--pair ...
 * 因此可以从 DOM 反推 JSONPath，用于"悬浮复制路径"与搜索跳转定位。
 */

import type { JsonPath } from "./paths";

export const JSON_HIT_CLASS = "mt-json-hit";

interface PairLabel {
  text: string;
  index: boolean;
}

/** 直接子元素里的键名 / 数组下标标签 */
function pairLabel(pair: Element): PairLabel | null {
  for (const child of Array.from(pair.children)) {
    if (child.classList.contains("json-view--property")) {
      return { text: child.textContent ?? "", index: false };
    }
    if (child.classList.contains("json-view--index")) {
      return { text: child.textContent ?? "", index: true };
    }
  }
  return null;
}

/** 直接子元素里的子节点缩进容器 */
function directIndent(element: Element): Element | null {
  for (const child of Array.from(element.children)) {
    if (child.classList.contains("jv-indent")) return child;
  }
  return null;
}

/** 从节点元素反推路径（root 为 `.json-view` 容器） */
export function pathFromElement(element: Element, root: Element): JsonPath | null {
  let pair: Element | null = element.closest(".json-view--pair");
  if (!pair || !root.contains(pair)) return null;
  const path: JsonPath = [];
  let guard = 0;
  while (pair && pair !== root && guard < 200) {
    guard += 1;
    const label = pairLabel(pair);
    if (!label) break;
    if (label.index) {
      const index = Number(label.text);
      if (!Number.isInteger(index)) break;
      path.unshift(index);
    } else {
      path.unshift(label.text);
    }
    const parentPair: HTMLElement | null = pair.parentElement;
    pair = parentPair?.parentElement?.closest(".json-view--pair") ?? null;
  }
  return path;
}

/** 按路径定位结构视图中的 pair 元素（未渲染/已折叠时返回 null） */
export function findPairByPath(root: Element, path: JsonPath): Element | null {
  let scope: Element = root;
  let pair: Element | null = null;
  for (const segment of path) {
    const indent = directIndent(scope);
    if (!indent) return null;
    pair =
      (Array.from(indent.children).find((child) => {
        if (!child.classList.contains("json-view--pair")) return false;
        const label = pairLabel(child);
        if (!label) return false;
        if (typeof segment === "number") {
          return label.index && Number(label.text) === segment;
        }
        return !label.index && label.text === segment;
      }) as Element | undefined) ?? null;
    if (!pair) return null;
    scope = pair;
  }
  return pair;
}

/** 高亮命中节点（同一容器内只保留一个） */
export function highlightPair(pair: Element, container: Element): void {
  for (const previous of Array.from(container.querySelectorAll(`.${JSON_HIT_CLASS}`))) {
    previous.classList.remove(JSON_HIT_CLASS);
  }
  pair.classList.add(JSON_HIT_CLASS);
}

/** 清除命中高亮 */
export function clearHighlight(container: Element | null | undefined): void {
  if (!container) return;
  for (const previous of Array.from(container.querySelectorAll(`.${JSON_HIT_CLASS}`))) {
    previous.classList.remove(JSON_HIT_CLASS);
  }
}

/** 在滚动容器内居中显示节点（避免带动整页滚动） */
export function scrollPairIntoView(container: HTMLElement, pair: Element): void {
  const containerRect = container.getBoundingClientRect();
  const pairRect = pair.getBoundingClientRect();
  if (pairRect.height === 0) return;
  const delta =
    pairRect.top - containerRect.top - container.clientHeight / 2 + pairRect.height / 2;
  container.scrollTop = Math.max(0, container.scrollTop + delta);
}
