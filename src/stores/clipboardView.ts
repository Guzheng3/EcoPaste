import { proxy, subscribe } from "valtio";
import type { ClipboardCategory, ClipboardRange } from "@/types/clipboard";

interface ClipboardViewState {
  category: ClipboardCategory | null;
  keyword: string;
  groupId: string | null;
  range: ClipboardRange;
}

/**
 * 剪贴板窗口的 UI 临时状态。
 * `range`（含"收藏"范围开关）持久化到 localStorage，保证窗口关闭/WebView 重建后重开时
 * 仍保持上次的选择；其余字段为会话态，不持久化。
 * 跨组件共享：Header 搜索框写入 `keyword`，Group 写入范围/分类/分组，List 监听后驱动查询。
 * 注意：这里的字段会被 List 用 `...rest` 透传成查询参数，**不要**塞进与 `ClipboardItemQuery` 同名
 * 但语义不同的字段（例如「窗口是否固定」要另起 store，否则会被当成 `pinned`(条目置顶) 过滤）。
 * `limit` / `offset` 不在这里——分页由 `useClipboardItems` 内部 `useInfiniteScroll` 管理。
 */
const RANGE_STORAGE_KEY = "clipboardView.range";

const isValidRange = (value: unknown): value is ClipboardRange =>
  value === "all" || value === "favorite";

const loadInitialRange = (): ClipboardRange => {
  try {
    const stored = localStorage.getItem(RANGE_STORAGE_KEY);
    if (stored && isValidRange(stored)) return stored;
  } catch {
    /* 忽略 localStorage 读取失败 */
  }

  return "all";
};

export const clipboardViewState = proxy<ClipboardViewState>({
  category: null,
  groupId: null,
  keyword: "",
  range: loadInitialRange(),
});

subscribe(clipboardViewState, () => {
  try {
    localStorage.setItem(
      RANGE_STORAGE_KEY,
      JSON.stringify(clipboardViewState.range),
    );
  } catch {
    /* 忽略 localStorage 写入失败 */
  }
});
