/**
 * Native macOS selection bridge for Popper.
 *
 * The Accessibility traversal, text-range bounds helpers, and input detection
 * in this file are adapted from selection-hook 2.0.2:
 *
 * Copyright (c) 2025 0xfullex (https://github.com/0xfullex/selection-hook)
 * Licensed under the MIT License. See LICENSE.selection-hook.
 *
 * This implementation exposes a small C ABI and intentionally contains no
 * Node/N-API integration.
 */

#import "selection_bridge.h"

#import <ApplicationServices/ApplicationServices.h>
#import <AppKit/AppKit.h>
#import <Carbon/Carbon.h>
#import <Foundation/Foundation.h>

#include <algorithm>
#include <atomic>
#include <cctype>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <cstdlib>
#include <cstdio>
#include <cstring>
#include <deque>
#include <mutex>
#include <new>
#include <limits>
#include <string>
#include <thread>
#include <unordered_map>
#include <utility>
#include <vector>

namespace {

using Clock = std::chrono::steady_clock;

constexpr double kMinimumDragDistance = 4.0;
constexpr uint64_t kMaximumDragDurationMs = 15'000;
constexpr uint64_t kDoubleClickDurationMs = 500;
constexpr double kDoubleClickDistance = 4.0;
constexpr size_t kMaximumQueuedTasks = 64;
// Bounds every AX round-trip to a single app so one slow/hung process can't
// stall the shared single-threaded capture worker for every other app.
constexpr float kAXMessagingTimeoutSeconds = 0.08f;
// Geometry is optional because the mouse-up point is always a placement
// fallback. Give range-bound queries a smaller budget so a correct text read
// is never held behind a slow PDF/Office layout calculation.
constexpr float kAXGeometryMessagingTimeoutSeconds = 0.03f;
// Selection capture stays below a perceptible delay even when a provider
// responds to individual AX requests but exposes a deep, sparse tree.
constexpr uint64_t kAXCaptureBudgetMs = 260;
constexpr uint64_t kDocumentAXCaptureBudgetMs = 300;
constexpr uint64_t kAXCompatibilityRetryDelayMs = 28;
constexpr uint64_t kAXLateSelectionRetryDelayMs = 24;
// Proactive self-healing check for the event tap, mirroring Windows' hook
// health-check cadence; the reactive re-enable in eventTapCallback only
// fires on the next incoming event, which never arrives if the tap was
// disabled during a quiet period.
constexpr CFTimeInterval kEventTapHealthCheckIntervalSeconds = 5.0;

enum class Trigger {
    Drag,
    DoubleClick,
    ShiftClick,
    Keyboard,
    Manual,
};

enum class SelectionMethod {
    Accessibility,
};

struct SelectionInfo {
    std::string text;
    std::string bundleId;
    std::string appName;
    // Kept private to the native bridge. Some AX providers report this value
    // as selected text when the actual range is unavailable.
    std::string windowTitle;
    SelectionMethod method = SelectionMethod::Accessibility;
    Trigger trigger = Trigger::Manual;
    bool fullscreen = false;

    bool hasBounds = false;
    CGRect bounds = CGRectZero;

    bool hasStartTop = false;
    bool hasStartBottom = false;
    bool hasEndTop = false;
    bool hasEndBottom = false;
    CGPoint startTop = CGPointZero;
    CGPoint startBottom = CGPointZero;
    CGPoint endTop = CGPointZero;
    CGPoint endBottom = CGPointZero;

    bool hasMouseStart = false;
    bool hasMouseEnd = false;
    CGPoint mouseStart = CGPointZero;
    CGPoint mouseEnd = CGPointZero;
    CGPoint mouseCurrent = CGPointZero;
};

enum class TaskKind {
    Capture,
    Dismiss,
};

struct Task {
    TaskKind kind = TaskKind::Dismiss;
    Trigger trigger = Trigger::Manual;
    std::string dismissReason;
    CGPoint mouseStart = CGPointZero;
    CGPoint mouseEnd = CGPointZero;
    CGPoint mouseCurrent = CGPointZero;
    bool hasMouseStart = false;
    bool hasMouseEnd = false;
    int64_t targetPid = 0;
    uint64_t generation = 0;
    uint64_t enqueuedAtMs = 0;
};

static uint64_t MonotonicMilliseconds() {
    return static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::milliseconds>(
        Clock::now().time_since_epoch()).count());
}

// Every AX request is tied to the originating input generation and a single
// wall-clock budget. AX providers can block independently even after their
// application element was configured with a messaging timeout, so checking at
// traversal boundaries is what keeps a stale PDF/Office read from delaying the
// next selection.
struct AccessibilityReadContext {
    const std::atomic<uint64_t> *generation = nullptr;
    uint64_t expectedGeneration = 0;
    uint64_t deadlineMs = 0;

    bool IsCurrent() const {
        return (generation == nullptr ||
                generation->load(std::memory_order_acquire) == expectedGeneration) &&
            MonotonicMilliseconds() < deadlineMs;
    }
};

static void ConfigureAccessibilityElement(AXUIElementRef element) {
    if (element != nullptr) {
        AXUIElementSetMessagingTimeout(element, kAXMessagingTimeoutSeconds);
    }
}

static bool SelectionTimingEnabled() {
    static const bool enabled = [] {
        const char *value = std::getenv("POPPER_SELECTION_TRACE");
        return value != nullptr &&
            (std::strcmp(value, "1") == 0 || std::strcmp(value, "true") == 0 ||
             std::strcmp(value, "TRUE") == 0);
    }();
    return enabled;
}

static void TraceSelectionTiming(const char *stage, uint64_t durationMs) {
    if (SelectionTimingEnabled()) {
        std::fprintf(
            stderr,
            "[selection-timing] stage=%s duration_ms=%llu\n",
            stage,
            static_cast<unsigned long long>(durationMs));
    }
}

static uint64_t TimestampMilliseconds() {
    return static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()).count());
}

static std::string StringFromNSString(NSString *value) {
    if (value == nil) {
        return {};
    }
    NSData *data = [value dataUsingEncoding:NSUTF8StringEncoding allowLossyConversion:NO];
    if (data == nil) {
        return {};
    }
    return std::string(static_cast<const char *>(data.bytes), data.length);
}

static NSString *NSStringFromString(const std::string &value) {
    return [[NSString alloc] initWithBytes:value.data()
                                    length:value.size()
                                  encoding:NSUTF8StringEncoding];
}

static bool StringFromCFString(CFStringRef value, std::string &result) {
    if (value == nullptr) {
        return false;
    }
    CFIndex length = CFStringGetLength(value);
    if (length <= 0) {
        return false;
    }
    CFIndex maximum = CFStringGetMaximumSizeForEncoding(length, kCFStringEncodingUTF8) + 1;
    if (maximum <= 1) {
        return false;
    }
    std::vector<char> buffer(static_cast<size_t>(maximum));
    if (!CFStringGetCString(value, buffer.data(), maximum, kCFStringEncodingUTF8)) {
        return false;
    }
    result.assign(buffer.data());
    return !result.empty();
}

static bool IsBlank(const std::string &value) {
    NSString *string = NSStringFromString(value);
    if (string == nil) {
        return true;
    }
    return [[string stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]] length] == 0;
}

static CGPoint CurrentMousePosition() {
    CGEventRef event = CGEventCreate(nullptr);
    if (event == nullptr) {
        return CGPointZero;
    }
    CGPoint point = CGEventGetLocation(event);
    CFRelease(event);
    return point;
}

static NSRunningApplication *FrontmostApplication() {
    return [NSWorkspace sharedWorkspace].frontmostApplication;
}

static bool IsExcludedApplication(
    NSRunningApplication *application,
    const std::string &excludedBundleId) {
    if (application == nil || application.processIdentifier == getpid()) {
        return true;
    }
    // A missing bundle id (some CLI-launched GUI processes, Wine/JVM-hosted
    // apps) no longer disqualifies an app outright; identity for matching
    // falls back to appName (localizedName) elsewhere in the pipeline.
    NSString *bundle = application.bundleIdentifier;
    if (bundle != nil && bundle.length > 0 && !excludedBundleId.empty() &&
        StringFromNSString(bundle) == excludedBundleId) {
        return true;
    }
    return false;
}

