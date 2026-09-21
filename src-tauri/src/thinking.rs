//! Thinking-level heuristics and OpenAI-compatible request body injection.

use serde_json::{json, Value};

use crate::models::{
    clamp_thinking_mode, ThinkingCapability, ThinkingCapabilitySource, ThinkingDialect,
    ThinkingLevel, ThinkingMode,
};
use crate::openai_protocol::ProviderErrorEnvelope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedThinkingCapability {
    pub levels: Vec<ThinkingLevel>,
    pub capability: ThinkingCapability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThinkingRequestPlan {
    pub capability: Option<ThinkingCapability>,
    pub levels: Vec<ThinkingLevel>,
    pub mode: ThinkingMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThinkingBodyControl {
    ReasoningEffort(String),
    EnableThinking(bool),
    ChatTemplateKwargs(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedThinkingControl {
    pub field_path: &'static str,
    pub source: ThinkingCapabilitySource,
}

/// Authorize the one narrowly-scoped no-field retry.
///
/// HTTP status is diagnostic only: providers may return a structured rejection
/// in a successful envelope or with a 400/422 response.  The caller must still
/// pass the original attempt number and content/cancellation state.
pub fn should_fallback_without_control(
    applied: &AppliedThinkingControl,
    mode: ThinkingMode,
    attempt: u8,
    _status: Option<u16>,
    provider_error: Option<&ProviderErrorEnvelope>,
    content_seen: bool,
    cancelled: bool,
) -> bool {
    applied.source == ThinkingCapabilitySource::Heuristic
        && mode == ThinkingMode::Off
        && attempt == 1
        && !content_seen
        && !cancelled
        && provider_error.is_some_and(|error| {
            matches!(
                error
                    .code
                    .as_deref()
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("unsupported_parameter" | "unknown_field")
            ) && error_names_field(error, applied.field_path)
        })
}

fn error_names_field(error: &ProviderErrorEnvelope, field_path: &str) -> bool {
    if error
        .param
        .as_deref()
        .is_some_and(|param| normalize_field_path(param) == field_path)
    {
        return true;
    }
    message_contains_field(&error.message, field_path)
}

fn normalize_field_path(value: &str) -> &str {
    value
        .trim()
        .strip_prefix("$.")
        .unwrap_or_else(|| value.trim())
}

fn message_contains_field(message: &str, field_path: &str) -> bool {
    let bytes = message.as_bytes();
    let needle = field_path.as_bytes();
    if needle.is_empty() || needle.len() > bytes.len() {
        return false;
    }
    bytes
        .windows(needle.len())
        .enumerate()
        .any(|(index, window)| {
            if window != needle {
                return false;
            }
            let before_index = index.checked_sub(1);
            let before = before_index.and_then(|i| bytes.get(i)).copied();
            let after = bytes.get(index + needle.len()).copied();
            let json_path_prefix = before == Some(b'.')
                && before_index
                    .and_then(|i| i.checked_sub(1))
                    .and_then(|i| bytes.get(i))
                    == Some(&b'$');
            (!before.is_some_and(is_field_identifier_char) || json_path_prefix)
                && !after.is_some_and(is_field_identifier_char)
        })
}

fn is_field_identifier_char(value: u8) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, b'_' | b'.')
}

/// Parse a free-form effort string from provider API metadata.
pub fn parse_level_str(raw: &str) -> Option<ThinkingLevel> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "minimal" => Some(ThinkingLevel::Minimal),
        "low" => Some(ThinkingLevel::Low),
        "medium" => Some(ThinkingLevel::Medium),
        "high" => Some(ThinkingLevel::High),
        "xhigh" | "x-high" | "extra_high" | "extra-high" => Some(ThinkingLevel::Xhigh),
        _ => None,
    }
}

