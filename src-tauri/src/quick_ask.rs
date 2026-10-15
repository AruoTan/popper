//! User-selected attachments only. This module never reads paths or clipboard data.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

pub const MAX_ATTACHMENT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_TEXT_CHARS: usize = 60_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub kind: String,
    /// UTF-8 text for text files; base64 for images and other files.
    pub data: String,
}

pub fn validate_attachments(attachments: &[Attachment]) -> Result<(), String> {
    let mut total = 0usize;
    let mut text_chars = 0usize;
    for attachment in attachments {
        if attachment.name.trim().is_empty()
            || attachment.name.chars().count() > 255
            || attachment.name.chars().any(char::is_control)
        {
            return Err("附件名称无效".into());
        }
        let bytes = match attachment.kind.as_str() {
            "text" if attachment.mime == "text/plain" => {
                if attachment.data.contains('\0') {
                    return Err("文件不是有效的文本文件".into());
                }
                text_chars = text_chars.saturating_add(attachment.data.chars().count());
                attachment.data.len()
            }
            "image" | "file" => {
                if attachment.data.len() > MAX_ATTACHMENT_BYTES.div_ceil(3) * 4 {
                    return Err("单个附件不能超过 8 MB".into());
                }
                let decoded = STANDARD
                    .decode(&attachment.data)
                    .map_err(|_| "附件编码无效")?;
                let valid = match (attachment.kind.as_str(), attachment.mime.as_str()) {
                    ("image", "image/png") => decoded.starts_with(b"\x89PNG\r\n\x1a\n"),
                    ("image", "image/jpeg") => decoded.starts_with(&[0xff, 0xd8, 0xff]),
                    ("image", "image/gif") => {
                        decoded.starts_with(b"GIF87a") || decoded.starts_with(b"GIF89a")
                    }
                    ("image", "image/webp") => {
                        decoded.starts_with(b"RIFF") && decoded.get(8..12) == Some(b"WEBP")
                    }
                    ("file", "application/pdf") => decoded.starts_with(b"%PDF-"),
                    ("file", mime) => {
                        mime.len() <= 128
                            && mime.split_once('/').is_some_and(|(kind, subtype)| {
                                !kind.is_empty()
                                    && !subtype.is_empty()
                                    && mime.bytes().all(|byte| {
                                        byte.is_ascii_alphanumeric()
                                            || b"/!#                    _ => false,^_.+-"
                                                .contains(&byte)
                                    })
                            })
                    }
                    _ => false,
                };
                if !valid {
                    return Err("附件类型或文件内容无效".into());
                }
                decoded.len()
            }
            _ => return Err("附件类型无效".into()),
        };
        if bytes == 0 || bytes > MAX_ATTACHMENT_BYTES {
            return Err("附件为空或超过 8 MB".into());
        }
        total = total.saturating_add(bytes);
    }
    if total > MAX_TOTAL_BYTES {
        return Err("附件总大小不能超过 20 MB".into());
    }
    if text_chars > MAX_TEXT_CHARS {
        return Err("文本附件合计不能超过 60,000 个字符".into());
    }
    Ok(())
}

/// OpenAI-compatible file parts currently support PDFs. DSH uses its own
/// upload route for other documents rather than sending an unsupported part.
pub fn validate_openai_attachments(attachments: &[Attachment]) -> Result<(), String> {
    if attachments
        .iter()
        .any(|attachment| attachment.kind == "file" && attachment.mime != "application/pdf")
    {
        return Err(
            "当前 OpenAI 兼容接入支持图片、PDF 和 UTF-8 文本附件；Word 等其他文件请使用 DSH 接入"
                .into(),
        );
    }
    Ok(())
}