static bool IsReasonableRect(CGRect rect) {
    return std::isfinite(rect.origin.x) && std::isfinite(rect.origin.y) &&
        std::isfinite(rect.size.width) && std::isfinite(rect.size.height) &&
        rect.size.width > 0.0 && rect.size.height > 0.0 &&
        rect.size.width < 200'000.0 && rect.size.height < 200'000.0 &&
        std::abs(rect.origin.x) < 500'000.0 && std::abs(rect.origin.y) < 500'000.0;
}

static void StoreAggregateBounds(SelectionInfo &info, CGRect rect) {
    info.hasBounds = true;
    info.bounds = rect;
    info.hasStartTop = true;
    info.hasStartBottom = true;
    info.hasEndTop = true;
    info.hasEndBottom = true;
    info.startTop = CGPointMake(CGRectGetMinX(rect), CGRectGetMinY(rect));
    info.startBottom = CGPointMake(CGRectGetMinX(rect), CGRectGetMaxY(rect));
    info.endTop = CGPointMake(CGRectGetMaxX(rect), CGRectGetMinY(rect));
    info.endBottom = CGPointMake(CGRectGetMaxX(rect), CGRectGetMaxY(rect));
}

static bool ReadSelectionBounds(
    AXUIElementRef element,
    SelectionInfo &info,
    const AccessibilityReadContext &context) {
    if (element == nullptr || !context.IsCurrent()) {
        return false;
    }

    // The selection text is already validated at this point. Bounds only
    // improve placement, so cap this optional work much more aggressively
    // than the text read itself.
    AXUIElementSetMessagingTimeout(element, kAXGeometryMessagingTimeoutSeconds);

    AXValueRef selectedRangeValue = nullptr;
    AXError error = AXUIElementCopyAttributeValue(
        element,
        kAXSelectedTextRangeAttribute,
        reinterpret_cast<CFTypeRef *>(&selectedRangeValue));
    if (error != kAXErrorSuccess || selectedRangeValue == nullptr) {
        return false;
    }

    CFRange range = CFRangeMake(0, 0);
    bool hasRange = AXValueGetValue(selectedRangeValue, kAXValueTypeCFRange, &range) &&
        range.location >= 0 && range.length > 0 &&
        range.length <= std::numeric_limits<CFIndex>::max() - range.location;
    if (!hasRange) {
        CFRelease(selectedRangeValue);
        return false;
    }

    AXValueRef aggregateBoundsValue = nullptr;
    AXError aggregateError = context.IsCurrent()
        ? AXUIElementCopyParameterizedAttributeValue(
              element,
              kAXBoundsForRangeParameterizedAttribute,
              selectedRangeValue,
              reinterpret_cast<CFTypeRef *>(&aggregateBoundsValue))
        : kAXErrorFailure;
    CGRect aggregateRect = CGRectZero;
    if (aggregateError == kAXErrorSuccess && aggregateBoundsValue != nullptr &&
        AXValueGetValue(aggregateBoundsValue, kAXValueTypeCGRect, &aggregateRect) &&
        IsReasonableRect(aggregateRect)) {
        StoreAggregateBounds(info, aggregateRect);
    }
    if (aggregateBoundsValue != nullptr) CFRelease(aggregateBoundsValue);

    CFRelease(selectedRangeValue);
    return info.hasBounds;
}

static bool ReadSelectedText(
    AXUIElementRef element,
    std::string &text,
    const AccessibilityReadContext &context) {
    if (element == nullptr || !context.IsCurrent()) {
        return false;
    }

    ConfigureAccessibilityElement(element);

    CFTypeRef selectedValue = nullptr;
    AXError error = AXUIElementCopyAttributeValue(element, kAXSelectedTextAttribute, &selectedValue);
    if (error == kAXErrorSuccess && selectedValue != nullptr) {
        bool success = CFGetTypeID(selectedValue) == CFStringGetTypeID() &&
            StringFromCFString(static_cast<CFStringRef>(selectedValue), text) && !IsBlank(text);
        CFRelease(selectedValue);
        if (success) {
            return true;
        }
    }

    if (!context.IsCurrent()) {
        return false;
    }

    CFTypeRef value = nullptr;
    error = AXUIElementCopyAttributeValue(element, kAXValueAttribute, &value);
    if (error != kAXErrorSuccess || value == nullptr) {
        return false;
    }
    if (CFGetTypeID(value) != CFStringGetTypeID()) {
        CFRelease(value);
        return false;
    }

    if (!context.IsCurrent()) {
        CFRelease(value);
        return false;
    }

    AXValueRef rangeValue = nullptr;
    error = AXUIElementCopyAttributeValue(
        element,
        kAXSelectedTextRangeAttribute,
        reinterpret_cast<CFTypeRef *>(&rangeValue));
    if (error != kAXErrorSuccess || rangeValue == nullptr) {
        CFRelease(value);
        return false;
    }

    CFRange range = CFRangeMake(0, 0);
    CFIndex valueLength = CFStringGetLength(static_cast<CFStringRef>(value));
    bool validRange = AXValueGetValue(rangeValue, kAXValueTypeCFRange, &range) &&
        range.location >= 0 && range.length > 0 && range.location < valueLength;
    if (validRange && range.length > valueLength - range.location) {
        range.length = valueLength - range.location;
    }

    bool success = false;
    if (validRange && range.length > 0) {
        CFStringRef substring = CFStringCreateWithSubstring(
            kCFAllocatorDefault,
            static_cast<CFStringRef>(value),
            range);
        if (substring != nullptr) {
            success = StringFromCFString(substring, text) && !IsBlank(text);
            CFRelease(substring);
        }
    }

    CFRelease(rangeValue);
    CFRelease(value);
    return success;
}

// Forward declaration because the application routing table is declared with
// the other capture policy helpers below.
static bool IsDocumentOrOfficeSelectionApplication(
    const std::string &bundleId,
    const std::string &appName);

static void ClearSelectionGeometry(SelectionInfo &info) {
    info.hasBounds = false;
    info.bounds = CGRectZero;
    info.hasStartTop = false;
    info.hasStartBottom = false;
    info.hasEndTop = false;
    info.hasEndBottom = false;
    info.startTop = CGPointZero;
    info.startBottom = CGPointZero;
    info.endTop = CGPointZero;
    info.endBottom = CGPointZero;
}

static std::string NormalizeSelectionIdentity(const std::string &value) {
    std::string normalized;
    normalized.reserve(value.size());
    for (unsigned char character : value) {
        if (std::isspace(character) != 0) {
            continue;
        }
        normalized.push_back(static_cast<char>(std::tolower(character)));
    }
    return normalized;
}

static bool WindowTitleComponentMatches(
    const std::string &title,
    const std::string &normalizedText) {
    const std::string normalizedTitle = NormalizeSelectionIdentity(title);
    if (normalizedTitle == normalizedText) {
        return true;
    }
    static const char *const separators[] = {" - ", " | ", " -- "};
    for (const char *separator : separators) {
        size_t start = 0;
        const std::string delimiter(separator);
        while (start <= title.size()) {
            size_t end = title.find(delimiter, start);
            std::string component = title.substr(start, end == std::string::npos ? end : end - start);
            if (!component.empty() && NormalizeSelectionIdentity(component) == normalizedText) {
                return true;
            }
            if (end == std::string::npos) {
                break;
            }
            start = end + delimiter.size();
        }
    }
    return false;
}

static bool SelectionTextMatchesSourceIdentity(const SelectionInfo &info) {
    if (info.text.empty() || info.text.size() > 2048) {
        return false;
    }
    const std::string normalizedText = NormalizeSelectionIdentity(info.text);
    if (normalizedText.empty()) {
        return false;
    }

    for (const std::string &candidate : {info.appName, info.bundleId}) {
        if (!candidate.empty() && NormalizeSelectionIdentity(candidate) == normalizedText) {
            return true;
        }
        const size_t separator = candidate.find_last_of("./");
        if (separator != std::string::npos &&
            NormalizeSelectionIdentity(candidate.substr(separator + 1)) == normalizedText) {
            return true;
        }
    }
    return !info.windowTitle.empty() &&
        WindowTitleComponentMatches(info.windowTitle, normalizedText);
}

static bool PointNearSelectionBounds(CGPoint point, CGRect bounds) {
    constexpr CGFloat tolerance = 56.0;
    return point.x >= CGRectGetMinX(bounds) - tolerance &&
        point.x <= CGRectGetMaxX(bounds) + tolerance &&
        point.y >= CGRectGetMinY(bounds) - tolerance &&
        point.y <= CGRectGetMaxY(bounds) + tolerance;
}

