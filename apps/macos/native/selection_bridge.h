#ifndef TEXTLENS_SELECTION_BRIDGE_H
#define TEXTLENS_SELECTION_BRIDGE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct TextLensSelectionMonitor TextLensSelectionMonitor;

/**
 * `json_utf8` is valid only for the duration of the callback. The callback may
 * be invoked from a background thread and must return promptly.
 */
typedef void (*TextLensSelectionEventCallback)(const char *json_utf8, void *context);

enum TextLensSelectionStatus {
    TEXTLENS_SELECTION_OK = 0,
    TEXTLENS_SELECTION_ALREADY_RUNNING = 1,
    TEXTLENS_SELECTION_NOT_RUNNING = 2,
    TEXTLENS_SELECTION_NOT_TRUSTED = -1,
    TEXTLENS_SELECTION_EVENT_TAP_FAILED = -2,
    TEXTLENS_SELECTION_INVALID_ARGUMENT = -3,
    TEXTLENS_SELECTION_INTERNAL_ERROR = -4,
};

uint8_t textlens_accessibility_is_trusted(void);

/** Requests macOS Accessibility access. The return value is the current state. */
uint8_t textlens_accessibility_request(void);

TextLensSelectionMonitor *textlens_selection_monitor_create(
    const char *excluded_bundle_id_utf8,
    TextLensSelectionEventCallback callback,
    void *context);

int32_t textlens_selection_monitor_start(TextLensSelectionMonitor *monitor);
int32_t textlens_selection_monitor_stop(TextLensSelectionMonitor *monitor);

/**
 * Captures the current selection synchronously. The returned UTF-8 JSON string
 * must be released with `textlens_selection_string_free`. A null result with
 * status `TEXTLENS_SELECTION_OK` means that no non-empty selection was
 * available.
 */
char *textlens_selection_monitor_capture_current(
    TextLensSelectionMonitor *monitor,
    int32_t *status_out);

void textlens_selection_string_free(char *value);

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
uint8_t textlens_selection_clear_matching_text(
    const char *bundle_id_utf8,
    const char *text_utf8);

void textlens_selection_monitor_destroy(TextLensSelectionMonitor *monitor);

#ifdef __cplusplus
}
#endif

#endif
