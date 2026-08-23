import type { FC, MouseEvent } from "react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  extractItemEntities,
  fillSelectedText,
  openEntityLink,
} from "@/commands";
import type { ExtractedEntity } from "@/types/clipboard";
import { cn } from "@/utils/cn";

interface EntityBarProps {
  /**
   * 文本记录 id，用于调用实体提取命令。
   */
  itemId: string;
}

/**
 * 文本卡片下方自动展开的实体下拉框：把长文本里的链接 / 邮箱 / 手机号 / QQ 逐项列出。
 * - 链接：高亮展示，支持「打开」与「键入」
 * - 邮箱 / 手机号 / QQ：支持「键入」（写剪贴板 + 模拟粘贴）或邮箱「打开」
 */
const EntityBar: FC<EntityBarProps> = (props) => {
  const { itemId } = props;

  const [entities, setEntities] = useState<ExtractedEntity[]>([]);

  useEffect(() => {
    let disposed = false;

    extractItemEntities(itemId).then((found) => {
      if (disposed) return;
      setEntities(found);
    });

    return () => {
      disposed = true;
    };
  }, [itemId]);

  if (entities.length === 0) return null;

  return (
    <div className="mt-1 flex flex-wrap gap-1.5">
      {entities.map((entity) => (
        <EntityChip entity={entity} key={`${entity.kind}:${entity.start}`} />
      ))}
    </div>
  );
};

export default EntityBar;

interface EntityChipProps {
  entity: ExtractedEntity;
}

const ENTITY_ICON: Record<ExtractedEntity["kind"], string> = {
  email: "i-lucide:mail",
  phone: "i-lucide:phone",
  qq: "i-lucide:message-circle",
  url: "i-lucide:link",
};

const EntityChip: FC<EntityChipProps> = (props) => {
  const { entity } = props;
  const { t } = useTranslation("clipboard");
  const isUrl = entity.kind === "url";

  const handleOpen = (event: MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    void openEntityLink(entity.kind, entity.value);
  };

  const handleFill = (event: MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    void fillSelectedText(entity.value);
  };

  return (
    <span className="inline-flex max-w-full items-center gap-1 overflow-hidden rounded-1 border border-ant-border-secondary bg-ant-fill-quaternary text-xs">
      <i
        aria-hidden="true"
        className={cn("ml-1.5 size-3.5 shrink-0", ENTITY_ICON[entity.kind])}
      />
      <span
        className={cn("truncate", "font-mono", isUrl && "text-ant-primary")}
        title={entity.value}
      >
        {entity.value}
      </span>
      <span className="flex shrink-0 items-center gap-0.5 px-0.5">
        {isUrl || entity.kind === "email" ? (
          <button
            className="rounded px-1 py-0.5 text-ant-secondary hover:bg-ant-fill-secondary hover:text-ant-primary"
            onClick={handleOpen}
            title={t("entities.open")}
            type="button"
          >
            {t("entities.open")}
          </button>
        ) : null}
        <button
          className="rounded px-1 py-0.5 text-ant-secondary hover:bg-ant-fill-secondary hover:text-ant-primary"
          onClick={handleFill}
          title={t("entities.fill")}
          type="button"
        >
          {t("entities.fill")}
        </button>
      </span>
    </span>
  );
};
