//! DSH intake uses staged file receipts and native image parts, never OpenAI parts.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

use super::{connection::Rpc, failure, TransportFailure};
use crate::quick_ask::Attachment;

pub(super) async fn prompt_content<N: FnMut(String)>(
    rpc: &Rpc,
    session_id: &str,
    prompt: String,
    attachments: &[Attachment],
    notice: &mut N,
) -> Result<Vec<Value>, TransportFailure> {
    let mut content = vec![json!({"type":"text", "text":prompt})];
    for (index, attachment) in attachments.iter().enumerate() {
        if attachment.kind == "image" {
            content.push(json!({
                "type":"image", "mediaType":attachment.mime,
                "data":attachment.data, "name":attachment.name,
            }));
        } else {
            notice(format!(
                "DSH 正在上传附件（{} / {}）：{}",
                index + 1,
                attachments.len(),
                attachment.name
            ));
            let bytes = if attachment.kind == "text" {
                attachment.data.as_bytes().to_vec()
            } else {
                STANDARD
                    .decode(&attachment.data)
                    .map_err(|_| failure("DSH_ATTACHMENT_INVALID", "附件编码无效"))?
            };
            let receipt = rpc.upload_file(session_id, &attachment.name, bytes).await?;
            content.push(json!({"type":"file", "receiptId":receipt}));
        }
    }
    Ok(content)
}