static bool SelectionCandidateMatchesInput(const SelectionInfo &candidate) {
    if (SelectionTextMatchesSourceIdentity(candidate)) {
        return false;
    }
    if (candidate.trigger == Trigger::Keyboard || candidate.trigger == Trigger::Manual) {
        return true;
    }
    if (!candidate.hasBounds) {
        // Some PDF and Office providers expose a real AXSelectedText range
        // but do not implement AXBoundsForRange. The text was read from an
        // explicit selection attribute and host/window-title values have
        // already been filtered above, so keep it rather than falling back to
        // a destructive fallback.
        return true;
    }
    return PointNearSelectionBounds(candidate.mouseCurrent, candidate.bounds) ||
        (candidate.hasMouseEnd && PointNearSelectionBounds(candidate.mouseEnd, candidate.bounds));
}

static bool ReadValidatedSelectedText(
    AXUIElementRef element,
    SelectionInfo &info,
    const AccessibilityReadContext &context) {
    std::string text;
    if (!ReadSelectedText(element, text, context)) {
        return false;
    }

    SelectionInfo candidate = info;
    ClearSelectionGeometry(candidate);
    candidate.text = std::move(text);
    ReadSelectionBounds(element, candidate, context);
    if (!SelectionCandidateMatchesInput(candidate)) {
        return false;
    }
    info = std::move(candidate);
    return true;
}

static bool FindSelectionInTree(
    AXUIElementRef element,
    SelectionInfo &info,
    int depth,
    size_t &remainingElements,
    const AccessibilityReadContext &context) {
    if (element == nullptr || depth < 0 || remainingElements == 0 || !context.IsCurrent()) {
        return false;
    }
    --remainingElements;

    ConfigureAccessibilityElement(element);

    if (ReadValidatedSelectedText(element, info, context)) {
        return true;
    }

    if (depth == 0) {
        return false;
    }
    CFArrayRef children = nullptr;
    AXError error = AXUIElementCopyAttributeValue(
        element,
        kAXChildrenAttribute,
        reinterpret_cast<CFTypeRef *>(&children));
    if (error != kAXErrorSuccess || children == nullptr) {
        return false;
    }

    bool found = false;
    CFIndex count = std::min<CFIndex>(CFArrayGetCount(children), 64);
    for (CFIndex index = 0;
         index < count && !found && remainingElements > 0 && context.IsCurrent();
         ++index) {
        AXUIElementRef child = static_cast<AXUIElementRef>(
            const_cast<void *>(CFArrayGetValueAtIndex(children, index)));
        found = FindSelectionInTree(child, info, depth - 1, remainingElements, context);
    }
    CFRelease(children);
    return found;
}

static bool FindSelectionInChildren(
    AXUIElementRef element,
    SelectionInfo &info,
    int depth,
    size_t &remainingElements,
    const AccessibilityReadContext &context) {
    if (element == nullptr || depth <= 0 || remainingElements == 0 || !context.IsCurrent()) {
        return false;
    }

    ConfigureAccessibilityElement(element);
    CFArrayRef children = nullptr;
    AXError error = AXUIElementCopyAttributeValue(
        element,
        kAXChildrenAttribute,
        reinterpret_cast<CFTypeRef *>(&children));
    if (error != kAXErrorSuccess || children == nullptr) {
        return false;
    }

    bool found = false;
    CFIndex count = std::min<CFIndex>(CFArrayGetCount(children), 64);
    for (CFIndex index = 0;
         index < count && !found && remainingElements > 0 && context.IsCurrent();
         ++index) {
        AXUIElementRef child = static_cast<AXUIElementRef>(
            const_cast<void *>(CFArrayGetValueAtIndex(children, index)));
        found = FindSelectionInTree(child, info, depth - 1, remainingElements, context);
    }
    CFRelease(children);
    return found;
}

static AXUIElementRef CopyFocusedElement(AXUIElementRef applicationElement) {
    AXUIElementRef element = nullptr;
    if (applicationElement != nullptr) {
        AXUIElementCopyAttributeValue(
            applicationElement,
            kAXFocusedUIElementAttribute,
            reinterpret_cast<CFTypeRef *>(&element));
    }
    return element;
}

static AXUIElementRef CopyFocusedWindow(AXUIElementRef applicationElement) {
    AXUIElementRef window = nullptr;
    if (applicationElement != nullptr) {
        AXUIElementCopyAttributeValue(
            applicationElement,
            kAXFocusedWindowAttribute,
            reinterpret_cast<CFTypeRef *>(&window));
    }
    return window;
}

static void ReadWindowTitle(
    AXUIElementRef window,
    std::string &title,
    const AccessibilityReadContext &context) {
    if (window == nullptr || !context.IsCurrent()) {
        return;
    }
    AXUIElementSetMessagingTimeout(window, kAXGeometryMessagingTimeoutSeconds);
    CFTypeRef value = nullptr;
    if (AXUIElementCopyAttributeValue(window, kAXTitleAttribute, &value) != kAXErrorSuccess ||
        value == nullptr) {
        return;
    }
    if (context.IsCurrent() && CFGetTypeID(value) == CFStringGetTypeID()) {
        std::string candidate;
        if (StringFromCFString(static_cast<CFStringRef>(value), candidate) &&
            !IsBlank(candidate)) {
            title = std::move(candidate);
        }
    }
    CFRelease(value);
}

static bool FindSelectionFromFocusedContext(
    AXUIElementRef element,
    SelectionInfo &info,
    const AccessibilityReadContext &context) {
    if (element == nullptr || !context.IsCurrent()) {
        return false;
    }

    if (ReadValidatedSelectedText(element, info, context)) {
        return true;
    }

    AXUIElementRef current = element;
    CFRetain(current);
    bool found = false;
    for (int level = 0; level < 10 && !found && context.IsCurrent(); ++level) {
        ConfigureAccessibilityElement(current);
        AXUIElementRef parent = nullptr;
        AXError error = AXUIElementCopyAttributeValue(
            current,
            kAXParentAttribute,
            reinterpret_cast<CFTypeRef *>(&parent));
        CFRelease(current);
        current = nullptr;
        if (error != kAXErrorSuccess || parent == nullptr) {
            break;
        }
        current = parent;
        if (ReadValidatedSelectedText(current, info, context)) {
            found = true;
        }
    }
    if (current != nullptr) CFRelease(current);
    if (found || !context.IsCurrent()) {
        return found;
    }

    // Direct focused/parent reads cover native editors and most Office/PDF
    // applications. Only then descend into a bounded child tree for WebKit,
    // Chromium, and custom canvas accessibility hierarchies.
    size_t remaining = 96;
    return FindSelectionInChildren(element, info, 4, remaining, context);
}

static bool ReadFullscreen(
    AXUIElementRef applicationElement,
    const AccessibilityReadContext &context) {
    if (applicationElement == nullptr || !context.IsCurrent()) {
        return false;
    }
    AXUIElementRef window = CopyFocusedWindow(applicationElement);
    if (window == nullptr) {
        return false;
    }
    AXUIElementSetMessagingTimeout(window, kAXGeometryMessagingTimeoutSeconds);
    CFTypeRef value = nullptr;
    AXError error = AXUIElementCopyAttributeValue(window, CFSTR("AXFullScreen"), &value);
    bool fullscreen = context.IsCurrent() && error == kAXErrorSuccess && value != nullptr &&
        CFGetTypeID(value) == CFBooleanGetTypeID() &&
        CFBooleanGetValue(static_cast<CFBooleanRef>(value));
    if (value != nullptr) CFRelease(value);
    CFRelease(window);
    return fullscreen;
}

static bool ElementIsProtected(
    AXUIElementRef element,
    const AccessibilityReadContext &context) {
    if (element == nullptr || !context.IsCurrent()) {
        return true;
    }
    ConfigureAccessibilityElement(element);

    bool isProtected = false;
    CFTypeRef subrole = nullptr;
    if (AXUIElementCopyAttributeValue(element, kAXSubroleAttribute, &subrole) == kAXErrorSuccess &&
        subrole != nullptr) {
        isProtected = CFGetTypeID(subrole) == CFStringGetTypeID() &&
            CFStringCompare(
                static_cast<CFStringRef>(subrole),
                CFSTR("AXSecureTextField"),
                0) == kCFCompareEqualTo;
        CFRelease(subrole);
    }
    if (isProtected || !context.IsCurrent()) {
        return true;
    }

    CFTypeRef protectedContent = nullptr;
    if (AXUIElementCopyAttributeValue(element, CFSTR("AXProtectedContent"), &protectedContent) ==
            kAXErrorSuccess &&
        protectedContent != nullptr) {
        isProtected = CFGetTypeID(protectedContent) == CFBooleanGetTypeID() &&
            CFBooleanGetValue(static_cast<CFBooleanRef>(protectedContent));
        CFRelease(protectedContent);
    }
    return isProtected || !context.IsCurrent();
}

