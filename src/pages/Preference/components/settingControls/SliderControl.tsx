import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Slider } from "antd";
import type { FC } from "react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { setWindowDirty } from "@/commands";
import type { PreferenceSetting } from "../../types/preferences";
import { translatePreferenceNumberSuffix } from "../../utils/preferenceI18n";
import type { ControlProps } from "./types";

interface SliderControlProps extends ControlProps {
  setting: PreferenceSetting;
  value: number;
}

/**
 * 滑块控件：拖动过程中持续更新本地草稿，松手/键入 End 时强制落盘。
 * 视觉宽度按 `min-w-40` 起步，避免过窄时 thumb 拽出 track。
 */
const SliderControl: FC<SliderControlProps> = (props) => {
  const { t } = useTranslation("preferences");
  const { disabled, onChange, setting, value } = props;
  const control = setting.control.type === "slider" ? setting.control : null;
  const [draft, setDraft] = useState<number>(value);
  const dirtyRef = useRef(false);
  const windowLabelRef = useRef(getCurrentWebviewWindow().label);
  const dirtyOwner = `slider:${setting.id}`;

  useEffect(() => {
    setDraft(value);
  }, [value]);

  useEffect(() => {
    const dirty = draft !== value;
    if (dirtyRef.current === dirty) return;

    dirtyRef.current = dirty;
    void setWindowDirty(windowLabelRef.current, dirtyOwner, dirty);
  }, [dirtyOwner, draft, value]);

  useEffect(() => {
    return () => {
      if (!dirtyRef.current) return;

      dirtyRef.current = false;
      void setWindowDirty(windowLabelRef.current, dirtyOwner, false);
    };
  }, [dirtyOwner]);

  if (!control) return null;

  const min = control.min ?? 0;
  const max = control.max ?? 100;
  const step = control.step ?? 1;
  const suffix = translatePreferenceNumberSuffix(t, setting);
  const clamped = Math.min(max, Math.max(min, draft));

  const handleChange = (next: number) => {
    setDraft(next);
  };

  /**
   * 滑块松手时把最终值落盘；拖动过程中的 onChange 只更新本地草稿，避免频繁写盘。
   */
  const handleChangeComplete = async (next: number) => {
    const bounded = Math.min(max, Math.max(min, next));
    setDraft(bounded);
    await onChange(setting, bounded);
  };

  return (
    <div className="flex min-w-40 items-center gap-2">
      <Slider
        disabled={disabled}
        max={max}
        min={min}
        onChange={handleChange}
        onChangeComplete={handleChangeComplete}
        step={step}
        style={{ flex: 1 }}
        value={clamped}
      />
      <span className="w-10 text-right text-ant-secondary text-xs tabular-nums">
        {clamped}
        {suffix ?? ""}
      </span>
    </div>
  );
};

export default SliderControl;
