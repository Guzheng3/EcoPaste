import { useMount } from "ahooks";
import type { TFunction } from "i18next";
import type {
  Dispatch,
  FC,
  MouseEvent,
  RefObject,
  SetStateAction,
} from "react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSnapshot } from "valtio";
import {
  deleteClipboardGroup,
  listClipboardGroups,
  openPreferenceWithHighlight,
  updateClipboardGroup,
} from "@/commands";
import ClipboardGroupIcon from "@/components/ClipboardGroupIcon";
import ClipboardGroupModal from "@/components/ClipboardGroupModal";
import Dropdown, { type DropdownMenuItems } from "@/components/Dropdown";
import KeyHint from "@/components/KeyHint";
import Tooltip from "@/components/Tooltip";
import { TAURI_EVENT } from "@/constants/events";
import { useKeyboardEvent } from "@/hooks/useKeyboardEvent";
import { useTauriListen } from "@/hooks/useTauriListen";
import { clipboardViewState } from "@/stores/clipboardView";
import type {
  ClipboardCategory,
  ClipboardGroupIcon as ClipboardGroupIconValue,
  ClipboardGroupInput,
  ClipboardGroupRecord,
  ClipboardRange,
} from "@/types/clipboard";
import { cn } from "@/utils/cn";
import { getModalApi } from "@/utils/feedback";

type MoreMenuAction = "manageGroups";
type GroupMenuAction = "delete" | "edit" | "hide";
type MoreMenuGroupKey = `group:${string}`;

interface RangeGroupOption {
  labelKey: string;
  value: ClipboardRange;
  icon: ClipboardGroupIconValue;
}

interface OverflowGroupMenuLabelProps {
  menuItems: DropdownMenuItems;
  onContext: (record: ClipboardGroupRecord) => void;
  onMenuClick: (info: { key: string }) => void;
  record: ClipboardGroupRecord;
}

interface GroupSeparatorProps {
  separatorRef?: RefObject<HTMLSpanElement | null>;
}

const RANGE_GROUP_OPTIONS: RangeGroupOption[] = [
  {
    icon: "i-lets-icons:star",
    labelKey: "groups.favorite",
    value: "favorite",
  },
];

type PrimaryGroupValue = ClipboardCategory | "all";

interface PrimaryGroupOption {
  labelKey: string;
  value: PrimaryGroupValue;
  icon: ClipboardGroupIconValue;
}

/**
 * 主筛选 Tab：全部 与 文本/图片/文件 **并列**（单选其一）。
 * 「全部」不包含分类，而是分类的一种未限定状态；「收藏」由 `range` 独立叠加。
 */
const PRIMARY_GROUP_OPTIONS: PrimaryGroupOption[] = [
  { icon: "i-lets-icons:widget", labelKey: "groups.all", value: "all" },
  { icon: "i-lets-icons:file-dock", labelKey: "groups.text", value: "text" },
  { icon: "i-lets-icons:img-box", labelKey: "groups.image", value: "image" },
  {
    icon: "i-lets-icons:folder-file-alt",
    labelKey: "groups.files",
    value: "files",
  },
];

const GROUP_MENU_ACTION = {
  DELETE: "delete",
  EDIT: "edit",
  HIDE: "hide",
} as const satisfies Record<string, GroupMenuAction>;

const MORE_MENU_ACTION = {
  MANAGE_GROUPS: "manageGroups",
} as const satisfies Record<string, MoreMenuAction>;

const CUSTOM_GROUPS_SETTING_ID = "organizing.customGroups";

const GROUP_BUTTON_BASE_CLASS =
  "flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-1.5 border-0 bg-transparent p-0 transition-colors";
const GROUP_ICON_BUTTON_CLASS =
  "flex h-9 w-12 shrink-0 cursor-pointer flex-col items-center justify-center gap-0.5 rounded-1.5 border-0 bg-transparent px-0.5 py-1 transition-colors";