constexpr uint8_t kAXEnhancedUserInterfaceApplied = 1u << 0;
constexpr uint8_t kAXManualAccessibilityApplied = 1u << 1;
constexpr uint8_t kAXCompatibilityAttributesApplied =
    kAXEnhancedUserInterfaceApplied | kAXManualAccessibilityApplied;

static uint64_t ProcessLaunchToken(NSRunningApplication *application) {
    if (application == nil || application.launchDate == nil) {
        return 0;
    }
    NSTimeInterval seconds = application.launchDate.timeIntervalSince1970;
    if (!std::isfinite(seconds) || seconds <= 0.0) {
        return 0;
    }
    return static_cast<uint64_t>(std::llround(seconds * 1'000'000.0));
}

// Tracks which AX compatibility attributes were successfully written. The
// launch token prevents a recycled PID from inheriting another app's state,
// and each attribute is retried independently when its write fails.
class EnhancedUiCache {
public:
    uint8_t Attributes(pid_t pid, uint64_t launchToken) {
        // Without a launch identity, a PID-only hit is unsafe because macOS
        // can recycle PIDs. Retry the compatibility writes for such apps
        // instead of carrying state across process lifetimes.
        if (launchToken == 0) {
            return 0;
        }
        std::lock_guard<std::mutex> lock(mutex_);
        auto it = appliedPids_.find(pid);
        if (it == appliedPids_.end() || it->second.launchToken != launchToken) {
            if (it != appliedPids_.end()) {
                appliedPids_.erase(it);
            }
            return 0;
        }
        return it->second.attributes;
    }

    void MarkApplied(pid_t pid, uint64_t launchToken, uint8_t attributes) {
        if (attributes == 0 || launchToken == 0) {
            return;
        }
        std::lock_guard<std::mutex> lock(mutex_);
        auto it = appliedPids_.find(pid);
        if (it == appliedPids_.end() || it->second.launchToken != launchToken) {
            if (appliedPids_.size() >= kMaxEntries) {
                appliedPids_.clear();
            }
            appliedPids_[pid] = CachedProcessState{launchToken, attributes};
            return;
        }
        it->second.attributes |= attributes;
    }

private:
    struct CachedProcessState {
        uint64_t launchToken;
        uint8_t attributes;
    };

    static constexpr size_t kMaxEntries = 256;
    std::mutex mutex_;
    std::unordered_map<pid_t, CachedProcessState> appliedPids_;
};

static bool ReadViaAccessibility(
    NSRunningApplication *application,
    SelectionInfo &info,
    uint8_t appliedAttributes,
    uint8_t &compatibilityAttributesJustApplied,
    const AccessibilityReadContext &context) {
    compatibilityAttributesJustApplied = 0;
    if (application == nil || !context.IsCurrent()) {
        return false;
    }
    AXUIElementRef applicationElement = AXUIElementCreateApplication(application.processIdentifier);
    if (applicationElement == nullptr) {
        return false;
    }
    ConfigureAccessibilityElement(applicationElement);

    AXUIElementRef focused = CopyFocusedElement(applicationElement);
    if (focused == nullptr) {
        focused = CopyFocusedWindow(applicationElement);
    }
    AXUIElementRef focusedWindow = CopyFocusedWindow(applicationElement);
    ReadWindowTitle(focusedWindow, info.windowTitle, context);

    const bool protectedFocus = focused != nullptr && ElementIsProtected(focused, context);
    bool found = false;
    if (focused != nullptr && !protectedFocus) {
        found = FindSelectionFromFocusedContext(focused, info, context);
    }
    if (!found && !protectedFocus && context.IsCurrent() && focusedWindow != nullptr &&
        (focused == nullptr || !CFEqual(focusedWindow, focused))) {
        size_t remaining = 256;
        found = FindSelectionInTree(focusedWindow, info, 5, remaining, context);
    }

    if (!protectedFocus && !found && context.IsCurrent() &&
        appliedAttributes != kAXCompatibilityAttributesApplied) {
        // Chromium/Electron often does not expose its AX tree until one of
        // these attributes is enabled. Cache only successful writes so a
        // transient AX timeout does not permanently suppress compatibility
        // setup for that process.
        if ((appliedAttributes & kAXEnhancedUserInterfaceApplied) == 0 &&
            AXUIElementSetAttributeValue(
                applicationElement,
                CFSTR("AXEnhancedUserInterface"),
                kCFBooleanTrue) == kAXErrorSuccess) {
            compatibilityAttributesJustApplied |= kAXEnhancedUserInterfaceApplied;
        }
        if ((appliedAttributes & kAXManualAccessibilityApplied) == 0 &&
            AXUIElementSetAttributeValue(
                applicationElement,
                CFSTR("AXManualAccessibility"),
                kCFBooleanTrue) == kAXErrorSuccess) {
            compatibilityAttributesJustApplied |= kAXManualAccessibilityApplied;
        }
    }

    if (found && context.IsCurrent()) {
        info.fullscreen = ReadFullscreen(applicationElement, context);
        if (context.IsCurrent()) {
            info.method = SelectionMethod::Accessibility;
        } else {
            found = false;
        }
    } else if (!context.IsCurrent()) {
        found = false;
    }
    if (focused != nullptr) CFRelease(focused);
    if (focusedWindow != nullptr) CFRelease(focusedWindow);
    CFRelease(applicationElement);
    return found;
}

static std::string LowercaseAscii(std::string value) {
    std::transform(value.begin(), value.end(), value.begin(), [](unsigned char character) {
        return static_cast<char>(std::tolower(character));
    });
    return value;
}

static bool IsWpsSelectionApplication(
    const std::string &normalizedBundleId,
    const std::string &normalizedAppName) {
    // Regional / channel variants use several Kingsoft / WPS identifiers.
    if (normalizedBundleId.rfind("com.kingsoft.", 0) == 0 ||
        normalizedBundleId.rfind("cn.wps.", 0) == 0 ||
        normalizedBundleId.rfind("com.wps.", 0) == 0 ||
        normalizedBundleId.find("wpsoffice") != std::string::npos ||
        normalizedBundleId.find("wps-office") != std::string::npos) {
        return true;
    }
    // Fallback when bundle id is missing or atypical but the display name is
    // clearly a WPS product (Writer / Spreadsheets / Presentation / PDF).
    return normalizedAppName.find("wps") != std::string::npos ||
        normalizedAppName.find("wpsoffice") != std::string::npos ||
        normalizedAppName.find("kingsoft") != std::string::npos;
}

static bool IsDocumentOrOfficeSelectionApplication(
    const std::string &bundleId,
    const std::string &appName) {
    const std::string normalizedBundleId = LowercaseAscii(bundleId);
    const std::string normalizedAppName = LowercaseAscii(appName);
    if (IsWpsSelectionApplication(normalizedBundleId, normalizedAppName)) {
        return true;
    }
    static const char *const prefixes[] = {
        "com.adobe.",
        "com.microsoft.word",
        "com.microsoft.excel",
        "com.microsoft.powerpoint",
        "com.apple.preview",
        "com.apple.iwork.pages",
        "com.apple.iwork.numbers",
        "com.apple.iwork.keynote",
        "org.libreoffice.",
        "org.openoffice.",
    };
    for (const char *prefix : prefixes) {
        if (normalizedBundleId.rfind(prefix, 0) == 0) {
            return true;
        }
    }
    return normalizedAppName.find("acrobat") != std::string::npos ||
        normalizedAppName.find("preview") != std::string::npos ||
        normalizedAppName == "word" ||
        normalizedAppName == "excel" ||
        normalizedAppName == "powerpoint" ||
        normalizedAppName.find("libreoffice") != std::string::npos;
}

