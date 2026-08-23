import { Modal } from "antd";
import type { FC, PointerEvent as ReactPointerEvent } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { fillSelectedText, segmentClipboardItem } from "@/commands";
import type { ClipboardItem } from "@/types/clipboard";
import { cn } from "@/utils/cn";

interface SegmentFillModalProps {
  /**
   * 当前拆词目标；为 null 时关闭。由列表层持有的单例状态注入。
   */
  item: ClipboardItem | null;
  /**
   * 关闭弹窗。
   */
  onClose: () => void;
}

/**
 * 「拆词填入」弹窗：对单条文本记录做分词后展示词块流，
 * 支持单击多选不连续词块，或按住并拖动框选连续区间；
 * 已选词块实时预览，点「填入」写剪贴板并模拟粘贴到前台输入框。
 */
const SegmentFillModal: FC<SegmentFillModalProps> = (props) => {
  const { item, onClose } = props;
  const { t } = useTranslation(["clipboard", "common"]);

  const [blocks, setBlocks] = useState<Array<{ id: string; text: string }>>([]);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const [filling, setFilling] = useState(false);
  // 拖动选择手势状态：anchor 为按下时的索引，base 为按下瞬间的已选快照。
  const dragRef = useRef<{
    anchor: number;
    base: ReadonlySet<number>;
    moved: boolean;
  } | null>(null);

  useEffect(() => {
    if (!item) return;

    setSelected(new Set());
    setBlocks([]);
    setLoading(true);

    segmentClipboardItem(item.id)
      .then((words) =>
        setBlocks(
          words.map((word, index) => ({
            id: `${item.id}-${index}`,
            text: word,
          })),
        ),
      )
      .finally(() => setLoading(false));
  }, [item]);

  const selectedText = useMemo(() => {
    return blocks
      .filter((_, index) => selected.has(index))
      .map((block) => block.text)
      .join(" ");
  }, [blocks, selected]);

  const handlePointerDown = (
    event: ReactPointerEvent<HTMLButtonElement>,
    index: number,
  ) => {
    if (event.button !== 0) return;

    event.preventDefault();
    dragRef.current = { anchor: index, base: new Set(selected), moved: false };
  };

  const handlePointerEnter = (
    event: ReactPointerEvent<HTMLButtonElement>,
    index: number,
  ) => {
    const drag = dragRef.current;
    if (!drag?.moved) return;

    event.preventDefault();
    const lo = Math.min(drag.anchor, index);
    const hi = Math.max(drag.anchor, index);
    const next = new Set(drag.base);
    for (let i = lo; i <= hi; i += 1) next.add(i);
    setSelected(next);
  };

  const handlePointerMove = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const drag = dragRef.current;
    if (!drag) return;

    event.preventDefault();
    drag.moved = true;
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

  return (
    <Modal
      afterOpenChange={(open) => {
        // 重新打开展示上次仍生效的选中，保持状态简单：每次打开由 item effect 重置。
        void open;
      }}
      cancelButtonProps={{ style: { display: "none" } }}
      confirmLoading={filling}
      destroyOnHidden
      okButtonProps={{ disabled: selected.size === 0 }}
      okText={t("clipboard:segmentFill.fill", { count: selected.size })}
      onCancel={onClose}
      onOk={handleFill}
      open={!!item}
      title={t("clipboard:segmentFill.title")}
    >
      <p className="mb-3 text-ant-quaternary text-xs">
        {t("clipboard:segmentFill.placeholder")}
      </p>

      {loading ? (
        <div className="h-20 animate-pulse rounded-1 bg-ant-fill-secondary" />
      ) : blocks.length === 0 ? (
        <p className="py-6 text-center text-ant-quaternary text-sm">
          {t("clipboard:segmentFill.empty")}
        </p>
      ) : (
        <div className="flex max-h-64 flex-wrap gap-1.5 overflow-y-auto pb-1">
          {blocks.map((block, index) => {
            const isActive = selected.has(index);
            return (
              <button
                className={cn(
                  "rounded-1 border px-2 py-1 text-sm leading-none transition-colors motion-reduce:transition-none",
                  isActive
                    ? "border-ant-primary bg-ant-blue-1 text-ant-primary"
                    : "border-ant-border bg-ant-container text-ant-text hover:border-ant-primary/60",
                )}
                key={block.id}
                onPointerDown={(event) => handlePointerDown(event, index)}
                onPointerEnter={(event) => handlePointerEnter(event, index)}
                onPointerMove={handlePointerMove}
                onPointerUp={handlePointerUp}
                type="button"
              >
                {block.text}
              </button>
            );
          })}
        </div>
      )}

      {selectedText ? (
        <div className="mt-3 break-all rounded-1 border border-ant-border-secondary bg-ant-fill-quaternary px-3 py-2 text-sm">
          {selectedText}
        </div>
      ) : null}
    </Modal>
  );
};

export default SegmentFillModal;
