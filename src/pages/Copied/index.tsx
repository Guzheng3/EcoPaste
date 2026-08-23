import type { FC } from "react";
import { useTranslation } from "react-i18next";

/**
 * 复制成功小气泡窗 `/copied`：由 Rust 侧按需创建独立置顶小窗（`transparent`）加载本页。
 * 窗体宽度恰好容纳胶囊，居中显示在屏幕底部。
 * 不依赖剪贴板主窗口是否可见。
 *
 * 胶囊是实心绿色（`.copied-toast`），不走毛玻璃，保证一眼可见。
 */
const Copied: FC = () => {
  const { t } = useTranslation("commands");

  return (
    <div className="flex h-screen w-screen items-end justify-center pb-3">
      <div className="copied-toast">
        <i
          aria-hidden="true"
          className="i-lucide:check size-4 shrink-0 text-white"
        />
        <span>{t("copied")}</span>
      </div>
    </div>
  );
};

export default Copied;
