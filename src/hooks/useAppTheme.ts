import type { Event, UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useMount, useUnmount } from "ahooks";
import { type ThemeConfig, theme } from "antd";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Theme as SettingsTheme } from "@/types/settings";
import { log } from "@/utils/log";

type ResolvedTheme = "light" | "dark";
type NativeTheme = ResolvedTheme | null;

export interface GlassThemeOptions {
  /** 当前窗口是否激活毛玻璃（仅挂了系统亚克力底的窗口）。 */
  enabled: boolean;
  /** 玻璃面 tint alpha（0-1），来自「透光度」设置。 */
  tintAlpha: number;
}

/**
 * 根据用户设置与系统偏好解析当前实际主题。
 */
const resolveTheme = (mode: SettingsTheme, systemTheme: ResolvedTheme) => {
  if (mode === "dark") return "dark";
  if (mode === "light") return "light";

  return systemTheme;
};

/**
 * 把应用主题设置转换成 Tauri 原生窗口主题；null 表示跟随系统。
 */
const resolveNativeTheme = (mode: SettingsTheme): NativeTheme => {
  if (mode === "auto") return null;

  return mode;
};

/**
 * 把 Tauri 返回的窗口主题归一到前端可用的 light / dark。
 */
const normalizeTauriTheme = (value: ResolvedTheme | null): ResolvedTheme => {
  if (value === "dark") return "dark";

  return "light";
};

/**
 * 同步当前 webview window 的原生主题，并在 auto 模式下回读实际系统主题。
 */
const syncTauriWindowTheme = async (
  mode: SettingsTheme,
): Promise<ResolvedTheme | null> => {
  const currentWindow = getCurrentWebviewWindow();
  const nativeTheme = resolveNativeTheme(mode);

  await currentWindow.setTheme(nativeTheme);

  if (nativeTheme !== null) return null;

  return normalizeTauriTheme(await currentWindow.theme());
};

/**
 * 玻璃面 token：把大面积容器背景换成半透明，透出 Rust 侧挂载的系统亚克力模糊。
 * 走 `theme.token` 而非 CSS 变量覆盖——antd v6 的组件级变量（`.css-var-xxx`）会遮蔽
 * html 级覆盖，从 token 源头注入才能真正作用于所有 antd 组件。
 *
 * 容器底色再乘 0.55：系统亚克力 tint 已带一层底（`opacity_to_alpha`），两层叠加
 * 才接近 TieZ 单层 0.56 的观感，避免层叠过厚"看不见毛玻璃"。
 */
const GLASS_TINT_FACTOR = 0.55;

/**
 * 解析应用主题并同步系统主题变化、html class 与 Ant Design token 算法。
 */
export const useAppTheme = (
  mode: SettingsTheme,
  glass?: GlassThemeOptions,
): ThemeConfig => {
  const themeUnlistenRef = useRef<UnlistenFn | null>(null);
  const themeMountedRef = useRef(false);
  const [systemTheme, setSystemTheme] = useState<ResolvedTheme>("light");
  const resolvedTheme = resolveTheme(mode, systemTheme);
  const algorithm =
    resolvedTheme === "dark" ? theme.darkAlgorithm : theme.defaultAlgorithm;
  const glassEnabled = glass?.enabled ?? false;
  const tintAlpha = glass?.tintAlpha ?? 0;
  const antdTheme = useMemo(() => {
    // antd v6 默认启用 CSS 变量模式，token 值会直接写入组件级 `--ant-*` 变量。
    const config: ThemeConfig = { algorithm };

    if (glassEnabled) {
      const surface = (
        Math.max(0, Math.min(1, tintAlpha)) * GLASS_TINT_FACTOR
      ).toFixed(3);

      config.token =
        resolvedTheme === "dark"
          ? {
              colorBgContainer: `rgba(20, 20, 20, ${surface})`,
              colorBgElevated: "rgba(30, 30, 30, 0.88)",
              colorBgLayout: `rgba(20, 20, 20, ${surface})`,
            }
          : {
              colorBgContainer: `rgba(252, 252, 252, ${surface})`,
              colorBgElevated: "rgba(255, 255, 255, 0.86)",
              colorBgLayout: `rgba(252, 252, 252, ${surface})`,
            };
    }

    return config;
  }, [algorithm, glassEnabled, tintAlpha, resolvedTheme]);

  /**
   * 接收 Tauri 系统主题变化事件，驱动 `auto` 模式的实际主题。
   */
  const handleTauriThemeChanged = (event: Event<ResolvedTheme>) => {
    setSystemTheme(event.payload);
  };

  /**
   * 初始化 Tauri 主题快照与系统主题变化监听。
   */
  const initializeTauriThemeListener = async () => {
    try {
      const currentWindow = getCurrentWebviewWindow();
      const currentTheme = await currentWindow.theme();
      const unlisten = await currentWindow.onThemeChanged(
        handleTauriThemeChanged,
      );

      setSystemTheme(normalizeTauriTheme(currentTheme));

      if (!themeMountedRef.current) {
        unlisten();
        return;
      }

      themeUnlistenRef.current = unlisten;
    } catch (error) {
      log.error("tauri theme listener failed", error);
    }
  };

  /**
   * 移除 Tauri 系统主题变化监听。
   */
  const cleanupTauriThemeListener = () => {
    themeMountedRef.current = false;

    if (!themeUnlistenRef.current) return;

    themeUnlistenRef.current();
    themeUnlistenRef.current = null;
  };

  useMount(() => {
    themeMountedRef.current = true;
    void initializeTauriThemeListener();
  });

  useUnmount(cleanupTauriThemeListener);

  useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", resolvedTheme === "dark");
    root.classList.toggle("light", resolvedTheme === "light");
  }, [resolvedTheme]);

  useEffect(() => {
    let stale = false;

    /**
     * 将设置里的主题模式同步给 Tauri 原生窗口，覆盖标题栏等非 Web 区域。
     */
    const syncNativeTheme = async () => {
      try {
        const currentTheme = await syncTauriWindowTheme(mode);

        if (stale || currentTheme === null) return;

        setSystemTheme(currentTheme);
      } catch (error) {
        log.error("tauri native theme sync failed", error);
      }
    };

    void syncNativeTheme();

    return () => {
      stale = true;
    };
  }, [mode]);

  return antdTheme;
};
