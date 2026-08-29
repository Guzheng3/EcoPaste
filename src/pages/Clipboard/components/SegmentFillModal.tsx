import { Input, Modal, Switch, Tabs } from "antd";
import type { FC, PointerEvent as ReactPointerEvent } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { fillSelectedText, segmentClipboardItem } from "@/commands";
import type { ClipboardItem, SegmentEditResult } from "@/types/clipboard";
import { cn } from "@/utils/cn";

const { TextArea } = Input;

interface SegmentFillModalProps {
  item: ClipboardItem | null;
  onClose: () => void;
}

interface BlockItem {
  id: string;
  text: string;
}

type TabKey = "words" | "links" | "emails" | "phones";

const TAB_KEYS: readonly TabKey[] = ["words", "links", "emails", "phones"];

const SegmentFillModal: FC<SegmentFillModalProps> = (props) => {
  const { item, onClose } = props;
  const { t } = useTranslation(["clipboard", "common"]);

  const [result, setResult] = useState<SegmentEditResult | null>(null);
  const [editingText, setEditingText] = useState("");
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const [activeTab, setActiveTab] = useState<TabKey>("words");
  const [filling, setFilling] = useState(false);

  // 开关状态
  const [enableWords, setEnableWords] = useState(true);
  const [enableLinks, setEnableLinks] = useState(true);
  const [enableEmails, setEnableEmails] = useState(true);
  const [enablePhones, setEnablePhones] = useState(true);

  const dragRef = useRef<{
    anchor: number;
    base: ReadonlySet<number>;
    moved: boolean;
  } | null>(null);

  const itemIdRef = useRef<string | null>(null);
  itemIdRef.current = item?.id ?? null;

  // 加载数据
  useEffect(() => {
    if (!item) return;
    setSelected(new Set());
    setLoading(true);
    setActiveTab("words");

    segmentClipboardItem(item.id)
      .then((res) => {
        setResult(res);
        setEditingText(res.text);
      })
      .finally(() => setLoading(false));
  }, [item]);

  // 重新分析：文本变化时自动重新提取
  // biome-ignore lint/correctness/useExhaustiveDependencies: result deps would cause infinite loop
  useEffect(() => {
    if (!result || editingText === result.text) return;
    const id = itemIdRef.current;
    if (!id) return;
    const timer = setTimeout(() => {
      setSelected(new Set());
      segmentClipboardItem(id).then((res) => {
        setResult({ ...res, text: editingText });
      });
    }, 300);
    return () => clearTimeout(timer);
  }, [editingText]);

  // 当前 tab 的词块列表
  const currentBlocks = useMemo(() => {
    if (!result) return [];
    const tabKey = activeTab;
    let items: string[] = [];
    switch (tabKey) {
      case "words":
        items = enableWords ? result.blocks : [];
        break;
      case "links":
        items = enableLinks ? result.links : [];
        break;
      case "emails":
        items = enableEmails ? result.emails : [];
        break;
      case "phones":
        items = enablePhones ? result.phones : [];
        break;
    }
    return items.map(
      (text, index): BlockItem => ({
        id: `${tabKey}-${index}`,
        text,
      }),
    );
  }, [result, activeTab, enableWords, enableLinks, enableEmails, enablePhones]);

  const selectedText = useMemo(() => {
    return currentBlocks
      .filter((_, index) => selected.has(index))
      .map((block) => block.text)
      .join(" ");
  }, [currentBlocks, selected]);

  // 松开鼠标结束框选：pointerup 可能落在词块以外的区域（间隙、预览框、弹窗背景），
  // 仅绑定在词块按钮上会漏掉，导致 dragRef 残留、选区在松开后仍跟随指针扩展。
  useEffect(() => {
    const endDrag = () => {
      dragRef.current = null;
    };
    window.addEventListener("pointerup", endDrag);
    window.addEventListener("pointercancel", endDrag);
    return () => {
      window.removeEventListener("pointerup", endDrag);
      window.removeEventListener("pointercancel", endDrag);
    };
  }, []);

  const handlePointerDown = (
    event: ReactPointerEvent<HTMLButtonElement>,
    index: number,
  ) => {
    if (event.button !== 0) return;
    event.preventDefault();
    dragRef.current = { anchor: index, base: new Set(selected), moved: false };
  };

  // 拖拽中的命中检测：指针滑到哪个词块就把选区扩展到哪；
  // 松开后 dragRef 已被 window 监听清空，此处直接返回，选区不再跟随。
  const handlePointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    if (event.buttons === 0) {
      dragRef.current = null;
      return;
    }
    const target = (event.target as HTMLElement).closest<HTMLElement>(
      "[data-block-index]",
    );
    if (!target) return;
    const index = Number(target.dataset.blockIndex);
    if (index !== drag.anchor) drag.moved = true;
    const lo = Math.min(drag.anchor, index);
    const hi = Math.max(drag.anchor, index);
    const next = new Set(drag.base);
    for (let i = lo; i <= hi; i += 1) next.add(i);
    setSelected(next);
  };

  const handlePointerUp = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    event.preventDefault();
    if (!drag.moved) {
      setSelected((prev) => {
        const next = new Set(prev);
        if (next.has(drag.anchor)) next.delete(drag.anchor);
        else next.add(drag.anchor);
        return next;
      });
    }
    dragRef.current = null;
  };

  const handleFill = async () => {
    if (!selectedText) return;
    setFilling(true);
    try {
      await fillSelectedText(selectedText);
      onClose();
    } finally {
      setFilling(false);
    }
  };

  const hasAnyBlocks = TAB_KEYS.some((key) => {
    if (!result) return false;
    switch (key) {
      case "words":
        return enableWords && result.blocks.length > 0;
      case "links":
        return enableLinks && result.links.length > 0;
      case "emails":
        return enableEmails && result.emails.length > 0;
      case "phones":
        return enablePhones && result.phones.length > 0;
      default:
        return false;
    }
  });

  const tabItems = useMemo(() => {
    if (!result) return [];
    return TAB_KEYS.filter((key) => {
      switch (key) {
        case "words":
          return enableWords && result.blocks.length > 0;
        case "links":
          return enableLinks && result.links.length > 0;
        case "emails":
          return enableEmails && result.emails.length > 0;
        case "phones":
          return enablePhones && result.phones.length > 0;
        default:
          return false;
      }
    }).map((key) => {
      const label = t(`clipboard:segmentFill.tabs.${key}`);
      let count = 0;
      switch (key) {
        case "words":
          count = result.blocks.length;
          break;
        case "links":
          count = result.links.length;
          break;
        case "emails":
          count = result.emails.length;
          break;
        case "phones":
          count = result.phones.length;
          break;
      }
      return {
        key,
        label: `${label} (${count})`,
      };
    });
  }, [result, enableWords, enableLinks, enableEmails, enablePhones, t]);

  return (
    <Modal
      cancelButtonProps={{ style: { display: "none" } }}
      confirmLoading={filling}
      destroyOnHidden
      okButtonProps={{ disabled: selected.size === 0 }}
      okText={t("clipboard:segmentFill.fill", { count: selected.size })}
      onCancel={onClose}
      onOk={handleFill}
      open={!!item}
      title={t("clipboard:segmentFill.title")}
      width={560}
    >
      {loading ? (
        <div className="h-40 animate-pulse rounded-1 bg-ant-fill-secondary" />
      ) : !result ? (
        <p className="py-6 text-center text-ant-quaternary text-sm">
          {t("clipboard:segmentFill.empty")}
        </p>
      ) : (
        <div className="flex flex-col gap-3">
          {/* 文本编辑区 */}
          <TextArea
            autoSize={{ maxRows: 6, minRows: 2 }}
            onChange={(e) => setEditingText(e.target.value)}
            placeholder={t("clipboard:segmentFill.editPlaceholder")}
            value={editingText}
          />

          {/* 开关区 */}
          <div className="flex flex-wrap gap-x-4 gap-y-1">
            <label
              className="flex cursor-pointer items-center gap-1.5 text-sm"
              htmlFor="toggle-words"
            >
              <Switch
                checked={enableWords}
                id="toggle-words"
                onChange={setEnableWords}
                size="small"
              />
              <span>{t("clipboard:segmentFill.toggles.words")}</span>
            </label>
            <label
              className="flex cursor-pointer items-center gap-1.5 text-sm"
              htmlFor="toggle-links"
            >
              <Switch
                checked={enableLinks}
                id="toggle-links"
                onChange={setEnableLinks}
                size="small"
              />
              <span>{t("clipboard:segmentFill.toggles.links")}</span>
            </label>
            <label
              className="flex cursor-pointer items-center gap-1.5 text-sm"
              htmlFor="toggle-emails"
            >
              <Switch
                checked={enableEmails}
                id="toggle-emails"
                onChange={setEnableEmails}
                size="small"
              />
              <span>{t("clipboard:segmentFill.toggles.emails")}</span>
            </label>
            <label
              className="flex cursor-pointer items-center gap-1.5 text-sm"
              htmlFor="toggle-phones"
            >
              <Switch
                checked={enablePhones}
                id="toggle-phones"
                onChange={setEnablePhones}
                size="small"
              />
              <span>{t("clipboard:segmentFill.toggles.phones")}</span>
            </label>
          </div>

          {/* Tab 切换 */}
          {tabItems.length > 0 && (
            <Tabs
              activeKey={activeTab}
              items={tabItems}
              onChange={(key) => {
                setActiveTab(key as TabKey);
                setSelected(new Set());
              }}
              size="small"
            />
          )}

          {/* 词块展示区 */}
          {hasAnyBlocks ? (
            <div
              className="flex max-h-48 flex-wrap gap-1.5 overflow-y-auto pb-1"
              onPointerMove={handlePointerMove}
            >
              {currentBlocks.map((block, index) => {
                const isActive = selected.has(index);
                return (
                  <button
                    className={cn(
                      "rounded-1 border px-2 py-1 text-sm leading-none transition-colors motion-reduce:transition-none",
                      isActive
                        ? "border-ant-primary bg-ant-blue-1 text-ant-primary"
                        : "border-ant-border bg-ant-container text-ant-text hover:border-ant-primary/60",
                    )}
                    data-block-index={index}
                    key={block.id}
                    onPointerDown={(event) => handlePointerDown(event, index)}
                    onPointerUp={handlePointerUp}
                    type="button"
                  >
                    {block.text}
                  </button>
                );
              })}
            </div>
          ) : (
            <p className="py-4 text-center text-ant-quaternary text-sm">
              {t("clipboard:segmentFill.noResults")}
            </p>
          )}

          {/* 已选预览 */}
          {selectedText ? (
            <div className="break-all rounded-1 border border-ant-border-secondary bg-ant-fill-quaternary px-3 py-2 text-sm">
              {selectedText}
            </div>
          ) : null}
        </div>
      )}
    </Modal>
  );
};

export default SegmentFillModal;