const GROUP_BUTTON_LABEL_CLASS = "w-full truncate text-[10px] leading-none";
const GROUP_ACTION_BUTTON_WIDTH = 24;
const GROUP_FILTER_BUTTON_WIDTH = 48;
const GROUP_BUTTON_GAP = 4;
const GROUP_SEPARATOR_MARGIN = 4;

/**
 * Header 下方的分组筛选栏：内置类型分组 + 自定义分组入口。
 */
const Group: FC = () => {
  const { t } = useTranslation(["clipboard", "common"]);
  const { category, groupId, range } = useSnapshot(clipboardViewState);

  const [customGroups, setCustomGroups] = useState<ClipboardGroupRecord[]>([]);
  const [modalOpen, setModalOpen] = useState(false);
  const [visibleCustomGroupCount, setVisibleCustomGroupCount] = useState(
    Number.POSITIVE_INFINITY,
  );
  const [editingGroup, setEditingGroup] = useState<ClipboardGroupRecord | null>(
    null,
  );
  const toolbarRef = useRef<HTMLDivElement>(null);
  const customGroupAnchorRef = useRef<HTMLSpanElement>(null);
  const contextGroupRef = useRef<ClipboardGroupRecord | null>(null);
  const deleteGroupRef = useRef<ClipboardGroupRecord | null>(null);

  const visibleCustomGroups = customGroups.filter((record) => {
    return !record.isHidden;
  });
  const inlineCustomGroups = visibleCustomGroups.slice(
    0,
    visibleCustomGroupCount,
  );
  const overflowCustomGroups = visibleCustomGroups.slice(
    visibleCustomGroupCount,
  );

  /**
   * 从 Rust 拉取自定义分组。
   */
  const loadGroups = async () => {
    const groups = await listClipboardGroups();

    setCustomGroups(groups);
    scheduleVisibleCustomGroupCountUpdate(
      toolbarRef,
      customGroupAnchorRef,
      setVisibleCustomGroupCount,
      groups.filter((record) => {
        return !record.isHidden;
      }).length,
    );
    ensureSelectedGroupStillExists(groups);
  };

  /**
   * 首次挂载时拉取分组。
   */
  useMount(() => {
    void loadGroups();
  });

  /**
   * 其他窗口或命令修改分组后刷新本地列表。
   */
  const handleGroupsUpdated = () => {
    void loadGroups();
  };

  useTauriListen(TAURI_EVENT.CLIPBOARD_GROUPS_UPDATED, handleGroupsUpdated);

  /**
   * 容器尺寸变化时重新测量溢出状态。
   */
  useEffect(() => {
    const toolbar = toolbarRef.current;
    const customAnchor = customGroupAnchorRef.current;
    if (!toolbar || !customAnchor) return;

    const updateVisibleCustomGroupCount = () => {
      commitVisibleCustomGroupCount(
        toolbar,
        customAnchor,
        setVisibleCustomGroupCount,
        visibleCustomGroups.length,
      );
    };

    const observer = new ResizeObserver(updateVisibleCustomGroupCount);
    observer.observe(toolbar);
    updateVisibleCustomGroupCount();

    return () => {
      observer.disconnect();
    };
  }, [visibleCustomGroups.length]);

  /**
   * 切换收藏范围开关；与主 Tab（全部/分类）正交，可叠加。
   */
  const toggleRange = (value: ClipboardRange) => {
    clipboardViewState.range =
      clipboardViewState.range === value ? "all" : value;
  };

  /**
   * 切换到某个主 Tab；「全部」与「文本/图片/文件」并列单选。
   */
  const selectPrimary = (value: PrimaryGroupValue) => {
    clipboardViewState.category = value === "all" ? null : value;
  };

  /**
   * 切换到自定义分组；再次点击当前分组时取消。
   */
  const toggleCustomGroup = (id: string) => {
    clipboardViewState.groupId = clipboardViewState.groupId === id ? null : id;
  };

  /**
   * 点击分组按钮时根据 data 属性切换筛选。
   */
  const handleGroupClick = (event: MouseEvent<HTMLButtonElement>) => {
    const type = event.currentTarget.dataset.type;
    const value = event.currentTarget.dataset.value;
    const nextGroupId = event.currentTarget.dataset.groupId;

    if (nextGroupId) {
      toggleCustomGroup(nextGroupId);
      return;
    }

    if (type === "range" && isRangeGroup(value)) {
      toggleRange(value);
      return;
    }

    if (type === "primary" && isPrimaryGroup(value)) {
      selectPrimary(value);
    }
  };

  /**
   * 记录右键菜单所属分组。
   */
  const handleCustomGroupContextMenu = (
    event: MouseEvent<HTMLButtonElement>,
  ) => {
    const nextGroupId = event.currentTarget.dataset.groupId;
    if (!nextGroupId) return;

    contextGroupRef.current =
      customGroups.find((record) => {
        return record.id === nextGroupId;
      }) ?? null;
  };

  /**
   * 处理分组栏快捷键：Cmd/Ctrl+Q 切换收藏开关，左右键在主 Tab（全部/分类）间循环，Tab / Shift+Tab 仅在可见自定义分组间循环。
   */
  const handleKeyDown = (event: KeyboardEvent) => {
    const eventModifierPressed = event.metaKey || event.ctrlKey;

    if (eventModifierPressed && event.key.toLowerCase() === "q") {
      event.preventDefault();
      toggleRange("favorite");

      return;
    }

    if (
      (event.key === "ArrowLeft" || event.key === "ArrowRight") &&
      !shouldUseNativeHorizontalNavigation(event)
    ) {
      event.preventDefault();
      selectAdjacentPrimary(event.key === "ArrowLeft" ? -1 : 1);

      return;
    }

    if (event.key !== "Tab") return;

    event.preventDefault();

    const nextGroupId = selectAdjacentCustomGroup(
      visibleCustomGroups,
      groupId,
      event.shiftKey,
    );

    if (!nextGroupId) return;

    toggleCustomGroup(nextGroupId);
  };

  useKeyboardEvent("keydown", handleKeyDown);

  /**
   * 按方向键在主 Tab 序列「全部→文本→图片→文件」内循环。
   */
  const selectAdjacentPrimary = (direction: -1 | 1) => {
    const options = PRIMARY_GROUP_OPTIONS.map((option) => {
      return option.value;
    });
    const currentValue = clipboardViewState.category ?? "all";
    const current = options.indexOf(currentValue);
    const startIndex = direction === 1 ? -1 : options.length;
    const nextIndex =
      (current === -1 ? startIndex + direction : current + direction) %
      options.length;
    const normalizedIndex = (nextIndex + options.length) % options.length;

    selectPrimary(options[normalizedIndex]);
  };

  /**
   * 打开编辑分组弹框。
   */
  const openEditModal = (record: ClipboardGroupRecord) => {
    setEditingGroup(record);
    setModalOpen(true);
  };

  /**
   * 关闭新增 / 编辑分组弹框。
   */
  const closeModal = () => {
    setModalOpen(false);
    setEditingGroup(null);
  };

  /**
   * 保存分组弹框内容。
   */
  const handleModalSubmit = async (input: ClipboardGroupInput) => {
    if (!editingGroup) return;

    await updateClipboardGroup(editingGroup.id, input);
    closeModal();
  };

  /**
   * 执行自定义分组右键菜单动作。
   */
  const handleGroupMenuClick = (info: { key: string }) => {
    const record = contextGroupRef.current;
    if (!record) return;

    const action = parseGroupMenuAction(info.key);
    if (!action) return;

    if (action === GROUP_MENU_ACTION.EDIT) {
      openEditModal(record);
      return;
    }

    if (action === GROUP_MENU_ACTION.HIDE) {
      void updateClipboardGroup(record.id, {
        icon: record.icon,
        isHidden: true,
        name: record.name,
      });
      return;
    }

    if (action === GROUP_MENU_ACTION.DELETE) {
      requestDeleteGroup(record);
    }
  };

  /**
   * 打开偏好设置并定位到自定义分组管理项。
   */
  const openGroupPreference = async () => {
    await openPreferenceWithHighlight(CUSTOM_GROUPS_SETTING_ID);
  };

  /**
   * 执行更多菜单动作：管理分组，或切换到溢出的自定义分组。
   */
  const handleMoreMenuClick = async (info: { key: string }) => {
    const action = parseMoreMenuAction(info.key);
    if (action === MORE_MENU_ACTION.MANAGE_GROUPS) {
      await openGroupPreference();
      return;
    }

    const id = parseMoreMenuGroupId(info.key);
    if (!id) return;

    toggleCustomGroup(id);
  };

  /**
   * 弹出删除确认框。
   */
  const requestDeleteGroup = (record: ClipboardGroupRecord) => {
    deleteGroupRef.current = record;

    getModalApi().confirm({
      centered: true,
      content: (
        <span className="text-ant-secondary text-sm">
          {t("clipboard:groups.deleteConfirmDescription", {
            group: record.name,
          })}
        </span>
      ),
      okButtonProps: { danger: true },
      okText: t("common:actions.delete"),
      onOk: confirmDeleteGroup,
      title: t("clipboard:groups.delete"),
    });
  };

  /**
   * 确认删除当前待删除分组。
   */
  const confirmDeleteGroup = async () => {
    const record = deleteGroupRef.current;
    if (!record) return;

    await deleteClipboardGroup(record.id);

    if (clipboardViewState.groupId === record.id) {
      clipboardViewState.groupId = null;
    }

    deleteGroupRef.current = null;
  };

  const groupMenuItems = buildGroupActionMenuItems(t);

  /**
   * 记录溢出菜单中右键菜单所属分组。
   */
  const handleOverflowGroupContext = (record: ClipboardGroupRecord) => {
    contextGroupRef.current = record;
  };

  const moreMenuItems = buildMoreMenuItems(
    overflowCustomGroups,
    groupMenuItems,
    handleGroupMenuClick,
    handleOverflowGroupContext,
    t,
  );
  const moreMenuSelectedKeys = groupId ? [buildMoreMenuGroupKey(groupId)] : [];
  const moreButtonSelected = overflowCustomGroups.some((record) => {
    return record.id === groupId;
  });

  /**
   * 渲染溢出分组菜单按钮。
   */
  const renderMoreButton = () => {
    if (overflowCustomGroups.length === 0) return null;

    return (
      <Dropdown
        menu={{
          items: moreMenuItems,
          onClick: handleMoreMenuClick,
          selectedKeys: moreMenuSelectedKeys,
        }}
        tooltip={t("clipboard:groups.more")}
        trigger={["click"]}
      >
        <button
          className={cn(GROUP_BUTTON_BASE_CLASS, {
            "bg-ant-primary text-ant-light-solid": moreButtonSelected,
            "text-ant-secondary hover:bg-ant-fill-tertiary":
              !moreButtonSelected,
          })}
          type="button"
        >
          <i aria-hidden className="i-lucide:more-horizontal text-sm!" />
        </button>
      </Dropdown>
    );
  };

  /**
   * 渲染收藏范围开关按钮；单按钮，点击在开/关间切换。
   */
  const renderRangeButton = ({ labelKey, value, icon }: RangeGroupOption) => {
    const selected = range === value;
    const showShortcutHint = range === "all" && value === "favorite";

    return renderFilterButton({
      icon,
      label: t(`clipboard:${labelKey}`),
      selected,
      showShortcutHint,
      type: "range",
      value,
    });
  };

  /**
   * 渲染主 Tab 按钮（全部|文本|图片|文件，并列单选）。
   */
  const renderPrimaryButton = ({
    labelKey,
    value,
    icon,
  }: PrimaryGroupOption) => {
    const selected = value === "all" ? category === null : category === value;

    return renderFilterButton({
      icon,
      label: t(`clipboard:${labelKey}`),
      selected,
      type: "primary",
      value,
    });
  };

  /**
   * 渲染单个筛选按钮。
   */
  const renderFilterButton = (options: {
    icon: ClipboardGroupIconValue;
    label: string;
    selected: boolean;
    showShortcutHint?: boolean;
    type: "primary" | "range";
    value: PrimaryGroupValue | ClipboardRange;
  }) => {
    const { icon, label, selected, showShortcutHint, type, value } = options;

    return (
      <Tooltip key={`${type}:${value}`} title={label}>
        <button
          className={cn(GROUP_ICON_BUTTON_CLASS, {
            "bg-ant-primary text-ant-light-solid": selected,
            "text-ant-secondary hover:bg-ant-fill-tertiary": !selected,
          })}
          data-type={type}
          data-value={value}
          onClick={handleGroupClick}
          type="button"
        >
          {showShortcutHint ? (
            <KeyHint hintKey="Q">
              <ClipboardGroupIcon icon={icon} selected={selected} />
            </KeyHint>
          ) : (
            <ClipboardGroupIcon icon={icon} selected={selected} />
          )}
          <span className={GROUP_BUTTON_LABEL_CLASS}>{label}</span>
        </button>
      </Tooltip>
    );
  };

  return (
    <>
      <div
        className="flex items-center gap-1 overflow-hidden px-3 pb-2"
        data-tauri-drag-region
        ref={toolbarRef}
      >
        {RANGE_GROUP_OPTIONS.map(renderRangeButton)}
        <GroupSeparator />
        {PRIMARY_GROUP_OPTIONS.map(renderPrimaryButton)}
        <GroupSeparator separatorRef={customGroupAnchorRef} />

        {inlineCustomGroups.length > 0 && (
          <div className="flex min-w-0 shrink-0 items-center gap-1 overflow-hidden">
            {inlineCustomGroups.map((record) => {
              const selected = groupId === record.id;

              return (
                <Dropdown
                  key={record.id}
                  menu={{
                    items: groupMenuItems,
                    onClick: handleGroupMenuClick,
                  }}
                  tooltip={record.name}
                  trigger={["contextMenu"]}
                >
                  <button
                    className={cn(GROUP_ICON_BUTTON_CLASS, {
                      "bg-ant-primary text-ant-light-solid": selected,
                      "text-ant-secondary hover:bg-ant-fill-tertiary":
                        !selected,
                    })}
                    data-group-id={record.id}
                    onClick={handleGroupClick}
                    onContextMenu={handleCustomGroupContextMenu}
                    type="button"
                  >
                    <ClipboardGroupIcon
                      icon={record.icon}
                      selected={selected}
                    />
                    <span className={GROUP_BUTTON_LABEL_CLASS}>
                      {record.name}
                    </span>
                  </button>
                </Dropdown>
              );
            })}
          </div>
        )}

        {renderMoreButton()}
      </div>

      <ClipboardGroupModal
        group={editingGroup}
        mode="edit"
        onCancel={closeModal}
        onSubmit={handleModalSubmit}
        open={modalOpen}
      />
    </>
  );
};

