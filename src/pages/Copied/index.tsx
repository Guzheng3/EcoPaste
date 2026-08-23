import type { FC } from "react";
import { useTranslation } from "react-i18next";

/**
 * 复制成功小气泡窗 `/copied`：由 Rust 侧按需创建独立置顶小窗（`transparent` + 亚克力材质）
 * 加载本页，1.5s 后自动隐藏。不依赖剪贴板主窗口是否可见。
 *
 * 液态玻璃：Rust 侧挂系统亚克力（真实桌面模糊），前端玻璃面占满整个窗口（`.copied-toast`
 * 复刻主窗口玻璃壳：半透明 tint + 高光渐变 + backdrop-filter），内容水平垂直完全居中。
 * 对勾用 `text-ant-success`（antd 成功色，明暗主题自动适配），文字用主题前景色。
 */
const Copied: FC = () => {
  const { t } = useTranslation("commands");

  return (
    <div className="copied-toast h-screen w-screen">
      <i
        aria-hidden="true"
        className="i-lucide:check size-4 shrink-0 text-ant-success"
      />
      <span className="truncate">{t("copied")}</span>
    </div>
  );
};

export default Copied;
