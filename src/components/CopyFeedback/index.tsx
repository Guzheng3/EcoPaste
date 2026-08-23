import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { AnimatePresence, motion } from "motion/react";
import type { FC } from "react";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useSnapshot } from "valtio";
import { TAURI_EVENT } from "@/constants/events";
import { settingsState } from "@/stores/settings";

/** 单个气泡存在时长（毫秒）。新内容到达时并在其上方叠加，互不影响。 */
const HIDE_AFTER_MS = 1500;

interface FeedbackItem {
  id: number;
}

/**
 * 复制成功小气泡：监听 Rust 发来的 `clipboard://copied`（仅「非去重新入库」时广播）
 * 事件，在右下角弹出一个很小的提示。多个不同内容连发时**并排叠加**，各自倒计时消失；
 * 同一内容重复复制（去重）不触发，由 Rust 侧 `notify_copy_feedback(deduplicated)` 把关。
 */
const CopyFeedback: FC = () => {
  const { t } = useTranslation("commands");
  const settings = useSnapshot(settingsState);
  const enabled = settings.clipboard.feedback.copySound;

  const [items, setItems] = useState<FeedbackItem[]>([]);
  const idRef = useRef(0);
  const timersRef = useRef(new Map<number, number>());

  useEffect(() => {
    if (!enabled) return;

    let disposed = false;
    let unlisten: (() => void) | undefined;

    const pushItem = () => {
      const id = ++idRef.current;
      setItems((prev) => [...prev, { id }]);
      timersRef.current.set(
        id,
        window.setTimeout(() => {
          setItems((prev) => prev.filter((item) => item.id !== id));
          timersRef.current.delete(id);
        }, HIDE_AFTER_MS),
      );
    };

    getCurrentWebviewWindow()
      .listen(TAURI_EVENT.CLIPBOARD_COPIED, () => {
        if (disposed) return;
        pushItem();
      })
      .then((fn) => {
        unlisten = fn;
      });

    return () => {
      disposed = true;
      unlisten?.();
      timersRef.current.forEach((timer) => {
        window.clearTimeout(timer);
      });
      timersRef.current.clear();
    };
  }, [enabled]);

  return createPortal(
    <div className="pointer-events-none fixed right-4 bottom-4 z-[3000] flex w-48 flex-col items-end gap-2">
      <AnimatePresence>
        {items.map((item) => (
          <motion.div
            animate={{ opacity: 1, scale: 1, y: 0 }}
            className="pointer-events-auto flex items-center gap-1.5 rounded-lg border border-ant-border bg-ant-container/95 py-1.5 pr-2.5 pl-2 shadow-lg backdrop-blur"
            exit={{ opacity: 0, scale: 0.94, y: -6 }}
            initial={{ opacity: 0, scale: 0.94, y: 10 }}
            key={item.id}
            transition={{ duration: 0.18, ease: "easeOut" }}
          >
            <i
              aria-hidden="true"
              className="i-lucide:check size-4 shrink-0 text-ant-success"
            />
            <span className="text-ant-text text-sm leading-none">
              {t("copied")}
            </span>
          </motion.div>
        ))}
      </AnimatePresence>
    </div>,
    document.body,
  );
};

export default CopyFeedback;