/**
 * 分隔范围、分类、自定义分组三段。
 */
const GroupSeparator: FC<GroupSeparatorProps> = (props) => {
  const { separatorRef } = props;

  return (
    <span
      aria-hidden
      className="mx-1 h-4 w-px shrink-0 bg-ant-split"
      ref={separatorRef}
    />
  );
};

/**
 * 溢出菜单里的分组行：左键选择分组，右键打开同一套分组管理菜单。
 */
const OverflowGroupMenuLabel: FC<OverflowGroupMenuLabelProps> = (props) => {
  const { menuItems, onContext, onMenuClick, record } = props;

  const handleContextMenu = () => {
    onContext(record);
  };

  return (
    <Dropdown
      menu={{
        items: menuItems,
        onClick: onMenuClick,
      }}
      trigger={["contextMenu"]}
    >
      <span
        className="flex min-w-28 items-center gap-2"
        onContextMenu={handleContextMenu}
        role="menuitem"
        tabIndex={-1}
      >
        <ClipboardGroupIcon icon={record.icon} inheritColor />
        <span>{record.name}</span>
      </span>
    </Dropdown>
  );
};

/**
 * 下一帧提交可见自定义分组数量；分组数据更新后等待 DOM 渲染完成再测量。
 */
function scheduleVisibleCustomGroupCountUpdate(
  toolbarRef: RefObject<HTMLDivElement | null>,
  customAnchorRef: RefObject<HTMLSpanElement | null>,
  setVisibleCustomGroupCount: Dispatch<SetStateAction<number>>,
  groupCount: number,
) {
  requestAnimationFrame(() => {
    commitVisibleCustomGroupCount(
      toolbarRef.current,
      customAnchorRef.current,
      setVisibleCustomGroupCount,
      groupCount,
    );
  });
}

