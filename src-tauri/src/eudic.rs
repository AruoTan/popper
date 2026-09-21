use crate::dictionary::json_response;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyBook {
    pub id: String,
    pub name: String,
    pub language: String,
}
pub async fn books(client: &Client, authorization: &str) -> Result<Vec<StudyBook>, String> {
    let value = json_response(
        client
            .get(crate::dictionary::query_url(
                "https://api.frdic.com/api/open/v1/studylist/category",
                &[("language", "en")],
            ))
            .header("Authorization", authorization),
    )
    .await?;
    serde_json::from_value(value.get("data").cloned().ok_or("生词本响应格式异常")?)
        .map_err(|_| "生词本响应格式异常".to_owned())
}
pub async fn add(
    client: &Client,
    authorization: &str,
    category: &str,
    word: &str,
) -> Result<(), String> {
    let value = json_response(add_request(client, authorization, category, word)?).await?;
    if value.get("message").and_then(|v| v.as_str()).is_none() {
        return Err("添加响应格式异常，请在欧路确认后重试".to_owned());
    }
    Ok(())
}

fn add_request(
    client: &Client,
    authorization: &str,
    category: &str,
    word: &str,
) -> Result<reqwest::RequestBuilder, String> {
    if category.is_empty() || category.len() > 128 {
        return Err("请选择有效的生词本".to_owned());
    }
    Ok(client
        .post("https://api.frdic.com/api/open/v1/studylist/words")
        .header("Authorization", authorization)
        .json(&json!({"language":"en", "category_id":category, "words":[word]})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addition_preserves_string_id_and_phrase_without_uploading_context_or_credentials_in_body() {
        let request = add_request(
            &Client::new(),
            "NIS secret",
            "132303016416635230",
            "take off",
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(request.headers()["Authorization"], "NIS secret");
        assert!(!request.url().as_str().contains("secret"));
        let body: serde_json::Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"language":"en", "category_id":"132303016416635230", "words":["take off"]})
        );
        assert!(add_request(&Client::new(), "NIS secret", "", "take off").is_err());
    }
}
