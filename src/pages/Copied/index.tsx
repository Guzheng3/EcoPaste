import type { FC } from "react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";

/**
 * 复制成功小气泡窗 `/copied`：由 Rust 侧按需创建独立置顶小窗（`transparent`）加载本页。
 * 窗体宽度恰好容纳胶囊，居中显示在屏幕底部；胶囊本身带毛玻璃样式（`.copied-toast`）。
 * 不依赖剪贴板主窗口是否可见。
 *
 * `#root` 挂上 `copied-root` 标识后，`global.scss` 的 `#root:not(.copied-root)` 不会对它
 * 加圆角玻璃壳，让独立透明窗的背景完全透出，毛玻璃胶囊独立呈现。
 */
const Copied: FC = () => {
  const { t } = useTranslation("commands");

  useEffect(() => {
    document.getElementById("root")?.classList.add("copied-root");

    return () => {
      document.getElementById("root")?.classList.remove("copied-root");
    };
  }, []);

  return (
    <div className="flex h-screen w-screen items-end justify-center pb-3">
      <div className="copied-toast">
        <i
          aria-hidden="true"
          className="i-lucide:check size-3.5 shrink-0 text-ant-success"
        />
        <span>{t("copied")}</span>
      </div>
    </div>
  );
};

export default Copied;