/**
 * 根据自定义分组栏可用宽度写入可见分组数量。
 */
function commitVisibleCustomGroupCount(
  toolbar: HTMLDivElement | null,
  customAnchor: HTMLSpanElement | null,
  setVisibleCustomGroupCount: Dispatch<SetStateAction<number>>,
  groupCount: number,
) {
  const rawCapacity =
    toolbar && customAnchor
      ? computeCustomGroupCapacity(toolbar, customAnchor)
      : groupCount;
  const visibleCount = Math.min(groupCount, rawCapacity);

  setVisibleCustomGroupCount((current) => {
    if (current === visibleCount) return current;

    return visibleCount;
  });
}

/**
 * 按整条分组栏剩余宽度计算自定义分组可显示数量。
 */
function computeCustomGroupCapacity(
  toolbar: HTMLDivElement,
  customAnchor: HTMLSpanElement,
) {
  const toolbarRect = toolbar.getBoundingClientRect();
  const customRect = customAnchor.getBoundingClientRect();
  const customStart =
    customRect.right -
    toolbarRect.left +
    GROUP_SEPARATOR_MARGIN +
    GROUP_BUTTON_GAP;
  const actionSlotWidth = GROUP_BUTTON_GAP + GROUP_ACTION_BUTTON_WIDTH;
  const availableWidth = Math.max(
    0,
    toolbar.clientWidth - customStart - actionSlotWidth,
  );

  return Math.max(
    0,
    Math.floor(
      (availableWidth + GROUP_BUTTON_GAP) /
        (GROUP_FILTER_BUTTON_WIDTH + GROUP_BUTTON_GAP),
    ),
  );
}