static bool CaptureSelection(
    const std::string &excludedBundleId,
    Trigger trigger,
    CGPoint mouseStart,
    bool hasMouseStart,
    CGPoint mouseEnd,
    bool hasMouseEnd,
    CGPoint currentMouse,
    SelectionInfo &info,
    EnhancedUiCache &enhancedUiCache,
    const std::atomic<uint64_t> *captureGeneration = nullptr,
    uint64_t expectedGeneration = 0) {
    @autoreleasepool {
        if (!AXIsProcessTrusted()) {
            return false;
        }
        NSRunningApplication *application = FrontmostApplication();
        if (IsExcludedApplication(application, excludedBundleId)) {
            return false;
        }

        info.bundleId = StringFromNSString(application.bundleIdentifier);
        info.appName = StringFromNSString(application.localizedName);
        info.trigger = trigger;
        info.mouseStart = mouseStart;
        info.mouseEnd = mouseEnd;
        info.mouseCurrent = currentMouse;
        info.hasMouseStart = hasMouseStart;
        info.hasMouseEnd = hasMouseEnd;

        pid_t pid = application.processIdentifier;
        uint64_t launchToken = ProcessLaunchToken(application);
        uint8_t appliedAttributes = enhancedUiCache.Attributes(pid, launchToken);
        uint8_t compatibilityAttributesJustApplied = 0;
        const bool documentOrOffice = IsDocumentOrOfficeSelectionApplication(
            info.bundleId,
            info.appName);
        const uint64_t budgetMs = documentOrOffice
            ? kDocumentAXCaptureBudgetMs
            : kAXCaptureBudgetMs;
        const AccessibilityReadContext context{
            captureGeneration,
            expectedGeneration,
            MonotonicMilliseconds() + budgetMs,
        };
        if (!context.IsCurrent()) return false;

        bool found = ReadViaAccessibility(
            application,
            info,
            appliedAttributes,
            compatibilityAttributesJustApplied,
            context);
        const bool shouldRetry = !found && context.IsCurrent() &&
            (compatibilityAttributesJustApplied != 0 || documentOrOffice);
        if (shouldRetry) {
            // Chromium/Electron needs a short turn after compatibility
            // attributes are set. Document canvases likewise commonly publish
            // their selection a frame after mouse-up. Keep the retry
            // cancellable and inside this request's existing AX budget.
            const uint64_t retryDelay = compatibilityAttributesJustApplied != 0
                ? kAXCompatibilityRetryDelayMs
                : kAXLateSelectionRetryDelayMs;
            const uint64_t retryAt = std::min(
                context.deadlineMs,
                MonotonicMilliseconds() + retryDelay);
            while (context.IsCurrent() && MonotonicMilliseconds() < retryAt) {
                std::this_thread::sleep_for(std::chrono::milliseconds(4));
            }
            if (context.IsCurrent()) {
                uint8_t retryJustApplied = 0;
                found = ReadViaAccessibility(
                    application,
                    info,
                    static_cast<uint8_t>(appliedAttributes | compatibilityAttributesJustApplied),
                    retryJustApplied,
                    context);
                compatibilityAttributesJustApplied |= retryJustApplied;
            }
        }
        if (compatibilityAttributesJustApplied != 0) {
            enhancedUiCache.MarkApplied(pid, launchToken, compatibilityAttributesJustApplied);
        }
        return found && context.IsCurrent() && !IsBlank(info.text);
    }
}

static NSDictionary *PointObject(CGPoint point) {
    return @{ @"x": @(point.x), @"y": @(point.y) };
}

static id OptionalPoint(bool present, CGPoint point) {
    return present ? PointObject(point) : [NSNull null];
}

static NSString *TriggerName(Trigger trigger) {
    switch (trigger) {
        case Trigger::Drag: return @"drag";
        case Trigger::DoubleClick: return @"doubleClick";
        case Trigger::ShiftClick: return @"shiftClick";
        case Trigger::Keyboard: return @"keyboard";
        case Trigger::Manual: return @"manual";
    }
}

static NSString *SelectionDirection(const SelectionInfo &info) {
    if (!info.hasMouseStart || !info.hasMouseEnd) {
        return @"unknown";
    }
    double deltaY = info.mouseEnd.y - info.mouseStart.y;
    double deltaX = info.mouseEnd.x - info.mouseStart.x;
    if (std::abs(deltaY) > 2.0) {
        return deltaY > 0 ? @"forward" : @"backward";
    }
    if (std::abs(deltaX) > 0.5) {
        return deltaX > 0 ? @"forward" : @"backward";
    }
    return @"unknown";
}

static std::string SerializeJSONObject(NSDictionary *object) {
    NSError *error = nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:object options:0 error:&error];
    if (data == nil || error != nil) {
        return {};
    }
    return std::string(static_cast<const char *>(data.bytes), data.length);
}

static std::string SelectionJSON(const SelectionInfo &info) {
    NSString *text = NSStringFromString(info.text);
    NSString *bundle = NSStringFromString(info.bundleId);
    NSString *name = NSStringFromString(info.appName);
    if (text == nil || bundle == nil) {
        return {};
    }

    id bounds = [NSNull null];
    if (info.hasBounds) {
        bounds = @{
            @"x": @(info.bounds.origin.x),
            @"y": @(info.bounds.origin.y),
            @"width": @(info.bounds.size.width),
            @"height": @(info.bounds.size.height),
        };
    }

    NSDictionary *object = @{
        @"type": @"selection",
        @"text": text,
        @"sourceApp": @{
            @"bundleId": bundle,
            @"name": name ?: bundle,
        },
        @"bounds": bounds,
        @"startTop": OptionalPoint(info.hasStartTop, info.startTop),
        @"startBottom": OptionalPoint(info.hasStartBottom, info.startBottom),
        @"endTop": OptionalPoint(info.hasEndTop, info.endTop),
        @"endBottom": OptionalPoint(info.hasEndBottom, info.endBottom),
        @"mouse": @{
            @"start": OptionalPoint(info.hasMouseStart, info.mouseStart),
            @"end": OptionalPoint(info.hasMouseEnd, info.mouseEnd),
            @"current": PointObject(info.mouseCurrent),
        },
        @"direction": SelectionDirection(info),
        @"isFullscreen": @(info.fullscreen),
        @"method": @"accessibility",
        @"trigger": TriggerName(info.trigger),
        @"timestampMs": @(TimestampMilliseconds()),
    };
    return SerializeJSONObject(object);
}

static std::string DismissJSON(const Task &task) {
    NSString *reason = NSStringFromString(task.dismissReason);
    if (reason == nil) {
        return {};
    }
    return SerializeJSONObject(@{
        @"type": @"dismiss",
        @"reason": reason,
        @"mouse": PointObject(task.mouseCurrent),
        @"targetPid": @(task.targetPid),
        @"timestampMs": @(TimestampMilliseconds()),
    });
}

static bool IsKeyboardSelectionKey(CGKeyCode keyCode) {
    switch (keyCode) {
        case kVK_LeftArrow:
        case kVK_RightArrow:
        case kVK_UpArrow:
        case kVK_DownArrow:
        case kVK_Home:
        case kVK_End:
        case kVK_PageUp:
        case kVK_PageDown:
            return true;
        default:
            return false;
    }
}

static bool IsModifierKey(CGKeyCode keyCode) {
    switch (keyCode) {
        case kVK_Shift:
        case kVK_RightShift:
        case kVK_Command:
        case kVK_RightCommand:
        case kVK_Control:
        case kVK_RightControl:
        case kVK_Option:
        case kVK_RightOption:
        case kVK_CapsLock:
        case kVK_Function:
            return true;
        default:
            return false;
    }
}

