import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useEventListener, useMount } from "ahooks";
import type { ConfigProviderProps } from "antd";
import { App as AntdApp, ConfigProvider } from "antd";
import enUS from "antd/locale/en_US";
import zhCN from "antd/locale/zh_CN";
import type { FC } from "react";
import { use, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { RouterProvider } from "react-router";
import { useSnapshot } from "valtio";
import { notifyWindowReady } from "@/commands";
import { WINDOW_LABEL } from "@/constants/windows";
import { useAppTheme } from "@/hooks/useAppTheme";
import { router } from "./router";
import { settingsReady, settingsState } from "./stores/settings";
import "./stores/windowLifecycle";
import type { Language } from "./types/settings";
import { setMessageApi, setModalApi } from "./utils/feedback";
import { log } from "./utils/log";

const ANTD_MODAL_CONFIG = {
  centered: true,
} satisfies ConfigProviderProps["modal"];

/**
 * 把设置语言映射到 Ant Design 内置 locale。
 */
const resolveAntdLocale = (language: Language) => {
  if (language === "en-US") return enUS;

  return zhCN;
};

/**
 * 把 0-100 的透光度转换为 0-1 之间的 alpha 值，供 `--glass-tint-alpha` 写入。
 * 与 Rust `opacity_to_alpha` 的反向关系：透光度 0 → alpha 1（实色），透光度 100 → alpha 0（完全透明）。
 */
const resolveTintAlpha = (opacity: number) => {
  const clamped = Math.max(0, Math.min(100, opacity));
  return ((100 - clamped) / 100).toFixed(2);
};

/**
 * 把 0-100 的模糊度映射到 `backdrop-filter: blur(<px>)` 的像素值。
 * 0 = 不模糊（0px），100 = 强烈模糊（48px），与 TieZ 视觉量级一致。
 */
const resolveBlurPx = (blur: number) => {
  const clamped = Math.max(0, Math.min(100, blur));
  return Math.round((clamped / 100) * 48);
};

const AppContent: FC = () => {
  const { message, modal } = AntdApp.useApp();

  useEffect(() => {
    setMessageApi(message);
    setModalApi(modal);
  }, [message, modal]);

  return <RouterProvider router={router} />;
};

/**
 * 等待 Rust 设置首屏快照灌入后再渲染，避免组件读到空对象闪烁默认值。
 * `use()` 在 promise pending 时抛出，由父级（`main.tsx`）的 Suspense 接住。
 */
const App: FC = () => {
  use(settingsReady);

  const { i18n } = useTranslation();
  const settings = useSnapshot(settingsState);
  const windowLabel = getCurrentWebviewWindow().label;
  const mode =
    windowLabel === WINDOW_LABEL.ONBOARDING
      ? "dark"
      : settings.appearance.theme;
  const language = settings.appearance.language;
  const windowEffect = settings.appearance.windowEffect;
  const acrylicOpacity = settings.appearance.acrylicOpacity;
  const acrylicBlur = settings.appearance.acrylicBlur;
  const antdTheme = useAppTheme(mode);
  const locale = resolveAntdLocale(language);

  // 窗口毛玻璃材质：挂到 html 上供 global.scss 完整复刻 TieZ 毛玻璃（半透明 antd 变量 +
  // backdrop-filter 玻璃面），透出 Rust 侧挂载的系统亚克力材质。引导窗口固定深色 UI，不参与；
  // 复制成功小气泡（copied）自带胶囊样式，也跳过全局玻璃壳（见 :not(.copied-root)）。
  useEffect(() => {
    const root = document.documentElement;
    const isFloatingWindow =
      windowLabel !== WINDOW_LABEL.ONBOARDING &&
      windowLabel !== WINDOW_LABEL.COPIED;
    const active = isFloatingWindow && windowEffect !== "none";

    root.classList.toggle("vibrancy", active);
    root.classList.toggle(
      "vibrancy-acrylic",
      active && windowEffect === "acrylic",
    );
  }, [windowLabel, windowEffect]);

  // 毛玻璃透光度 / 模糊度：把设置项映射到 CSS 变量，slider 拖动时实时刷新。
  useEffect(() => {
    const root = document.documentElement;
    const tintAlpha = resolveTintAlpha(acrylicOpacity);
    const blurPx = resolveBlurPx(acrylicBlur);
    root.style.setProperty("--glass-tint-alpha", tintAlpha);
    root.style.setProperty("--glass-blur", `${blurPx}px`);
  }, [acrylicOpacity, acrylicBlur]);

  useEffect(() => {
    document.documentElement.lang = language;

    if (i18n.language === language) return;

    void i18n.changeLanguage(language);
  }, [i18n, language]);

  // settingsReady 已由 use() gate，挂载即视为前端基础初始化完成；回报 Rust 推进窗口到 ready 阶段。
  // notifyWindowReady 内部已吞掉并记录失败，这里无需再 try/catch。
  useMount(async () => {
    await notifyWindowReady(getCurrentWebviewWindow().label);
  });

  // 兜底未捕获的 Promise rejection：统一进日志通道，避免只在 devtools 红字闪过、生产环境完全无痕。
  useEventListener("unhandledrejection", (event) => {
    const { reason } = event;

    log.error(
      "unhandled promise rejection",
      reason instanceof Error ? reason : { reason },
    );
  });

  // 兜底未捕获的同步异常（含资源加载错误）。React 渲染错误由 ErrorBoundary 接，不会走到这里。
  useEventListener("error", (event) => {
    const { error, ...rest } = event;

    log.error("uncaught error", error instanceof Error ? error : rest);
  });

  return (
    <ConfigProvider locale={locale} modal={ANTD_MODAL_CONFIG} theme={antdTheme}>
      <AntdApp>
        <AppContent />
      </AntdApp>
    </ConfigProvider>
  );
};

export default App;