/**
 * 构建自定义分组右键菜单；内联分组和溢出菜单分组共用这一份定义。
 */
function buildGroupActionMenuItems(
  t: TFunction<["clipboard", "common"]>,
): DropdownMenuItems {
  return [
    {
      icon: "i-lucide:pencil",
      key: GROUP_MENU_ACTION.EDIT,
      label: t("clipboard:groups.edit"),
    },
    {
      icon: "i-lucide:eye-off",
      key: GROUP_MENU_ACTION.HIDE,
      label: t("clipboard:groups.hide"),
    },
    { type: "divider" },
    {
      danger: true,
      icon: "i-lucide:trash-2",
      key: GROUP_MENU_ACTION.DELETE,
      label: t("clipboard:groups.delete"),
    },
  ];
}

/**
 * 构建更多菜单项：管理入口 + 溢出分组快速入口。
 * 新增分组只在偏好设置的分组管理里提供。
 */
function buildMoreMenuItems(
  groups: ClipboardGroupRecord[],
  groupMenuItems: DropdownMenuItems,
  onGroupMenuClick: (info: { key: string }) => void,
  onGroupContext: (record: ClipboardGroupRecord) => void,
  t: TFunction<["clipboard", "common"]>,
): DropdownMenuItems {
  const groupItems = groups.map((record) => {
    return {
      key: buildMoreMenuGroupKey(record.id),
      label: (
        <OverflowGroupMenuLabel
          menuItems={groupMenuItems}
          onContext={onGroupContext}
          onMenuClick={onGroupMenuClick}
          record={record}
        />
      ),
    };
  });

  if (groupItems.length === 0) {
    return [
      {
        icon: "i-lucide:settings-2",
        key: MORE_MENU_ACTION.MANAGE_GROUPS,
        label: t("clipboard:groups.manage"),
      },
    ];
  }

  return [
    ...groupItems,
    { type: "divider" },
    {
      icon: "i-lucide:settings-2",
      key: MORE_MENU_ACTION.MANAGE_GROUPS,
      label: t("clipboard:groups.manage"),
    },
  ];
}

