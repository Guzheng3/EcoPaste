import { invoke } from "@tauri-apps/api/core";
import { useMount } from "ahooks";
import type { FC } from "react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useTauriListen } from "@/hooks/useTauriListen";

/**
 * 复制成功小气泡窗 `/copied`：由 Rust 侧按需创建独立置顶小窗（`transparent`）加载本页。
 *
 * 生命周期由前端驱动：
 * 1. 页面加载（mount）自动播放一遍；
 * 2. 每次 show 后端广播 `copied://play`，前端据此重播；
 * 3. 按「出现 → 画圆 → 画勾 → 停留 → 淡出」播放，动画结束后 invoke
 *    `hide_copied_toast` 让后端隐藏窗口。
 */
const Copied: FC = () => {
  const { t } = useTranslation("commands");

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

    // 入场：出现 + 画圆 + 画勾（0.9s）
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
    play(++seqRef.current);
  });

  useTauriListen("copied://play", () => {
    play(++seqRef.current);
  });

  // 不再守卫 i18n.isInitialized——窗口在 keepalive 过期后会被销毁重建，
  // 新 WebView 的 i18n 初始化需要时间，若此时 return null 会导致 useMount 的
  // play() 在不可见状态下跑完动画，窗口被提前隐藏。
  // t() 已提供 defaultValue 兜底，无需阻断渲染。
  return (
    <div className="h-screen w-screen overflow-hidden">
      <style>{COPED_CSS}</style>
      <div className="contents" key={replayKey}>
        <div className={`copied-toast copied-toast--${phase}`}>
          <svg aria-hidden="true" className="copied-badge" viewBox="0 0 24 24">
            <circle
              className="copied-ring"
              cx="12"
              cy="12"
              r="10"
              transform="rotate(-90 12 12)"
            />
            <path className="copied-line" d="M7 12.6 L10.8 16.4 L17 8.5" />
          </svg>
          <span className="copied-text">
            {t("copied", { defaultValue: "复制成功" })}
          </span>
        </div>
      </div>
    </div>
  );
};

/** 动画关键帧与视觉样式，与演示稿保持一致（半透明胶囊 + 描边圆/勾 + 平滑绿光晕）。 */
const COPED_CSS = `
 :root {
  --copied-success: #22c55e;
  --copied-bg: #ffffff;
  --copied-text: #3f3f46;
  --copied-shadow: 0 8px 24px rgb(22 163 74 / 0.18);
}
 html, body, #root {
   margin: 0; padding: 0;
   width: 100%; height: 100%;
   overflow: hidden;
   background: transparent;
 }
 html.dark {
   --copied-bg: #141816;
   --copied-text: #dcfce7;
 }

 .copied-toast {
  display: flex; align-items: center; justify-content: center; gap: 10px;
  width: 100%; height: 100%;
  padding: 8px 18px 8px 10px;
  border-radius: 10px;
  background: var(--copied-bg);
  box-shadow: var(--copied-shadow);
  border: none; outline: none;
  font-size: 14px; line-height: 1; font-weight: 700; color: var(--copied-text);
}
 .copied-toast { opacity: 0; transform: scale(.6) translateY(8px); }

 /* 入场：出现 + 绿色光晕柔和扩散再收敛（forwards 保持到淡出） */
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
   0% { box-shadow: 0 0 0 0 rgb(34 197 94 / 0.4); }
   40% { box-shadow: 0 0 30px 4px rgb(34 197 94 / 0.32); }
   100% { box-shadow: var(--copied-shadow); }
 }

 /* 停留：保持可见，无过渡回退，避免显隐闪烁 */
 .copied-toast--showing { opacity: 1; transform: scale(1) translateY(0); }

 /* 淡出：缩小上滑 */
 .copied-toast--exit { animation: copied-pop-out .5s cubic-bezier(.6, 0, .9, .4) forwards; }
 @keyframes copied-pop-out {
   0% { opacity: 1; transform: scale(1) translateY(0); }
   100% { opacity: 0; transform: scale(.75) translateY(-16px); }
 }

 /* 文字：单行渲染，绝不换行（DPI 变化等极端情况下宁可稍宽也不折行） */
 .copied-text { white-space: nowrap; }

 /* 图标：圆环 + 对勾，stroke 逐笔画出 */
 .copied-badge { width: 30px; height: 30px; display: block; flex-shrink: 0; }
 .copied-ring {
   fill: none; stroke: var(--copied-success);
   stroke-width: 2.5; stroke-linecap: round;
   stroke-dasharray: 63; stroke-dashoffset: 0;
 }
 .copied-line {
   fill: none; stroke: var(--copied-success);
   stroke-width: 3; stroke-linecap: round; stroke-linejoin: round;
   stroke-dasharray: 26; stroke-dashoffset: 0;
 }
 /* 入场时置为「未画」，圆环先画（.55s），圆环收笔后再画勾（.32s 衔接） */
 .copied-toast--enter .copied-ring {
   stroke-dashoffset: 63; animation: copied-draw .55s cubic-bezier(.4, .1, .3, 1) forwards;
 }
 .copied-toast--enter .copied-line {
   stroke-dashoffset: 26; animation: copied-draw .32s cubic-bezier(.3, .1, .2, 1) .55s forwards;
 }
 @keyframes copied-draw { to { stroke-dashoffset: 0; } }
`;

export default Copied;
