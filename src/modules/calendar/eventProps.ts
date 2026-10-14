// Props shared by every rendering of an event (time grid, bars, month).

import clsx from "clsx";
import type { CSSProperties, MouseEvent } from "react";

import type { Calendar, CalendarEvent } from "../../api/types";
import { tintStyle } from "./colors";
import { displayShowAs, isStruck } from "./eventText";

export interface EventSelectProps {
  calendar: Calendar | undefined;
  selected: boolean;
  onSelect: (event: CalendarEvent, element: HTMLElement) => void;
}

/** Classes shared by every rendering of an event. */
export function eventClasses(event: CalendarEvent, selected: boolean): string {
  return clsx(
    "cal-tint",
    `cal-show--${displayShowAs(event)}`,
    isStruck(event) && "cal-ev--struck",
    selected && "cal-ev--selected",
  );
}

export function eventStyle(calendar: Calendar | undefined, extra?: CSSProperties): CSSProperties {
  return { ...tintStyle(calendar?.color), ...extra };
}

/** Click handler that also keeps the pointerdown from starting a drag-create. */
export function selectHandlers(
  event: CalendarEvent,
  onSelect: EventSelectProps["onSelect"],
): {
  onClick: (e: MouseEvent<HTMLElement>) => void;
  onPointerDown: (e: { stopPropagation: () => void }) => void;
} {
  return {
    onClick: (e) => {
      e.stopPropagation();
      onSelect(event, e.currentTarget);
    },
    onPointerDown: (e) => e.stopPropagation(),
  };
}