/**
 * 解析自定义分组右键菜单动作。
 */
function parseGroupMenuAction(key: string): GroupMenuAction | null {
  const actions = Object.values(GROUP_MENU_ACTION);
  if (!actions.includes(key as GroupMenuAction)) return null;

  return key as GroupMenuAction;
}

/**
 * 解析更多菜单动作。
 */
function parseMoreMenuAction(key: string): MoreMenuAction | null {
  const actions = Object.values(MORE_MENU_ACTION);
  if (!actions.includes(key as MoreMenuAction)) return null;

  return key as MoreMenuAction;
}

/**
 * 生成更多菜单中的分组 key。
 */
function buildMoreMenuGroupKey(id: string): MoreMenuGroupKey {
  return `group:${id}`;
}

/**
 * 从更多菜单 key 中解析自定义分组 id。
 */
function parseMoreMenuGroupId(key: string) {
  if (!key.startsWith("group:")) return null;

  return key.slice("group:".length);
}

/**
 * 判断字符串是否为范围分组值。
 */
function isRangeGroup(value: unknown): value is ClipboardRange {
  return RANGE_GROUP_OPTIONS.some((option) => {
    return option.value === value;
  });
}

/**
 * 判断字符串是否为主 Tab（全部/分类）值。
 */
function isPrimaryGroup(value: unknown): value is PrimaryGroupValue {
  return PRIMARY_GROUP_OPTIONS.some((option) => {
    return option.value === value;
  });
}