/// Extract explicit thinking evidence from an OpenAI-compatible `/models` item.
/// The presence of relevant metadata is authoritative even when it is empty or ambiguous.
pub fn extract_thinking_capability_from_api_item(
    item: &Value,
) -> Option<DetectedThinkingCapability> {
    let supported_parameters = item.get("supported_parameters");
    let nested_options = item.pointer("/reasoning/effort_options");
    let root_options = item.get("reasoning_effort_options");
    let has_support_flag = has_explicit_support_flag(item);
    if supported_parameters.is_none()
        && nested_options.is_none()
        && root_options.is_none()
        && !has_support_flag
    {
        return None;
    }

    let parameters = supported_parameters
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();

    let mut dialects = Vec::new();
    let mut add_dialect = |dialect| {
        if !dialects.contains(&dialect) {
            dialects.push(dialect);
        }
    };
    let options = nested_options
        .and_then(Value::as_array)
        .or_else(|| root_options.and_then(Value::as_array));
    if parameters.iter().any(|value| value == "reasoning_effort") || options.is_some() {
        add_dialect(ThinkingDialect::ReasoningEffort);
    }
    if parameters.iter().any(|value| value == "enable_thinking") {
        add_dialect(ThinkingDialect::EnableThinking);
    }
    if parameters.iter().any(|value| {
        matches!(
            value.as_str(),
            "chat_template_kwargs" | "chat_template_kwargs.enable_thinking"
        )
    }) {
        add_dialect(ThinkingDialect::ChatTemplateKwargs);
    }

    let mut levels = Vec::new();
    let mut supports_off = false;
    if let Some(options) = options {
        for raw in options.iter().filter_map(Value::as_str) {
            let normalized = raw.trim().to_ascii_lowercase();
            if normalized == "none" {
                supports_off = true;
            } else if let Some(level) = parse_level_str(&normalized) {
                if !levels.contains(&level) {
                    levels.push(level);
                }
            }
        }
    } else if dialects == [ThinkingDialect::ReasoningEffort] {
        levels = standard_levels();
    }

    let explicitly_unsupported = explicit_support_flag(item) == Some(false);
    if explicitly_unsupported {
        levels.clear();
        supports_off = false;
    }
    let dialect = (!explicitly_unsupported && dialects.len() == 1).then(|| dialects[0]);
    if matches!(
        dialect,
        Some(ThinkingDialect::EnableThinking | ThinkingDialect::ChatTemplateKwargs)
    ) {
        supports_off = true;
        if levels.is_empty() {
            levels = standard_levels();
        }
    }

    Some(DetectedThinkingCapability {
        levels,
        capability: ThinkingCapability {
            source: ThinkingCapabilitySource::Explicit,
            dialect,
            supports_off,
        },
    })
}

/// Infer capability from model id only when provider metadata is silent.
pub fn infer_thinking_capability(model_id: &str) -> Option<DetectedThinkingCapability> {
    let id = model_id.trim().to_ascii_lowercase();
    if id.is_empty() {
        return None;
    }
    let openai_full = vec![
        ThinkingLevel::Minimal,
        ThinkingLevel::Low,
        ThinkingLevel::Medium,
        ThinkingLevel::High,
        ThinkingLevel::Xhigh,
    ];

    let (levels, dialect, supports_off) = if o_series_match(&id) {
        (
            standard_levels(),
            Some(ThinkingDialect::ReasoningEffort),
            true,
        )
    } else if id.contains("gpt-5") || id.contains("gpt5") {
        (openai_full, Some(ThinkingDialect::ReasoningEffort), true)
    } else if id.contains("deepseek-r1") || id.contains("deepseek-reasoner") {
        (
            standard_levels(),
            Some(ThinkingDialect::ReasoningEffort),
            true,
        )
    } else if id.contains("qwen3")
        && (id.contains("think") || id.contains("reasoning") || !id.contains("instruct"))
    {
        (
            standard_levels(),
            Some(ThinkingDialect::EnableThinking),
            true,
        )
    } else if id.contains("thinking") || id.contains("reasoner") || id.contains("reasoning") {
        (standard_levels(), None, false)
    } else {
        return None;
    };

    Some(DetectedThinkingCapability {
        levels,
        capability: ThinkingCapability {
            source: ThinkingCapabilitySource::Heuristic,
            dialect,
            supports_off,
        },
    })
}

fn standard_levels() -> Vec<ThinkingLevel> {
    vec![
        ThinkingLevel::Low,
        ThinkingLevel::Medium,
        ThinkingLevel::High,
    ]
}

