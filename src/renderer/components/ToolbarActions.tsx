import { LoaderCircle } from "lucide-react";
import { useState, type JSX, type MouseEvent } from "react";

import { MAX_ENABLED_ACTIONS, type ActionDefinition } from "../../shared";
import { ActionIcon } from "./ActionIcon";
import { PopperIcon } from "./PopperIcon";

export function getToolbarActions(actions: readonly ActionDefinition[]): ActionDefinition[] {
  return actions
    .filter((action) => action.enabled && action.kind !== "ask")
    .sort((left, right) => left.order - right.order)
    .slice(0, MAX_ENABLED_ACTIONS);
}

interface ToolbarActionsProps {
  actions: readonly ActionDefinition[];
  preview?: boolean;
  busyActionId?: string | null;
  copySuccessActionId?: string | null;
  hoveredControlId?: string | null;
  onAsk?: (event: MouseEvent<HTMLButtonElement>) => void;
  onAction?: (actionId: string, event: MouseEvent<HTMLButtonElement>) => void;
}

// Both the settings preview and the native toolbar use the same compact controls.
export function ToolbarActions({
  actions,
  preview = false,
  busyActionId = null,
  copySuccessActionId = null,
  hoveredControlId = null,
  onAsk,
  onAction,
}: ToolbarActionsProps): JSX.Element {
  const [previewHoveredId, setPreviewHoveredId] = useState<string | null>(null);
  const hoverProps = (controlId: string) => ({
    "data-hovered": (preview ? previewHoveredId : hoveredControlId) === controlId
      ? "true" : undefined,
    onMouseEnter: preview ? () => setPreviewHoveredId(controlId) : undefined,
    onMouseLeave: preview ? () => setPreviewHoveredId(null) : undefined,
  });

  return (
    <>
      <button
        className="toolbar-action toolbar-action--fixed-ask"
        type="button"
        data-toolbar-control="fixed-ask"
        {...hoverProps("fixed-ask")}
        title="问 AI"
        aria-label="问 AI"
        aria-disabled={preview ? "true" : undefined}
        tabIndex={preview ? -1 : undefined}
        onMouseDown={(event) => event.preventDefault()}
        onClick={preview ? undefined : onAsk}
      >
        <PopperIcon />
      </button>
      {actions.map((action) => {
        const busy = busyActionId === action.id;
        const muted = busyActionId !== null && !busy;
        const copySucceeded = action.kind === "copy" && copySuccessActionId === action.id;
        return (
          <button
            className={`toolbar-action ${muted ? "toolbar-action--muted" : ""}`}
            type="button"
            key={action.id}
            data-toolbar-control={`action:${action.id}`}
            {...hoverProps(`action:${action.id}`)}
            title={action.name}
            aria-label={action.name}
            disabled={busy}
            aria-disabled={preview || muted ? "true" : undefined}
            tabIndex={preview ? -1 : undefined}
            onMouseDown={(event) => event.preventDefault()}
            onClick={preview ? undefined : (event) => onAction?.(action.id, event)}
          >
            {busy ? (
              <LoaderCircle className="spin" size={16} aria-hidden="true" />
            ) : (
              <ActionIcon
                className={copySucceeded ? "toolbar-copy-success" : undefined}
                name={copySucceeded ? "clipboard-check" : action.icon}
                size={16}
                strokeWidth={2}
              />
            )}
            <span>{action.name}</span>
          </button>
        );
      })}
    </>
  );
}
