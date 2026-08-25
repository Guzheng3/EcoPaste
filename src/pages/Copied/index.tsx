import { invoke } from "@tauri-apps/api/core";
import { useMount } from "ahooks";
import type { FC } from "react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useTauriListen } from "@/hooks/useTauriListen";
import { log } from "@/utils/log";

type ToastVariant = "success" | "duplicate";

interface PlayPayload {
  variant: ToastVariant;
}

/**
 * 复制反馈小气泡窗 `/copied`：由 Rust 侧按需创建独立置顶小窗（`transparent`）加载本页。
 *
 * 支持两种变体：
 * - `success`：绿色圆环 + 对勾 + "复制成功"（新内容入库）
 * - `duplicate`：红色圆环 + 箭头 + "已复制"（去重命中）
 *
 * 生命周期由前端驱动：
 * 1. 页面加载（mount）自动播放一遍（默认 success）；
 * 2. 每次 show 后端广播 `copied://play`（携带 variant），前端据此重播；
 * 3. 按「出现 → 画圆 → 画图标 → 停留 → 淡出」播放，动画结束后 invoke
 *    `hide_copied_toast` 让后端隐藏窗口。
 */
const Copied: FC = () => {
  const { t } = useTranslation("commands");

  const [variant, setVariant] = useState<ToastVariant>("success");
  const [phase, setPhase] = useState<"enter" | "showing" | "exit">("enter");
  /** 单调递增的播放序号，用于让过期的「重播」回调自失效。 */
  const seqRef = useRef(0);
  const timerRef = useRef<number[]>([]);
  /** 播放批次：每次 play 递增并以之为 DOM key，强制重建节点以可靠重放 CSS 动画。 */
  const [replayKey, setReplayKey] = useState(0);

  const clearTimers = () => {
    timerRef.current.forEach((id) => {
      window.clearTimeout(id);
    });
    timerRef.current = [];
  };

  const schedule = (fn: () => void, ms: number) => {
    const id = window.setTimeout(() => {
      timerRef.current = timerRef.current.filter((v) => v !== id);
      fn();
    }, ms);
    timerRef.current.push(id);
  };

  /**
   * 播放完整动画一次。`seq` 用于丢弃过期的播放（新的 show 到来时旧回调不再生效）。
   */
  const play = (seq: number, v: ToastVariant) => {
    clearTimers();
    setVariant(v);
    setPhase("enter");
    setReplayKey((k) => k + 1);

    // 入场：出现 + 画圆 + 画图标（0.9s）
    schedule(() => {
      if (seq === seqRef.current) setPhase("showing");
    }, 900);
    // 停留（0.52s）后进入淡出（0.5s），淡出结束后通知后端隐藏
    schedule(() => {
      if (seq === seqRef.current) {
        setPhase("exit");
        schedule(() => {
          if (seq === seqRef.current) {
            void invoke("hide_copied_toast");
          }
        }, 500);
      }
    }, 900 + 520);
  };

  useMount(() => {
    play(++seqRef.current, "success");
  });

  useTauriListen<PlayPayload>("copied://play", (event) => {
    log.debug(`[copied] play event received: variant=${event.payload.variant}`);
    play(++seqRef.current, event.payload.variant);
  });

  const isDup = variant === "duplicate";

  return (
    <div className="h-screen w-screen overflow-hidden">
      <style>{TOAST_CSS}</style>
      <div className="contents" key={replayKey}>
        <div
          className={`copied-toast copied-toast--${phase}${isDup ? "copied-toast--dup" : ""}`}
        >
          <svg aria-hidden="true" className="copied-badge" viewBox="0 0 24 24">
            <circle
              className="copied-ring"
              cx="12"
              cy="12"
              r="10"
              transform="rotate(-90 12 12)"
            />
            {isDup ? (
              <path className="copied-line" d="M10 12h6M13 9l3 3-3 3" />
            ) : (
              <path className="copied-line" d="M7 12.6 L10.8 16.4 L17 8.5" />
            )}
          </svg>
          <span className="copied-text">
            {isDup
              ? t("copiedDup", { defaultValue: "已复制" })
              : t("copied", { defaultValue: "复制成功" })}
          </span>
        </div>
      </div>
    </div>
  );
};

