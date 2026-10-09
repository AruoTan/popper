import type { JSX } from "react";

import appIcon from "../../../assets/popper.png";

export function PopperIcon({ size = 24 }: { size?: number }): JSX.Element {
  return (
    <img
      className="popper-icon"
      src={appIcon}
      width={size}
      height={size}
      alt=""
      aria-hidden="true"
      draggable={false}
    />
  );
}
