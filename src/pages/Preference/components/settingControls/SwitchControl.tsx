import { Switch } from "antd";
import type { FC } from "react";
import type { PreferenceSetting } from "../../types/preferences";
import ControlFrame from "./ControlFrame";
import type { ControlProps } from "./types";

interface SwitchControlProps extends ControlProps {
  setting: PreferenceSetting;
  value: boolean;
}

/**
 * 即时保存二元设置。
 */
const SwitchControl: FC<SwitchControlProps> = (props) => {
  const { disabled, onChange, setting, value } = props;

  const handleChange = async (checked: boolean) => {
    await onChange(setting, checked);
  };

  return (
    <ControlFrame>
      <Switch checked={value} disabled={disabled} onChange={handleChange} />
    </ControlFrame>
  );
};

export default SwitchControl;