// Collapse the focused AX selection to a caret at the end of the former range
// when the selected text still exactly matches `expectedText`. Optional
// `requiredBundleId` rejects apps that are not the original host.
static uint8_t ClearMatchingTextOnMainThread(
    const std::string &requiredBundleId,
    const std::string &expectedText) {
    @autoreleasepool {
        if (expectedText.empty() || !AXIsProcessTrusted()) {
            return 0;
        }

        NSRunningApplication *application = FrontmostApplication();
        if (application == nil || application.processIdentifier == getpid()) {
            return 0;
        }

        std::string appBundleId = StringFromNSString(application.bundleIdentifier);
        if (!requiredBundleId.empty() && appBundleId != requiredBundleId) {
            return 0;
        }

        AXUIElementRef applicationElement =
            AXUIElementCreateApplication(application.processIdentifier);
        if (applicationElement == nullptr) {
            return 0;
        }
        AXUIElementSetMessagingTimeout(applicationElement, kAXMessagingTimeoutSeconds);

        AXUIElementRef focused = CopyFocusedElement(applicationElement);
        CFRelease(applicationElement);
        if (focused == nullptr) {
            return 0;
        }

        const AccessibilityReadContext context{
            nullptr,
            0,
            MonotonicMilliseconds() +
                static_cast<uint64_t>(kAXMessagingTimeoutSeconds * 1'000.0f),
        };
        std::string current;
        if (!ReadSelectedText(focused, current, context) || current != expectedText) {
            CFRelease(focused);
            return 0;
        }

        AXValueRef selectedRangeValue = nullptr;
        AXError error = AXUIElementCopyAttributeValue(
            focused,
            kAXSelectedTextRangeAttribute,
            reinterpret_cast<CFTypeRef *>(&selectedRangeValue));
        if (error != kAXErrorSuccess || selectedRangeValue == nullptr) {
            CFRelease(focused);
            return 0;
        }

        CFRange range = CFRangeMake(0, 0);
        bool hasRange = AXValueGetValue(selectedRangeValue, kAXValueTypeCFRange, &range) &&
            range.location >= 0 &&
            range.length > 0 &&
            range.length <= std::numeric_limits<CFIndex>::max() - range.location;
        CFRelease(selectedRangeValue);
        if (!hasRange) {
            CFRelease(focused);
            return 0;
        }

        // Caret at the end of the former selection (natural "deselect after read").
        CFRange collapsed = CFRangeMake(range.location + range.length, 0);
        AXValueRef collapsedValue = AXValueCreate(kAXValueTypeCFRange, &collapsed);
        if (collapsedValue == nullptr) {
            CFRelease(focused);
            return 0;
        }

        error = AXUIElementSetAttributeValue(
            focused,
            kAXSelectedTextRangeAttribute,
            collapsedValue);
        CFRelease(collapsedValue);
        CFRelease(focused);
        return error == kAXErrorSuccess ? 1 : 0;
    }
}

} // namespace

struct PopperSelectionMonitor {
    explicit PopperSelectionMonitor(
        std::string excludedBundle,
        PopperSelectionEventCallback eventCallback,
        void *eventContext)
        : excludedBundleId(std::move(excludedBundle)),
          callback(eventCallback),
          callbackContext(eventContext) {}

    ~PopperSelectionMonitor() {
        stop();
        std::lock_guard<std::mutex> lock(callbackMutex);
        callback = nullptr;
        callbackContext = nullptr;
    }

    int32_t start() {
        std::lock_guard<std::mutex> lifecycleLock(lifecycleMutex);
        if (running.load(std::memory_order_acquire)) {
            return POPPER_SELECTION_ALREADY_RUNNING;
        }
        if (!AXIsProcessTrusted()) {
            return POPPER_SELECTION_NOT_TRUSTED;
        }

        {
            std::lock_guard<std::mutex> startupLock(startupMutex);
            startupReady = false;
            startupStatus = POPPER_SELECTION_INTERNAL_ERROR;
        }
        {
            std::lock_guard<std::mutex> runLoopLock(runLoopMutex);
            eventRunLoop = nullptr;
        }
        {
            std::lock_guard<std::mutex> taskLock(taskMutex);
            tasks.clear();
        }
        {
            std::lock_guard<std::mutex> dismissLock(dismissMutex);
            dismissTasks.clear();
        }
        keyboardSelectionPending = false;
        running.store(true, std::memory_order_release);

        try {
            workerThread = std::thread(&PopperSelectionMonitor::workerMain, this);
            dismissThread = std::thread(&PopperSelectionMonitor::dismissMain, this);
            eventThread = std::thread(&PopperSelectionMonitor::eventMain, this);
        } catch (...) {
            running.store(false, std::memory_order_release);
            taskCondition.notify_all();
            dismissCondition.notify_all();
            if (eventThread.joinable()) eventThread.join();
            if (dismissThread.joinable()) dismissThread.join();
            if (workerThread.joinable()) workerThread.join();
            return POPPER_SELECTION_INTERNAL_ERROR;
        }

        int32_t status = POPPER_SELECTION_INTERNAL_ERROR;
        {
            std::unique_lock<std::mutex> startupLock(startupMutex);
            startupCondition.wait(startupLock, [this] { return startupReady; });
            status = startupStatus;
        }
        if (status != POPPER_SELECTION_OK) {
            running.store(false, std::memory_order_release);
            taskCondition.notify_all();
            dismissCondition.notify_all();
            if (eventThread.joinable()) eventThread.join();
            if (dismissThread.joinable()) dismissThread.join();
            if (workerThread.joinable()) workerThread.join();
        }
        return status;
    }

    int32_t stop() {
        std::lock_guard<std::mutex> lifecycleLock(lifecycleMutex);
        bool wasRunning = running.exchange(false, std::memory_order_acq_rel);
        {
            std::lock_guard<std::mutex> runLoopLock(runLoopMutex);
            if (eventRunLoop != nullptr) {
                CFRunLoopStop(eventRunLoop);
                CFRunLoopWakeUp(eventRunLoop);
            }
        }
        taskCondition.notify_all();
        dismissCondition.notify_all();
        if (eventThread.joinable()) eventThread.join();
        if (dismissThread.joinable()) dismissThread.join();
        if (workerThread.joinable()) workerThread.join();
        {
            std::lock_guard<std::mutex> taskLock(taskMutex);
            tasks.clear();
        }
        {
            std::lock_guard<std::mutex> dismissLock(dismissMutex);
            dismissTasks.clear();
        }
        return wasRunning ? POPPER_SELECTION_OK : POPPER_SELECTION_NOT_RUNNING;
    }

    bool captureCurrent(SelectionInfo &info) {
        std::lock_guard<std::mutex> captureLock(captureMutex);
        CGPoint mouse = CurrentMousePosition();
        return CaptureSelection(
            excludedBundleId,
            Trigger::Manual,
            mouse,
            false,
            mouse,
            false,
            mouse,
            info,
            enhancedUiCache);
    }

    static CGEventRef eventTapCallback(
        CGEventTapProxy,
        CGEventType type,
        CGEventRef event,
        void *context) {
        auto *monitor = static_cast<PopperSelectionMonitor *>(context);
        if (monitor == nullptr || !monitor->running.load(std::memory_order_acquire)) {
            return event;
        }
        if (type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput) {
            if (monitor->eventTap != nullptr) {
                CGEventTapEnable(monitor->eventTap, true);
            }
            return event;
        }
        monitor->handleEvent(type, event);
        return event;
    }

    // Reactive re-enable in eventTapCallback only fires on the next incoming
    // event; if the tap is disabled with no further events arriving (e.g. a
    // sustained high event-rate burst tripped the OS-side rate limit),
    // capture would otherwise stay silent until app restart. This periodic
    // check gives the same self-healing Windows already has for its hook.
    static void HealthCheckTimerCallback(CFRunLoopTimerRef, void *context) {
        auto *monitor = static_cast<PopperSelectionMonitor *>(context);
        if (monitor == nullptr || !monitor->running.load(std::memory_order_acquire)) {
            return;
        }
        if (monitor->eventTap != nullptr && !CGEventTapIsEnabled(monitor->eventTap)) {
            CGEventTapEnable(monitor->eventTap, true);
        }
    }

