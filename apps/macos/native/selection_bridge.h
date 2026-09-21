#ifndef POPPER_SELECTION_BRIDGE_H
#define POPPER_SELECTION_BRIDGE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PopperSelectionMonitor PopperSelectionMonitor;

/**
 * `json_utf8` is valid only for the duration of the callback. The callback may
 * be invoked from a background thread and must return promptly.
 */
typedef void (*PopperSelectionEventCallback)(const char *json_utf8, void *context);

enum PopperSelectionStatus {
    POPPER_SELECTION_OK = 0,
    POPPER_SELECTION_ALREADY_RUNNING = 1,
    POPPER_SELECTION_NOT_RUNNING = 2,
    POPPER_SELECTION_NOT_TRUSTED = -1,
    POPPER_SELECTION_EVENT_TAP_FAILED = -2,
    POPPER_SELECTION_INVALID_ARGUMENT = -3,
    POPPER_SELECTION_INTERNAL_ERROR = -4,
};

uint8_t popper_accessibility_is_trusted(void);

/** Requests macOS Accessibility access. The return value is the current state. */
uint8_t popper_accessibility_request(void);

PopperSelectionMonitor *popper_selection_monitor_create(
    const char *excluded_bundle_id_utf8,
    PopperSelectionEventCallback callback,
    void *context);

int32_t popper_selection_monitor_start(PopperSelectionMonitor *monitor);
int32_t popper_selection_monitor_stop(PopperSelectionMonitor *monitor);

/**
 * Captures the current selection synchronously. The returned UTF-8 JSON string
 * must be released with `popper_selection_string_free`. A null result with
 * status `POPPER_SELECTION_OK` means that no non-empty selection was
 * available.
 */
char *popper_selection_monitor_capture_current(
    PopperSelectionMonitor *monitor,
    int32_t *status_out);

void popper_selection_string_free(char *value);

/**
 * Collapse the current accessibility selection to a caret when the focused
 * element's selected text exactly matches `text_utf8`.
 *
 * `bundle_id_utf8` may be NULL or empty: then any focused app is accepted.
 * When non-empty, the focused application's bundle id must match.
 *
 * Returns 1 if a selection was collapsed, 0 otherwise. Never throws; safe to
 * call from a background thread that then hops to the main thread internally
 * if required by AppKit/AX.
 */
uint8_t popper_selection_clear_matching_text(
    const char *bundle_id_utf8,
    const char *text_utf8);

void popper_selection_monitor_destroy(PopperSelectionMonitor *monitor);

#ifdef __cplusplus
}
#endif

#endif