/**
 * 在可见自定义分组间前后循环；当前未选中分组时，正向取第一个，反向取最后一个。
 */
function selectAdjacentCustomGroup(
  groups: ClipboardGroupRecord[],
  groupId: string | null,
  reverse: boolean,
) {
  if (groups.length === 0) return null;

  const currentIndex = groupId
    ? groups.findIndex((record) => {
        return record.id === groupId;
      })
    : -1;

  if (reverse) {
    if (currentIndex === -1) return groups[groups.length - 1]?.id ?? null;

    return (
      groups[(currentIndex - 1 + groups.length) % groups.length]?.id ?? null
    );
  }

  if (currentIndex === -1) return groups[0]?.id ?? null;

  return groups[(currentIndex + 1) % groups.length]?.id ?? null;
}

/**
 * 判断左右键是否应交给输入控件原生光标导航。
 */
function shouldUseNativeHorizontalNavigation(event: KeyboardEvent) {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return false;

  const tagName = target.tagName.toLowerCase();
  if (target.isContentEditable) return true;

  return tagName === "input" || tagName === "textarea";
}

/**
 * 当前选中分组被删除或不再存在时，回到全部分组。
 */
function ensureSelectedGroupStillExists(groups: ClipboardGroupRecord[]) {
  const selectedGroupId = clipboardViewState.groupId;
  if (!selectedGroupId) return;

  const exists = groups.some((record) => {
    return record.id === selectedGroupId;
  });
  if (exists) return;

  clipboardViewState.groupId = null;
}

export default Group;
