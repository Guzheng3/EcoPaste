import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useMount } from "ahooks";
import type { FC } from "react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useTauriListen } from "@/hooks/useTauriListen";

/**
 * 重复复制「已复制」小气泡窗 `/copied-dup`：由 Rust 侧按需创建独立置顶小窗（`transparent`）
 * 加载本页，红色圆圈 + 箭头，文案「已复制」，区别于绿色「复制成功」。
 *
 * 与 `/copied` 同构：生命周期由前端驱动——
 * 1. 页面加载（mount）自动播放一遍；
 * 2. 每次 show 后端广播 `copied-dup://play`，前端据此重播；
 * 3. 按「出现 → 画圆 → 画箭头 → 停留 → 淡出」播放，动画结束后 invoke
 *    `hide_copied_dup_toast`（携带本窗口 label）让后端精确隐藏对应实例。
 */
const CopiedDup: FC = () => {
  const { t } = useTranslation("commands");
  const label = getCurrentWebviewWindow().label;

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
  const play = (seq: number) => {
    // 卸载尚未清理的旧 timer，并按批次递增 key 强制重建节点重放动画
    clearTimers();
    setPhase("enter");
    setReplayKey((k) => k + 1);

    // 入场：出现 + 画圆 + 画箭头（0.9s）
    schedule(() => {
      if (seq === seqRef.current) setPhase("showing");
    }, 900);
    // 停留（0.52s）后进入淡出（0.5s），淡出结束后通知后端隐藏
    schedule(() => {
      if (seq === seqRef.current) {
        setPhase("exit");
        schedule(() => {
          if (seq === seqRef.current) {
            void invoke("hide_copied_dup_toast", { label });
          }
        }, 500);
      }
    }, 900 + 520);
  };

  useMount(() => {
    play(++seqRef.current);
  });

  useTauriListen("copied-dup://play", () => {
    play(++seqRef.current);
  });

  return (
    <div className="h-screen w-screen overflow-hidden">
      <style>{COPED_DUP_CSS}</style>
      <div className="contents" key={replayKey}>
        <div className={`copied-dup copied-dup--${phase}`}>
          <svg aria-hidden="true" className="dup-badge" viewBox="0 0 24 24">
            <circle
              className="dup-ring"
              cx="12"
              cy="12"
              r="10"
              transform="rotate(-90 12 12)"
            />
            <path className="dup-arrow" d="M10 12h6M13 9l3 3-3 3" />
          </svg>
          <span className="dup-text">
            {t("copiedDup", { defaultValue: "已复制" })}
          </span>
        </div>
      </div>
    </div>
  );
};

/** 动画关键帧与视觉样式：半透明胶囊 + 红色描边圆/箭头 + 柔和红光晕，窗口 128×48 紧贴内容。 */
const COPED_DUP_CSS = `
 :root {
   --dup-color: #ef4444;
   --dup-bg: #ffffff;
   --dup-text: #3f3f46;
   --dup-shadow: 0 8px 24px rgb(239 68 68 / 0.18);
 }
 html, body, #root {
   margin: 0; padding: 0;
   width: 100%; height: 100%;
   overflow: hidden;
   background: transparent;
 }
 html.dark {
   --dup-bg: #1a1414;
   --dup-text: #fecaca;
 }

 .copied-dup {
  display: flex; align-items: center; justify-content: center; gap: 10px;
  width: 100%; height: 100%;
  padding: 8px 18px 8px 10px;
  border-radius: 10px;
  background: var(--dup-bg);
  box-shadow: var(--dup-shadow);
  border: none; outline: none;
  font-size: 14px; line-height: 1; font-weight: 700; color: var(--dup-text);
}
 .copied-dup { opacity: 0; transform: scale(.6) translateY(8px); }

 /* 入场：出现 + 红色光晕柔和扩散再收敛（forwards 保持到淡出） */
 .copied-dup--enter {
   animation: dup-pop-in .45s cubic-bezier(.2, .9, .3, 1.25) forwards,
     dup-glow 1.4s ease-out forwards;
 }
 @keyframes dup-pop-in {
   0% { opacity: 0; transform: scale(.6) translateY(8px); }
   60% { opacity: 1; transform: scale(1.06) translateY(0); }
   100% { opacity: 1; transform: scale(1) translateY(0); }
 }
 @keyframes dup-glow {
   0% { box-shadow: 0 0 0 0 rgb(239 68 68 / 0.4); }
   40% { box-shadow: 0 0 30px 4px rgb(239 68 68 / 0.32); }
   100% { box-shadow: var(--dup-shadow); }
 }

 /* 停留：保持可见，无过渡回退，避免显隐闪烁 */
 .copied-dup--showing { opacity: 1; transform: scale(1) translateY(0); }

 /* 淡出：缩小上滑 */
 .copied-dup--exit { animation: dup-pop-out .5s cubic-bezier(.6, 0, .9, .4) forwards; }
 @keyframes dup-pop-out {
   0% { opacity: 1; transform: scale(1) translateY(0); }
   100% { opacity: 0; transform: scale(.75) translateY(-16px); }
 }

 /* 图标：圆环 + 箭头，stroke 逐笔画出 */
 .dup-badge { width: 30px; height: 30px; display: block; flex-shrink: 0; }
 .dup-ring {
   fill: none; stroke: var(--dup-color);
   stroke-width: 2.5; stroke-linecap: round;
   stroke-dasharray: 63; stroke-dashoffset: 0;
 }
 .dup-arrow {
   fill: none; stroke: var(--dup-color);
   stroke-width: 3; stroke-linecap: round; stroke-linejoin: round;
   stroke-dasharray: 30; stroke-dashoffset: 0;
 }
 /* 入场时置为「未画」，圆环先画（.55s），圆环收笔后再画箭头（.32s 衔接） */
 .copied-dup--enter .dup-ring {
   stroke-dashoffset: 63; animation: dup-draw .55s cubic-bezier(.4, .1, .3, 1) forwards;
 }
 .copied-dup--enter .dup-arrow {
   stroke-dashoffset: 30; animation: dup-draw .32s cubic-bezier(.3, .1, .2, 1) .55s forwards;
 }
 @keyframes dup-draw { to { stroke-dashoffset: 0; } }
`;

export default CopiedDup;