    void handleEvent(CGEventType type, CGEventRef event) {
        const int64_t targetPid = CGEventGetIntegerValueField(event, kCGEventTargetUnixProcessID);
        // Clicks/scrolls targeting Popper itself must still dismiss an open
        // toolbar (e.g. re-select toolbar over the result webview). Runtime
        // ignores dismiss points that land on the no-activate toolbar window.
        // Never schedule selection capture for the own process.
        if (targetPid == getpid()) {
            if (type == kCGEventLeftMouseDown ||
                type == kCGEventRightMouseDown ||
                type == kCGEventOtherMouseDown) {
                const CGPoint point = CGEventGetLocation(event);
                captureGeneration.fetch_add(1, std::memory_order_acq_rel);
                enqueueDismiss("mouseDown", point, targetPid);
            } else if (type == kCGEventScrollWheel) {
                const CGPoint point = CGEventGetLocation(event);
                captureGeneration.fetch_add(1, std::memory_order_acq_rel);
                enqueueDismiss("scroll", point, targetPid);
            }
            return;
        }
        const CGPoint point = CGEventGetLocation(event);
        const uint64_t now = MonotonicMilliseconds();

        switch (type) {
            case kCGEventLeftMouseDown: {
                captureGeneration.fetch_add(1, std::memory_order_acq_rel);
                mouseDown = point;
                mouseDownTime = now;
                enqueueDismiss("mouseDown", point, targetPid);
                break;
            }
            case kCGEventRightMouseDown:
            case kCGEventOtherMouseDown:
                captureGeneration.fetch_add(1, std::memory_order_acq_rel);
                enqueueDismiss("mouseDown", point, targetPid);
                break;
            case kCGEventScrollWheel:
                captureGeneration.fetch_add(1, std::memory_order_acq_rel);
                enqueueDismiss("scroll", point, targetPid);
                break;
            case kCGEventLeftMouseUp: {
                uint64_t duration = now >= mouseDownTime ? now - mouseDownTime : 0;
                double distance = std::hypot(point.x - mouseDown.x, point.y - mouseDown.y);
                double previousDistance = std::hypot(point.x - lastMouseUp.x, point.y - lastMouseUp.y);
                int64_t clickCount = CGEventGetIntegerValueField(event, kCGMouseEventClickState);
                CGEventFlags flags = CGEventGetFlags(event);

                Trigger trigger = Trigger::Manual;
                bool shouldCapture = false;
                CGPoint start = mouseDown;
                if (duration <= kMaximumDragDurationMs && distance >= kMinimumDragDistance) {
                    trigger = Trigger::Drag;
                    shouldCapture = true;
                } else if (clickCount >= 2 ||
                    (lastClickWasValid && now - lastMouseUpTime <= kDoubleClickDurationMs &&
                     previousDistance <= kDoubleClickDistance)) {
                    trigger = Trigger::DoubleClick;
                    start = point;
                    shouldCapture = true;
                } else {
                    bool shiftOnly = (flags & kCGEventFlagMaskShift) != 0 &&
                        (flags & (kCGEventFlagMaskCommand | kCGEventFlagMaskControl |
                                  kCGEventFlagMaskAlternate)) == 0;
                    if (shiftOnly) {
                        trigger = Trigger::ShiftClick;
                        start = lastMouseUp;
                        shouldCapture = true;
                    }
                }

                lastClickWasValid = duration <= kDoubleClickDurationMs;
                lastMouseUp = point;
                lastMouseUpTime = now;
                if (shouldCapture) {
                    enqueueCapture(trigger, start, true, point, true, point);
                }
                break;
            }
            case kCGEventKeyDown: {
                CGKeyCode keyCode = static_cast<CGKeyCode>(
                    CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode));
                CGEventFlags flags = CGEventGetFlags(event);
                bool extendsWithShift = (flags & kCGEventFlagMaskShift) != 0 &&
                    IsKeyboardSelectionKey(keyCode);
                bool selectAll = (flags & kCGEventFlagMaskCommand) != 0 &&
                    (flags & (kCGEventFlagMaskControl | kCGEventFlagMaskAlternate)) == 0 &&
                    keyCode == kVK_ANSI_A;
                if (extendsWithShift || selectAll) {
                    keyboardSelectionPending = true;
                    keyboardSelectionKey = keyCode;
                }
                if (!IsModifierKey(keyCode)) {
                    captureGeneration.fetch_add(1, std::memory_order_acq_rel);
                    enqueueDismiss("keyDown", point, targetPid);
                }
                break;
            }
            case kCGEventKeyUp: {
                CGKeyCode keyCode = static_cast<CGKeyCode>(
                    CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode));
                if (keyboardSelectionPending && keyCode == keyboardSelectionKey) {
                    keyboardSelectionPending = false;
                    enqueueCapture(Trigger::Keyboard, point, false, point, false, point);
                }
                break;
            }
            default:
                break;
        }
    }

    void enqueueCapture(
        Trigger trigger,
        CGPoint start,
        bool hasStart,
        CGPoint end,
        bool hasEnd,
        CGPoint current) {
        Task task;
        task.kind = TaskKind::Capture;
        task.trigger = trigger;
        task.mouseStart = start;
        task.mouseEnd = end;
        task.mouseCurrent = current;
        task.hasMouseStart = hasStart;
        task.hasMouseEnd = hasEnd;
        task.generation = captureGeneration.fetch_add(1, std::memory_order_acq_rel) + 1;
        task.enqueuedAtMs = MonotonicMilliseconds();
        enqueueTask(std::move(task));
    }

    void enqueueDismiss(const char *reason, CGPoint point, int64_t targetPid) {
        Task task;
        task.kind = TaskKind::Dismiss;
        task.dismissReason = reason;
        task.mouseCurrent = point;
        task.targetPid = targetPid;
        if (!running.load(std::memory_order_acquire)) {
            return;
        }
        {
            std::lock_guard<std::mutex> lock(dismissMutex);
            if (dismissTasks.size() >= kMaximumQueuedTasks) {
                dismissTasks.pop_front();
            }
            if (!dismissTasks.empty() &&
                dismissTasks.back().dismissReason == task.dismissReason) {
                dismissTasks.back() = std::move(task);
            } else {
                dismissTasks.push_back(std::move(task));
            }
        }
        dismissCondition.notify_one();
    }

    void enqueueTask(Task task) {
        if (!running.load(std::memory_order_acquire) || task.kind != TaskKind::Capture) {
            return;
        }
        {
            std::lock_guard<std::mutex> lock(taskMutex);
            // Capture is latest-wins. A task already executing may finish its
            // bounded AX transaction, but queued stale captures must never
            // delay the newest mouse-up or keyboard selection.
            tasks.clear();
            tasks.push_back(std::move(task));
        }
        taskCondition.notify_one();
    }

    void emit(const std::string &json) {
        if (json.empty() || !running.load(std::memory_order_acquire)) {
            return;
        }
        std::lock_guard<std::mutex> lock(callbackMutex);
        if (callback != nullptr && running.load(std::memory_order_acquire)) {
            callback(json.c_str(), callbackContext);
        }
    }

    void workerMain() {
        while (true) {
            Task task;
            {
                std::unique_lock<std::mutex> lock(taskMutex);
                taskCondition.wait(lock, [this] {
                    return !running.load(std::memory_order_acquire) || !tasks.empty();
                });
                if (!running.load(std::memory_order_acquire)) {
                    tasks.clear();
                    return;
                }
                task = std::move(tasks.front());
                tasks.pop_front();
            }

            @autoreleasepool {
                if (task.generation != captureGeneration.load(std::memory_order_acquire)) {
                    continue;
                }

                // Let the target application commit its selection after the
                // input event without ever blocking the event-tap callback.
                std::this_thread::sleep_for(std::chrono::milliseconds(12));
                if (task.generation != captureGeneration.load(std::memory_order_acquire)) {
                    continue;
                }
                const uint64_t captureStartedAt = MonotonicMilliseconds();
                SelectionInfo info;
                bool captured = false;
                {
                    std::lock_guard<std::mutex> captureLock(captureMutex);
                    captured = CaptureSelection(
                        excludedBundleId,
                        task.trigger,
                        task.mouseStart,
                        task.hasMouseStart,
                        task.mouseEnd,
                        task.hasMouseEnd,
                        task.mouseCurrent,
                        info,
                        enhancedUiCache,
                        &captureGeneration,
                        task.generation);
                }
                const uint64_t completedAt = MonotonicMilliseconds();
                TraceSelectionTiming("capture", completedAt - captureStartedAt);
                TraceSelectionTiming(
                    "mouse-up-to-capture-complete",
                    completedAt - task.enqueuedAtMs);
                if (captured && task.generation == captureGeneration.load(std::memory_order_acquire)) {
                    emit(SelectionJSON(info));
                }
            }
        }
    }

    void dismissMain() {
        while (true) {
            Task task;
            {
                std::unique_lock<std::mutex> lock(dismissMutex);
                dismissCondition.wait(lock, [this] {
                    return !running.load(std::memory_order_acquire) || !dismissTasks.empty();
                });
                if (!running.load(std::memory_order_acquire)) {
                    dismissTasks.clear();
                    return;
                }
                task = std::move(dismissTasks.front());
                dismissTasks.pop_front();
            }
            emit(DismissJSON(task));
        }
    }

    void eventMain() {
        @autoreleasepool {
            CFRunLoopRef runLoop = CFRunLoopGetCurrent();
            CGEventMask eventMask =
                CGEventMaskBit(kCGEventLeftMouseDown) |
                CGEventMaskBit(kCGEventLeftMouseUp) |
                CGEventMaskBit(kCGEventRightMouseDown) |
                CGEventMaskBit(kCGEventOtherMouseDown) |
                CGEventMaskBit(kCGEventScrollWheel) |
                CGEventMaskBit(kCGEventKeyDown) |
                CGEventMaskBit(kCGEventKeyUp);

            eventTap = CGEventTapCreate(
                kCGSessionEventTap,
                kCGTailAppendEventTap,
                kCGEventTapOptionListenOnly,
                eventMask,
                &PopperSelectionMonitor::eventTapCallback,
                this);
            if (eventTap != nullptr) {
                runLoopSource = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, eventTap, 0);
            }

            int32_t status = eventTap != nullptr && runLoopSource != nullptr
                ? POPPER_SELECTION_OK
                : POPPER_SELECTION_EVENT_TAP_FAILED;
            if (status == POPPER_SELECTION_OK) {
                {
                    std::lock_guard<std::mutex> lock(runLoopMutex);
                    eventRunLoop = runLoop;
                }
                CFRunLoopAddSource(runLoop, runLoopSource, kCFRunLoopDefaultMode);
                CGEventTapEnable(eventTap, true);

                CFRunLoopTimerContext timerContext = {0, this, nullptr, nullptr, nullptr};
                healthTimer = CFRunLoopTimerCreate(
                    kCFAllocatorDefault,
                    CFAbsoluteTimeGetCurrent() + kEventTapHealthCheckIntervalSeconds,
                    kEventTapHealthCheckIntervalSeconds,
                    0,
                    0,
                    &PopperSelectionMonitor::HealthCheckTimerCallback,
                    &timerContext);
                if (healthTimer != nullptr) {
                    CFRunLoopAddTimer(runLoop, healthTimer, kCFRunLoopDefaultMode);
                }
            }
            {
                std::lock_guard<std::mutex> lock(startupMutex);
                startupStatus = status;
                startupReady = true;
            }
            startupCondition.notify_all();

            if (status == POPPER_SELECTION_OK && running.load(std::memory_order_acquire)) {
                CFRunLoopRun();
            }

            if (eventTap != nullptr) {
                CGEventTapEnable(eventTap, false);
            }
            if (healthTimer != nullptr) {
                CFRunLoopRemoveTimer(runLoop, healthTimer, kCFRunLoopDefaultMode);
                CFRelease(healthTimer);
                healthTimer = nullptr;
            }
            if (runLoopSource != nullptr) {
                CFRunLoopRemoveSource(runLoop, runLoopSource, kCFRunLoopDefaultMode);
                CFRelease(runLoopSource);
                runLoopSource = nullptr;
            }
            if (eventTap != nullptr) {
                CFRelease(eventTap);
                eventTap = nullptr;
            }
            {
                std::lock_guard<std::mutex> lock(runLoopMutex);
                eventRunLoop = nullptr;
            }
        }
    }

    std::string excludedBundleId;
    PopperSelectionEventCallback callback = nullptr;
    void *callbackContext = nullptr;
    std::mutex callbackMutex;
    EnhancedUiCache enhancedUiCache;

    std::atomic<bool> running{false};
    std::atomic<uint64_t> captureGeneration{0};
    std::mutex lifecycleMutex;
    std::mutex captureMutex;

    std::thread eventThread;
    std::thread workerThread;
    std::thread dismissThread;
    CFMachPortRef eventTap = nullptr;
    CFRunLoopSourceRef runLoopSource = nullptr;
    CFRunLoopTimerRef healthTimer = nullptr;
    CFRunLoopRef eventRunLoop = nullptr;
    std::mutex runLoopMutex;

    std::mutex startupMutex;
    std::condition_variable startupCondition;
    bool startupReady = false;
    int32_t startupStatus = POPPER_SELECTION_INTERNAL_ERROR;

    std::mutex taskMutex;
    std::condition_variable taskCondition;
    std::deque<Task> tasks;
    std::mutex dismissMutex;
    std::condition_variable dismissCondition;
    std::deque<Task> dismissTasks;

    CGPoint mouseDown = CGPointZero;
    CGPoint lastMouseUp = CGPointZero;
    uint64_t mouseDownTime = 0;
    uint64_t lastMouseUpTime = 0;
    bool lastClickWasValid = false;
    bool keyboardSelectionPending = false;
    CGKeyCode keyboardSelectionKey = 0;
};

