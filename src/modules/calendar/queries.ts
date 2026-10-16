// React Query hooks of the calendar module. All keys come from `qk` and start
// with "calendar", so backend data-changed events invalidate them.

import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { useBackend } from "../../api/context";
import { qk } from "../../api/queries";
import type { EventDraft, EventResponse } from "../../api/types";

export function useCalendars() {
  const backend = useBackend();
  return useQuery({ queryKey: qk.calendars(), queryFn: () => backend.listCalendars() });
}

/**
 * Cached events overlapping [start, end). Also asks the backend to refresh the
 * range from the servers; fresh results arrive through data-changed events.
 */
export function useEventsInRange(start: Date, end: Date) {
  const backend = useBackend();
  const from = start.toISOString();
  const to = end.toISOString();

  useEffect(() => {
    backend.refreshEvents(from, to).catch(() => {
      // Offline or a provider error: the cached events are still shown and the
      // sync status in the title bar reports account problems.
    });
  }, [backend, from, to]);

  return useQuery({
    queryKey: qk.events(from, to),
    queryFn: () => backend.listEvents(from, to),
    placeholderData: keepPreviousData,
  });
}

function useInvalidateCalendar() {
  const client = useQueryClient();
  return () => client.invalidateQueries({ queryKey: qk.root("calendar") });
}

export function useSaveEvent() {
  const backend = useBackend();
  const invalidate = useInvalidateCalendar();
  return useMutation({
    mutationFn: ({ eventId, draft }: { eventId: string | null; draft: EventDraft }) =>
      eventId ? backend.updateEvent(eventId, draft) : backend.createEvent(draft),
    onSuccess: () => invalidate(),
  });
}

export function useDeleteEvent() {
  const backend = useBackend();
  const invalidate = useInvalidateCalendar();
  return useMutation({
    mutationFn: (eventId: string) => backend.deleteEvent(eventId),
    onSuccess: () => invalidate(),
  });
}

export function useRespondToEvent() {
  const backend = useBackend();
  const invalidate = useInvalidateCalendar();
  return useMutation({
    mutationFn: (args: {
      eventId: string;
      response: EventResponse;
      comment: string | null;
      sendResponse: boolean;
    }) => backend.respondToEvent(args.eventId, args.response, args.comment, args.sendResponse),
    onSuccess: () => invalidate(),
  });
}

/** Address suggestions for the attendee box (shared contacts key). */
export function useAddressSuggestions(text: string) {
  const backend = useBackend();
  const q = text.trim();
  return useQuery({
    queryKey: qk.suggestions(q),
    queryFn: () => backend.suggestAddresses(q),
    enabled: q.length > 0,
    staleTime: 60_000,
  });
}
