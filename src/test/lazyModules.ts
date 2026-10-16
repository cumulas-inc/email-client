/**
 * Loads the modules App shows through React.lazy: Calendar, Contacts and
 * Tasks. A test that renders App and switches to one otherwise waits for its
 * first load inside a findBy or waitFor, and on a busy CI runner that load
 * alone can outlast the wait (Calendar's takes about 2 s on an idle machine).
 * Call it from beforeAll, with a long timeout; once loaded, the lazy
 * components resolve at once.
 */
export async function loadLazyModules() {
  await Promise.all([
    import("../modules/calendar/CalendarModule"),
    import("../modules/people/PeopleModule"),
    import("../modules/tasks/TasksModule"),
  ]);
}