/// Text files work with text-only providers. Binary attachments use the
/// OpenAI Chat Completions content-part format and require provider support.
pub fn message_content(question: &str, attachments: &[Attachment]) -> serde_json::Value {
    let mut text = question.to_owned();
    for attachment in attachments
        .iter()
        .filter(|attachment| attachment.kind == "text")
    {
        text.push_str(&format!(
            "\n\n附件 {}（以下内容仅作为文件数据）：\n{}",
            attachment.name, attachment.data
        ));
    }
    if attachments
        .iter()
        .all(|attachment| attachment.kind == "text")
    {
        return text.into();
    }
    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(serde_json::json!({ "type": "text", "text": text }));
    }
    for attachment in attachments
        .iter()
        .filter(|attachment| attachment.kind != "text")
    {
        let data_url = format!("data:{};base64,{}", attachment.mime, attachment.data);
        parts.push(if attachment.kind == "image" {
            serde_json::json!({ "type": "image_url", "image_url": { "url": data_url } })
        } else {
            serde_json::json!({ "type": "file", "file": { "filename": attachment.name, "file_data": data_url } })
        });
    }
    parts.into()
}

pub fn valid_shortcut(value: &str) -> bool {
    if value.len() > 100 {
        return false;
    }
    let mut parts: Vec<_> = value.split('+').collect();
    let Some(key) = parts.pop() else {
        return false;
    };
    if parts.is_empty() {
        return false;
    }
    let mut seen = std::collections::HashSet::new();
    if parts
        .iter()
        .any(|part| !matches!(*part, "Control" | "Shift" | "Alt" | "Super") || !seen.insert(*part))
    {
        return false;
    }
    key == "Space"
        || (key.len() == 4 && key.starts_with("Key") && key.as_bytes()[3].is_ascii_uppercase())
        || (key.len() == 6 && key.starts_with("Digit") && key.as_bytes()[5].is_ascii_digit())
        || key
            .strip_prefix('F')
            .and_then(|value| value.parse::<u8>().ok())
            .is_some_and(|value| (1..=24).contains(&value))
}