fn explicit_support_flag(item: &Value) -> Option<bool> {
    thinking_support_flag_keys()
        .into_iter()
        .find_map(|key| item.get(key).and_then(Value::as_bool))
}

fn has_explicit_support_flag(item: &Value) -> bool {
    thinking_support_flag_keys()
        .into_iter()
        .any(|key| item.get(key).is_some())
}

fn thinking_support_flag_keys() -> [&'static str; 6] {
    [
        "supports_thinking",
        "supports_reasoning",
        "thinking_supported",
        "reasoning_supported",
        "supportsThinking",
        "supportsReasoning",
    ]
}

fn o_series_match(id: &str) -> bool {
    // Mirrors TS: /(^|[/:_-])(o1|o3|o4)([/:_-]|$)/ or includes o1-/o3-/o4-
    if id.contains("o1-") || id.contains("o3-") || id.contains("o4-") {
        return true;
    }
    let bytes = id.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let is_o = bytes[i] == b'o';
        let digit = bytes[i + 1];
        if is_o && matches!(digit, b'1' | b'3' | b'4') {
            let left_ok = i == 0 || matches!(bytes[i - 1], b'/' | b':' | b'_' | b'-');
            let right_ok =
                i + 2 >= bytes.len() || matches!(bytes[i + 2], b'/' | b':' | b'_' | b'-');
            if left_ok && right_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn level_api_str(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::Xhigh => "xhigh",
    }
}

pub fn resolve_body_control(plan: &ThinkingRequestPlan) -> Option<ThinkingBodyControl> {
    let capability = plan.capability.as_ref()?;
    let dialect = capability.dialect?;
    let mode = clamp_thinking_mode(plan.mode, &plan.levels);
    if mode == ThinkingMode::Off {
        if !capability.supports_off {
            return None;
        }
        return Some(match dialect {
            ThinkingDialect::ReasoningEffort => {
                ThinkingBodyControl::ReasoningEffort("none".to_owned())
            }
            ThinkingDialect::EnableThinking => ThinkingBodyControl::EnableThinking(false),
            ThinkingDialect::ChatTemplateKwargs => ThinkingBodyControl::ChatTemplateKwargs(false),
        });
    }

    Some(match dialect {
        ThinkingDialect::ReasoningEffort => {
            ThinkingBodyControl::ReasoningEffort(level_api_str(mode.as_level()?).to_owned())
        }
        ThinkingDialect::EnableThinking => ThinkingBodyControl::EnableThinking(true),
        ThinkingDialect::ChatTemplateKwargs => ThinkingBodyControl::ChatTemplateKwargs(true),
    })
}

pub fn apply_thinking_to_body(
    body: &mut Value,
    plan: &ThinkingRequestPlan,
) -> Option<AppliedThinkingControl> {
    let source = plan.capability.as_ref()?.source;
    let control = resolve_body_control(plan)?;
    let object = body.as_object_mut()?;
    let field_path = match control {
        ThinkingBodyControl::ReasoningEffort(value) => {
            object.insert("reasoning_effort".to_owned(), json!(value));
            "reasoning_effort"
        }
        ThinkingBodyControl::EnableThinking(value) => {
            object.insert("enable_thinking".to_owned(), json!(value));
            "enable_thinking"
        }
        ThinkingBodyControl::ChatTemplateKwargs(value) => {
            object.insert(
                "chat_template_kwargs".to_owned(),
                json!({"enable_thinking": value}),
            );
            "chat_template_kwargs.enable_thinking"
        }
    };
    Some(AppliedThinkingControl { field_path, source })
}

pub fn resolve_thinking_for_request(
    model_id: &str,
    model_levels: &[ThinkingLevel],
    capability: Option<&ThinkingCapability>,
    mode: ThinkingMode,
) -> ThinkingRequestPlan {
    let inferred = if capability.is_none() {
        infer_thinking_capability(model_id)
    } else {
        None
    };
    let levels = if model_levels.is_empty() {
        inferred
            .as_ref()
            .map(|value| value.levels.clone())
            .unwrap_or_default()
    } else {
        model_levels.to_vec()
    };
    ThinkingRequestPlan {
        capability: capability
            .cloned()
            .or_else(|| inferred.map(|value| value.capability)),
        mode: clamp_thinking_mode(mode, &levels),
        levels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authorized(
        applied: &AppliedThinkingControl,
        mode: ThinkingMode,
        attempt: u8,
        error: Option<&ProviderErrorEnvelope>,
        content_seen: bool,
        cancelled: bool,
    ) -> bool {
        should_fallback_without_control(
            applied,
            mode,
            attempt,
            Some(422),
            error,
            content_seen,
            cancelled,
        )
    }

    #[test]
    fn fallback_allows_only_heuristic_off_exact_field_rejection_before_content() {
        let applied = AppliedThinkingControl {
            field_path: "reasoning_effort",
            source: ThinkingCapabilitySource::Heuristic,
        };
        let error = ProviderErrorEnvelope {
            code: Some("unsupported_parameter".to_owned()),
            kind: Some("invalid_request_error".to_owned()),
            param: Some("reasoning_effort".to_owned()),
            message: "Unsupported parameter: reasoning_effort".to_owned(),
        };
        assert!(should_fallback_without_control(
            &applied,
            ThinkingMode::Off,
            1,
            Some(400),
            Some(&error),
            false,
            false,
        ));
    }

    #[test]
    fn fallback_rejects_every_disallowed_context() {
        let heuristic = AppliedThinkingControl {
            field_path: "enable_thinking",
            source: ThinkingCapabilitySource::Heuristic,
        };
        let explicit = AppliedThinkingControl {
            field_path: "enable_thinking",
            source: ThinkingCapabilitySource::Explicit,
        };
        let matching = ProviderErrorEnvelope {
            code: Some("unknown_field".to_owned()),
            kind: None,
            param: Some("enable_thinking".to_owned()),
            message: "unknown field enable_thinking".to_owned(),
        };
        let unrelated = ProviderErrorEnvelope {
            code: Some("unknown_field".to_owned()),
            kind: None,
            param: Some("temperature".to_owned()),
            message: "unknown field temperature".to_owned(),
        };
        let generic = ProviderErrorEnvelope {
            code: Some("bad_request".to_owned()),
            kind: None,
            param: Some("enable_thinking".to_owned()),
            message: "bad request".to_owned(),
        };

        assert!(!authorized(
            &explicit,
            ThinkingMode::Off,
            1,
            Some(&matching),
            false,
            false
        ));
        assert!(!authorized(
            &heuristic,
            ThinkingMode::High,
            1,
            Some(&matching),
            false,
            false
        ));
        assert!(!authorized(
            &heuristic,
            ThinkingMode::Off,
            2,
            Some(&matching),
            false,
            false
        ));
        assert!(!authorized(
            &heuristic,
            ThinkingMode::Off,
            1,
            Some(&matching),
            true,
            false
        ));
        assert!(!authorized(
            &heuristic,
            ThinkingMode::Off,
            1,
            Some(&matching),
            false,
            true
        ));
        assert!(!authorized(
            &heuristic,
            ThinkingMode::Off,
            1,
            Some(&unrelated),
            false,
            false
        ));
        assert!(!authorized(
            &heuristic,
            ThinkingMode::Off,
            1,
            Some(&generic),
            false,
            false
        ));
        assert!(!authorized(
            &heuristic,
            ThinkingMode::Off,
            1,
            None,
            false,
            false
        ));
    }

    #[test]
    fn fallback_message_match_requires_field_boundaries() {
        let applied = AppliedThinkingControl {
            field_path: "reasoning_effort",
            source: ThinkingCapabilitySource::Heuristic,
        };
        for message in [
            "unknown field reasoning_effort",
            "unknown field `reasoning_effort`",
            "unknown field $.reasoning_effort",
        ] {
            let error = ProviderErrorEnvelope {
                code: Some("unknown_field".to_owned()),
                kind: None,
                param: None,
                message: message.to_owned(),
            };
            assert!(authorized(
                &applied,
                ThinkingMode::Off,
                1,
                Some(&error),
                false,
                false
            ));
        }
        let false_positive = ProviderErrorEnvelope {
            code: Some("unknown_field".to_owned()),
            kind: None,
            param: None,
            message: "unknown field not_reasoning_effort_backup".to_owned(),
        };
        assert!(!authorized(
            &applied,
            ThinkingMode::Off,
            1,
            Some(&false_positive),
            false,
            false,
        ));
    }

    #[test]
    fn thinking_openai_dialect_never_contains_qwen_fields() {
        let plan = ThinkingRequestPlan {
            capability: Some(ThinkingCapability {
                source: ThinkingCapabilitySource::Explicit,
                dialect: Some(ThinkingDialect::ReasoningEffort),
                supports_off: true,
            }),
            levels: vec![
                ThinkingLevel::Low,
                ThinkingLevel::Medium,
                ThinkingLevel::High,
            ],
            mode: ThinkingMode::Off,
        };
        let mut body = serde_json::json!({"model":"o3-mini","stream":true});
        let applied = apply_thinking_to_body(&mut body, &plan).unwrap();
        assert_eq!(body["reasoning_effort"], "none");
        assert!(body.get("enable_thinking").is_none());
        assert!(body.get("chat_template_kwargs").is_none());
        assert_eq!(applied.field_path, "reasoning_effort");
    }

    #[test]
    fn thinking_qwen_dialects_never_contain_reasoning_effort() {
        for dialect in [
            ThinkingDialect::EnableThinking,
            ThinkingDialect::ChatTemplateKwargs,
        ] {
            let plan = ThinkingRequestPlan {
                capability: Some(ThinkingCapability {
                    source: ThinkingCapabilitySource::Explicit,
                    dialect: Some(dialect),
                    supports_off: true,
                }),
                levels: vec![
                    ThinkingLevel::Low,
                    ThinkingLevel::Medium,
                    ThinkingLevel::High,
                ],
                mode: ThinkingMode::Off,
            };
            let mut body = serde_json::json!({"model":"qwen3","stream":true});
            apply_thinking_to_body(&mut body, &plan).unwrap();
            assert!(body.get("reasoning_effort").is_none());
            match dialect {
                ThinkingDialect::EnableThinking => assert_eq!(body["enable_thinking"], false),
                ThinkingDialect::ChatTemplateKwargs => {
                    assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false)
                }
                ThinkingDialect::ReasoningEffort => unreachable!(),
            }
        }
    }

    #[test]
    fn thinking_off_omits_none_when_explicit_levels_do_not_support_off() {
        let plan = ThinkingRequestPlan {
            capability: Some(ThinkingCapability {
                source: ThinkingCapabilitySource::Explicit,
                dialect: Some(ThinkingDialect::ReasoningEffort),
                supports_off: false,
            }),
            levels: vec![ThinkingLevel::Low, ThinkingLevel::Medium],
            mode: ThinkingMode::Off,
        };
        let mut body = serde_json::json!({"model":"vendor/model","stream":true});
        assert!(apply_thinking_to_body(&mut body, &plan).is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn thinking_explicit_empty_metadata_blocks_name_inference() {
        let item = serde_json::json!({"id":"o3-mini","supported_parameters":[]});
        let detected = extract_thinking_capability_from_api_item(&item).unwrap();
        assert!(detected.levels.is_empty());
        assert_eq!(
            detected.capability.source,
            ThinkingCapabilitySource::Explicit
        );
        assert_eq!(detected.capability.dialect, None);

        let unsupported = extract_thinking_capability_from_api_item(&serde_json::json!({
            "id": "o3-mini",
            "supports_thinking": false
        }))
        .unwrap();
        assert!(unsupported.levels.is_empty());
        assert_eq!(unsupported.capability.dialect, None);

        let malformed_options = extract_thinking_capability_from_api_item(&serde_json::json!({
            "id": "o3-mini",
            "reasoning_effort_options": "low"
        }))
        .unwrap();
        assert!(malformed_options.levels.is_empty());
        assert_eq!(malformed_options.capability.dialect, None);
    }

    #[test]
    fn thinking_silent_metadata_uses_family_specific_heuristics() {
        assert_eq!(
            infer_thinking_capability("o3-mini")
                .unwrap()
                .capability
                .dialect,
            Some(ThinkingDialect::ReasoningEffort)
        );
        assert_eq!(
            infer_thinking_capability("qwen3-235b-a22b")
                .unwrap()
                .capability
                .dialect,
            Some(ThinkingDialect::EnableThinking)
        );
        assert!(infer_thinking_capability("gpt-4o-mini").is_none());
    }

    #[test]
    fn thinking_explicit_metadata_selects_one_safe_dialect() {
        let effort = extract_thinking_capability_from_api_item(&serde_json::json!({
            "supported_parameters": ["reasoning_effort"],
            "reasoning": {"effort_options": ["none", "minimal", "high", "x-high"]}
        }))
        .unwrap();
        assert_eq!(
            effort.levels,
            vec![
                ThinkingLevel::Minimal,
                ThinkingLevel::High,
                ThinkingLevel::Xhigh
            ]
        );
        assert_eq!(
            effort.capability.dialect,
            Some(ThinkingDialect::ReasoningEffort)
        );
        assert!(effort.capability.supports_off);

        let boolean = extract_thinking_capability_from_api_item(&serde_json::json!({
            "supported_parameters": ["enable_thinking"]
        }))
        .unwrap();
        assert_eq!(
            boolean.capability.dialect,
            Some(ThinkingDialect::EnableThinking)
        );

        let nested = extract_thinking_capability_from_api_item(&serde_json::json!({
            "supported_parameters": ["chat_template_kwargs.enable_thinking"]
        }))
        .unwrap();
        assert_eq!(
            nested.capability.dialect,
            Some(ThinkingDialect::ChatTemplateKwargs)
        );
    }

    #[test]
    fn thinking_ambiguous_explicit_metadata_never_guesses_a_dialect() {
        for parameters in [
            serde_json::json!(["reasoning"]),
            serde_json::json!(["include_reasoning"]),
            serde_json::json!(["reasoning_effort", "enable_thinking"]),
        ] {
            let detected = extract_thinking_capability_from_api_item(&serde_json::json!({
                "supported_parameters": parameters
            }))
            .unwrap();
            assert_eq!(
                detected.capability.source,
                ThinkingCapabilitySource::Explicit
            );
            assert_eq!(detected.capability.dialect, None);
        }
    }

    #[test]
    fn thinking_resolution_prefers_persisted_evidence_and_legacy_levels() {
        let persisted = ThinkingCapability {
            source: ThinkingCapabilitySource::Explicit,
            dialect: None,
            supports_off: false,
        };
        let plan = resolve_thinking_for_request(
            "o3-mini",
            &[ThinkingLevel::Low],
            Some(&persisted),
            ThinkingMode::High,
        );
        assert_eq!(plan.capability, Some(persisted));
        assert_eq!(plan.levels, vec![ThinkingLevel::Low]);
        assert_eq!(plan.mode, ThinkingMode::Off);

        let legacy =
            resolve_thinking_for_request("gpt-5", &[ThinkingLevel::High], None, ThinkingMode::High);
        assert_eq!(
            legacy.capability.unwrap().source,
            ThinkingCapabilitySource::Heuristic
        );
        assert_eq!(legacy.levels, vec![ThinkingLevel::High]);
        assert_eq!(legacy.mode, ThinkingMode::High);
    }

    #[test]
    fn thinking_non_off_boolean_dialect_sends_true_only() {
        let plan = ThinkingRequestPlan {
            capability: Some(ThinkingCapability {
                source: ThinkingCapabilitySource::Heuristic,
                dialect: Some(ThinkingDialect::EnableThinking),
                supports_off: true,
            }),
            levels: vec![
                ThinkingLevel::Low,
                ThinkingLevel::Medium,
                ThinkingLevel::High,
            ],
            mode: ThinkingMode::Medium,
        };
        let mut body = serde_json::json!({"model":"qwen3"});
        let applied = apply_thinking_to_body(&mut body, &plan).unwrap();
        assert_eq!(body["enable_thinking"], true);
        assert!(body.get("reasoning_effort").is_none());
        assert_eq!(applied.field_path, "enable_thinking");
    }
}
