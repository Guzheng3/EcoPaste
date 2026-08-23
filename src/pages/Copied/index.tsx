import type { FC } from "react";
import { useTranslation } from "react-i18next";

/**
 * 复制成功小气泡窗 `/copied`：由 Rust 侧按需创建独立置顶小窗（`transparent`）加载本页，
 * 1.5s 后自动隐藏。不依赖剪贴板主窗口是否可见。
 *
 * 绿色对勾用 `text-ant-success`（antd 成功色，明暗主题自动适配），文字用主题前景色，
 * 外层为半透明胶囊背景，内容水平垂直完全居中。
 */
const Copied: FC = () => {
  const { t } = useTranslation("commands");

  return (
    <div className="flex h-screen w-screen items-center justify-center">
      <div className="flex items-center gap-1.5 rounded-full border border-zinc-300/60 bg-white/90 px-3.5 py-2 font-semibold text-[13px] text-zinc-700 leading-none shadow-sm dark:border-zinc-600/60 dark:bg-zinc-800/90 dark:text-zinc-200">
        <i
          aria-hidden="true"
          className="i-lucide:check size-4 shrink-0 text-ant-success"
        />
        <span className="truncate">{t("copied")}</span>
      </div>
    </div>
  );
};

export default Copied;