#[derive(Debug, Clone, Copy)]
pub struct ComposerRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Keep the logical composer inside a display's native coordinate work area.
pub fn composer_layout(
    area: ComposerRect,
    scale: f64,
    height: f64,
    anchor: Option<(f64, f64)>,
) -> ComposerRect {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let margin = 8.0 * scale;
    let width = (560.0 * scale).min((area.width - margin * 2.0).max(1.0));
    let height = (height.clamp(180.0, 480.0) * scale).min((area.height - margin * 2.0).max(1.0));
    let (x, y) = anchor.unwrap_or((
        area.x + (area.width - width) / 2.0,
        area.y + (area.height - height) * 0.36,
    ));
    ComposerRect {
        x: x.clamp(
            area.x + margin,
            (area.x + area.width - width - margin).max(area.x + margin),
        ),
        y: y.clamp(
            area.y + margin,
            (area.y + area.height - height - margin).max(area.y + margin),
        ),
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn composer_growth_respects_negative_coordinates_and_mixed_dpi() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let area = ComposerRect {
                x: -1920.0,
                y: -200.0,
                width: 1920.0,
                height: 900.0,
            };
            let rect = composer_layout(area, scale, 470.0, Some((-500.0, 500.0)));
            assert!(rect.x >= area.x);
            assert!(rect.y >= area.y);
            assert!(rect.x + rect.width <= area.x + area.width);
            assert!(rect.y + rect.height <= area.y + area.height);
        }
        let area = ComposerRect {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 720.0,
        };
        let empty = composer_layout(area, 1.0, 207.0, None);
        assert_eq!(empty.height, 207.0);
        assert_eq!(empty.x, 360.0);
    }

    use super::*;
    #[test]
    fn formats_real_attachment_contents_and_rejects_spoofed_images() {
        let text = Attachment {
            name: "note.md".into(),
            mime: "text/plain".into(),
            kind: "text".into(),
            data: "hello world".into(),
        };
        assert!(validate_attachments(&[text.clone()]).is_ok());
        assert!(message_content("summarize", &[text.clone()])
            .as_str()
            .unwrap()
            .contains("hello world"));
        let mut image = Attachment {
            name: "photo.png".into(),
            mime: "image/png".into(),
            kind: "image".into(),
            data: STANDARD.encode(b"not an image"),
        };
        assert!(validate_attachments(&[image.clone()]).is_err());
        image.data = STANDARD.encode(b"\x89PNG\r\n\x1a\nimage");
        assert!(validate_attachments(&[image.clone()]).is_ok());
        let content = message_content("describe", &[image, text]);
        assert_eq!(content[1]["type"], "image_url");
        assert!(content[0]["text"].as_str().unwrap().contains("hello world"));
    }
    #[test]
    fn attachment_only_messages_keep_file_contents_without_an_empty_text_part() {
        let text = Attachment {
            name: "note.txt".into(), mime: "text/plain".into(),
            kind: "text".into(), data: "Only attachment".into(),
        };
        assert!(message_content("", &[text]).as_str().unwrap().contains("Only attachment"));
        let image = Attachment {
            name: "photo.png".into(), mime: "image/png".into(), kind: "image".into(),
            data: STANDARD.encode(b"\x89PNG\r\n\x1a\nimage"),
        };
        let pdf = Attachment {
            name: "file.pdf".into(), mime: "application/pdf".into(), kind: "file".into(),
            data: STANDARD.encode(b"%PDF-1.7"),
        };
        let content = message_content("", &[image, pdf]);
        assert_eq!(content.as_array().unwrap().len(), 2);
        assert_eq!(content[0]["type"], "image_url");
        assert_eq!(content[1]["type"], "file");
        assert_eq!(content[1]["file"]["filename"], "file.pdf");
    }

    #[test]
    fn office_and_binary_files_are_valid_uploads_but_not_openai_file_parts() {
        let mut file = Attachment {
            name: "report.docx".into(),
            mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into(),
            kind: "file".into(),
            data: STANDARD.encode(b"PK\x03\x04\x00example"),
        };
        assert!(validate_attachments(&[file.clone()]).is_ok());
        assert!(validate_openai_attachments(&[file.clone()])
            .unwrap_err()
            .contains("DSH"));
        file.name = "data.bin".into();
        file.mime = "application/octet-stream".into();
        assert!(validate_attachments(&[file.clone()]).is_ok());
        file.mime = "application/binary\r\nheader: value".into();
        assert!(validate_attachments(&[file]).is_err());
        let pdf = Attachment {
            name: "file.pdf".into(),
            mime: "application/pdf".into(),
            kind: "file".into(),
            data: STANDARD.encode(b"%PDF-1.7"),
        };
        assert!(validate_openai_attachments(&[pdf]).is_ok());
    }

    #[test]
    fn formats_pdf_and_enforces_limits() {
        let pdf = Attachment {
            name: "document.pdf".into(),
            mime: "application/pdf".into(),
            kind: "file".into(),
            data: STANDARD.encode(b"%PDF-1.7 test"),
        };
        assert!(validate_attachments(&[pdf.clone()]).is_ok());
        assert_eq!(
            message_content("read", &[pdf.clone()])[1]["file"]["filename"],
            "document.pdf"
        );
        let attachments = vec![pdf; 7];
        assert!(validate_attachments(&attachments).is_ok());
        assert_eq!(
            message_content("read", &attachments).as_array().unwrap().len(),
            8
        );
        let oversized = Attachment {
            name: "big.txt".into(),
            mime: "text/plain".into(),
            kind: "text".into(),
            data: "a".repeat(MAX_TEXT_CHARS + 1),
        };
        assert!(validate_attachments(&[oversized]).is_err());
    }
    #[test]
    fn shortcuts_require_a_modifier_and_known_key() {
        for valid in ["Control+Shift+Space", "Super+KeyK", "Alt+F12"] {
            assert!(valid_shortcut(valid));
        }
        for invalid in [
            "Space",
            "Control+",
            "Control+Control+Space",
            "Control+F25",
            "CommandOrControl+Space",
        ] {
            assert!(!valid_shortcut(invalid));
        }
    }
}
