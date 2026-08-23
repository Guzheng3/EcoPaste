import type { FC } from "react";
import { useTranslation } from "react-i18next";

/**
 * 复制成功小气泡窗 `/copied`：由 Rust 侧按需创建独立置顶小窗（`transparent`）加载本页，
 * 全窗透明容器里居中渲染「已复制」卡片，展示完成后由 Rust 侧 1.5s 自动隐藏窗口。
 * 不依赖剪贴板主窗口是否可见。
 */
const Copied: FC = () => {
  const { t } = useTranslation("commands");

  return (
    <div className="flex h-screen w-screen items-center justify-center">
      <div className="flex items-center gap-1 rounded-lg border border-ant-border bg-ant-container/95 px-2.5 py-1.5 shadow-md">
        <i
          aria-hidden="true"
          className="i-lucide:check size-3.5 shrink-0 text-ant-success"
        />
        <span className="text-ant-text text-xs leading-none">
          {t("copied")}
        </span>
      </div>
    </div>
  );
};

export default Copied;