/** 动画关键帧与视觉样式，支持绿/红双模式切换。 */
const TOAST_CSS = `
 :root {
  --toast-color: #22c55e;
  --toast-bg: #ffffff;
  --toast-text: #3f3f46;
  --toast-shadow: 0 8px 24px rgb(22 163 74 / 0.18);
  --toast-glow: rgb(34 197 94 / 0.4);
  --toast-glow-mid: rgb(34 197 94 / 0.32);
}
 html, body, #root {
   margin: 0; padding: 0;
   width: 100%; height: 100%;
   overflow: hidden;
   background: transparent;
 }
 html.dark {
   --toast-bg: #141816;
   --toast-text: #dcfce7;
 }
 html.dark .copied-toast--dup {
   --toast-bg: #1a1414;
   --toast-text: #fecaca;
 }

 /* 红色变体覆盖色值 */
 .copied-toast--dup {
   --toast-color: #ef4444;
   --toast-shadow: 0 8px 24px rgb(239 68 68 / 0.18);
   --toast-glow: rgb(239 68 68 / 0.4);
   --toast-glow-mid: rgb(239 68 68 / 0.32);
 }

 .copied-toast {
  display: flex; align-items: center; justify-content: center; gap: 10px;
  width: 100%; height: 100%;
  padding: 8px 18px 8px 10px;
  border-radius: 10px;
  background: var(--toast-bg);
  box-shadow: var(--toast-shadow);
  border: none; outline: none;
  font-size: 14px; line-height: 1; font-weight: 700; color: var(--toast-text);
}
 .copied-toast { opacity: 0; transform: scale(.6) translateY(8px); }

 /* 入场：出现 + 光晕柔和扩散再收敛 */
 .copied-toast--enter {
   animation: copied-pop-in .45s cubic-bezier(.2, .9, .3, 1.25) forwards,
     copied-glow 1.4s ease-out forwards;
 }
 @keyframes copied-pop-in {
   0% { opacity: 0; transform: scale(.6) translateY(8px); }
   60% { opacity: 1; transform: scale(1.06) translateY(0); }
   100% { opacity: 1; transform: scale(1) translateY(0); }
 }
 @keyframes copied-glow {
   0% { box-shadow: 0 0 0 0 var(--toast-glow); }
   40% { box-shadow: 0 0 30px 4px var(--toast-glow-mid); }
   100% { box-shadow: var(--toast-shadow); }
 }

 /* 停留：保持可见 */
 .copied-toast--showing { opacity: 1; transform: scale(1) translateY(0); }

 /* 淡出：缩小上滑 */
 .copied-toast--exit { animation: copied-pop-out .5s cubic-bezier(.6, 0, .9, .4) forwards; }
 @keyframes copied-pop-out {
   0% { opacity: 1; transform: scale(1) translateY(0); }
   100% { opacity: 0; transform: scale(.75) translateY(-16px); }
 }

 /* 图标 */
 .copied-badge { width: 30px; height: 30px; display: block; flex-shrink: 0; }
 .copied-ring {
   fill: none; stroke: var(--toast-color);
   stroke-width: 2.5; stroke-linecap: round;
   stroke-dasharray: 63; stroke-dashoffset: 0;
 }
 .copied-line {
   fill: none; stroke: var(--toast-color);
   stroke-width: 3; stroke-linecap: round; stroke-linejoin: round;
   stroke-dasharray: 26; stroke-dashoffset: 0;
 }
 /* 红色箭头需要更长的 stroke-dasharray */
 .copied-toast--dup .copied-line {
   stroke-dasharray: 30;
 }
 /* 入场时置为「未画」，圆环先画（.55s），圆环收笔后再画图标（.32s 衔接） */
 .copied-toast--enter .copied-ring {
   stroke-dashoffset: 63; animation: copied-draw .55s cubic-bezier(.4, .1, .3, 1) forwards;
 }
 .copied-toast--enter .copied-line {
   stroke-dashoffset: 26; animation: copied-draw .32s cubic-bezier(.3, .1, .2, 1) .55s forwards;
 }
 .copied-toast--enter.copied-toast--dup .copied-line {
   stroke-dashoffset: 30;
 }
 @keyframes copied-draw { to { stroke-dashoffset: 0; } }
`;

export default Copied;