extern "C" uint8_t popper_accessibility_is_trusted(void) {
    return AXIsProcessTrusted() ? 1 : 0;
}

extern "C" uint8_t popper_accessibility_request(void) {
    @autoreleasepool {
        const void *keys[] = { kAXTrustedCheckOptionPrompt };
        const void *values[] = { kCFBooleanTrue };
        CFDictionaryRef options = CFDictionaryCreate(
            kCFAllocatorDefault,
            keys,
            values,
            1,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks);
        bool trusted = AXIsProcessTrustedWithOptions(options);
        if (options != nullptr) CFRelease(options);
        return trusted ? 1 : 0;
    }
}

extern "C" PopperSelectionMonitor *popper_selection_monitor_create(
    const char *excluded_bundle_id_utf8,
    PopperSelectionEventCallback callback,
    void *context) {
    if (excluded_bundle_id_utf8 == nullptr || callback == nullptr) {
        return nullptr;
    }
    try {
        return new PopperSelectionMonitor(excluded_bundle_id_utf8, callback, context);
    } catch (...) {
        return nullptr;
    }
}

extern "C" int32_t popper_selection_monitor_start(PopperSelectionMonitor *monitor) {
    if (monitor == nullptr) {
        return POPPER_SELECTION_INVALID_ARGUMENT;
    }
    try {
        return monitor->start();
    } catch (...) {
        return POPPER_SELECTION_INTERNAL_ERROR;
    }
}

extern "C" int32_t popper_selection_monitor_stop(PopperSelectionMonitor *monitor) {
    if (monitor == nullptr) {
        return POPPER_SELECTION_INVALID_ARGUMENT;
    }
    try {
        return monitor->stop();
    } catch (...) {
        return POPPER_SELECTION_INTERNAL_ERROR;
    }
}

extern "C" char *popper_selection_monitor_capture_current(
    PopperSelectionMonitor *monitor,
    int32_t *status_out) {
    if (status_out != nullptr) *status_out = POPPER_SELECTION_INVALID_ARGUMENT;
    if (monitor == nullptr) {
        return nullptr;
    }
    if (!AXIsProcessTrusted()) {
        if (status_out != nullptr) *status_out = POPPER_SELECTION_NOT_TRUSTED;
        return nullptr;
    }
    try {
        SelectionInfo info;
        if (!monitor->captureCurrent(info)) {
            if (status_out != nullptr) *status_out = POPPER_SELECTION_OK;
            return nullptr;
        }
        std::string json = SelectionJSON(info);
        if (json.empty()) {
            if (status_out != nullptr) *status_out = POPPER_SELECTION_INTERNAL_ERROR;
            return nullptr;
        }
        char *result = static_cast<char *>(std::malloc(json.size() + 1));
        if (result == nullptr) {
            if (status_out != nullptr) *status_out = POPPER_SELECTION_INTERNAL_ERROR;
            return nullptr;
        }
        std::memcpy(result, json.data(), json.size());
        result[json.size()] = '\0';
        if (status_out != nullptr) *status_out = POPPER_SELECTION_OK;
        return result;
    } catch (...) {
        if (status_out != nullptr) *status_out = POPPER_SELECTION_INTERNAL_ERROR;
        return nullptr;
    }
}

extern "C" void popper_selection_string_free(char *value) {
    std::free(value);
}

extern "C" uint8_t popper_selection_clear_matching_text(
    const char *bundle_id_utf8,
    const char *text_utf8) {
    try {
        if (text_utf8 == nullptr || text_utf8[0] == '\0') {
            return 0;
        }
        std::string expectedText(text_utf8);
        std::string requiredBundleId;
        if (bundle_id_utf8 != nullptr && bundle_id_utf8[0] != '\0') {
            requiredBundleId.assign(bundle_id_utf8);
        }

        __block uint8_t result = 0;
        void (^work)(void) = ^{
            result = ClearMatchingTextOnMainThread(requiredBundleId, expectedText);
        };
        if ([NSThread isMainThread]) {
            work();
        } else {
            // AX write must run on the main thread; hop when called from a worker.
            dispatch_sync(dispatch_get_main_queue(), work);
        }
        return result;
    } catch (...) {
        return 0;
    }
}

extern "C" void popper_selection_monitor_destroy(PopperSelectionMonitor *monitor) {
    delete monitor;
}
