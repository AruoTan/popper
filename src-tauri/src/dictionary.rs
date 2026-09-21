//! Bounded, read-only Youdao transport and normalized dictionary data.
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub word: String,
    pub explanation: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WordForm {
    pub name: String,
    pub value: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Example {
    pub text: String,
    pub translation: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryEntry {
    pub word: String,
    pub uk_phone: Option<String>,
    pub us_phone: Option<String>,
    pub definitions: Vec<String>,
    pub forms: Vec<WordForm>,
    pub examples: Vec<Example>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictionarySnapshot {
    pub session_id: String,
    pub revision: u64,
    pub query_generation: u64,
    pub query: String,
    pub mode: String,
    pub status: String,
    pub entry: Option<DictionaryEntry>,
    pub suggestions: Vec<Suggestion>,
    pub error: Option<String>,
    pub suggestion_error: Option<String>,
}

pub fn normalize_query(text: &str) -> Option<String> {
    let mut text = text.trim();
    // Strip paired quotes and trailing sentence punctuation, in either nesting order.
    for _ in 0..3 {
        text = text
            .trim_end_matches([
                '.', ',', '!', '?', ';', ':', '。', '，', '！', '？', '；', '：',
            ])
            .trim();
        for (left, right) in [('"', '"'), ('\'', '\''), ('“', '”'), ('‘', '’')] {
            if text.len() > left.len_utf8() + right.len_utf8()
                && text.starts_with(left)
                && text.ends_with(right)
            {
                text = &text[left.len_utf8()..text.len() - right.len_utf8()];
            }
        }
    }
    let words: Vec<_> = text.split_whitespace().collect();
    if words.is_empty() || words.len() > 5 || text.len() > 256 {
        return None;
    }
    for word in &words {
        let chars: Vec<_> = word.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            if c.is_ascii_alphabetic() {
                continue;
            }
            if !matches!(c, '-' | '\'' | '’')
                || i == 0
                || i + 1 == chars.len()
                || !chars[i - 1].is_ascii_alphabetic()
                || !chars[i + 1].is_ascii_alphabetic()
            {
                return None;
            }
        }
    }
    Some(words.join(" ").replace('’', "'"))
}

pub async fn json_response(request: reqwest::RequestBuilder) -> Result<Value, String> {
    json_response_with_timeout(request, Duration::from_secs(10)).await
}

async fn json_response_with_timeout(
    request: reqwest::RequestBuilder,
    timeout: Duration,
) -> Result<Value, String> {
    let mut response = request
        .timeout(timeout)
        .send()
        .await
        .map_err(|_| "网络请求失败或超时，请重试".to_owned())?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => "授权无效或已过期，请在设置中重新配置".to_owned(),
            429 => "请求过于频繁，请稍后重试".to_owned(),
            code => format!("服务请求失败（HTTP {code}）"),
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "读取响应失败或超时".to_owned())?
    {
        if bytes.len() + chunk.len() > 3 * 1024 * 1024 {
            return Err("服务响应过大".to_owned());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "服务返回了无效 JSON".to_owned())?;
    if !value.is_object() {
        return Err("服务响应格式异常".to_owned());
    }
    Ok(value)
}
pub fn parse_suggestions(value: &Value) -> Result<Vec<Suggestion>, String> {
    if value.pointer("/result/code").and_then(Value::as_i64) != Some(200) {
        return Err("联想查询失败".to_owned());
    }
    let Some(entries) = value.pointer("/data/entries").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    Ok(entries
        .iter()
        .filter_map(|v| {
            Some(Suggestion {
                word: v.get("entry")?.as_str()?.to_owned(),
                explanation: v
                    .get("explain")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .take(5)
        .collect())
}
pub async fn suggest(client: &Client, query: &str) -> Result<Vec<Suggestion>, String> {
    let value = json_response(client.get(query_url(
        "https://dict.youdao.com/suggest",
        &[
            ("q", query),
            ("le", "eng"),
            ("num", "5"),
            ("doctype", "json"),
        ],
    )))
    .await?;
    parse_suggestions(&value)
}
pub fn parse_entry(value: &Value, query: &str) -> Result<Option<DictionaryEntry>, String> {
    if value.get("input").and_then(Value::as_str).is_none() {
        return Err("词典响应格式异常".to_owned());
    }
    let Some(word) = value.pointer("/ec/word/0").filter(|v| v.is_object()) else {
        return Ok(None);
    };
    let definitions: Vec<String> = word
        .get("trs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|v| v.get("tr").and_then(Value::as_array).into_iter().flatten())
        .flat_map(|v| {
            v.pointer("/l/i")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .collect();
    if definitions.is_empty() {
        return Ok(None);
    }
    let phone = |name: &str| {
        word.get(name)
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .pointer("/simple/word/0")
                    .and_then(|v| v.get(name))
                    .and_then(Value::as_str)
            })
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let entry = DictionaryEntry {
        word: word
            .pointer("/return-phrase/l/i")
            .and_then(Value::as_str)
            .unwrap_or(query)
            .to_owned(),
        uk_phone: phone("ukphone"),
        us_phone: phone("usphone"),
        definitions,
        forms: word
            .get("wfs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| {
                Some(WordForm {
                    name: v.pointer("/wf/name")?.as_str()?.to_owned(),
                    value: v.pointer("/wf/value")?.as_str()?.to_owned(),
                })
            })
            .collect(),
        examples: value
            .pointer("/blng_sents_part/sentence-pair")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| {
                Some(Example {
                    text: v.get("sentence")?.as_str()?.to_owned(),
                    translation: v.get("sentence-translation")?.as_str()?.to_owned(),
                })
            })
            .take(3)
            .collect(),
    };
    Ok(Some(entry))
}
pub async fn lookup(client: &Client, query: &str) -> Result<Option<DictionaryEntry>, String> {
    let value = json_response(client.get(query_url(
        "https://dict.youdao.com/jsonapi",
        &[("q", query), ("jsonversion", "2"), ("client", "mobile")],
    )))
    .await?;
    parse_entry(&value, query)
}

pub(crate) fn query_url(base: &'static str, pairs: &[(&str, &str)]) -> url::Url {
    let mut url = url::Url::parse(base).expect("static service URL");
    url.query_pairs_mut().extend_pairs(pairs.iter().copied());
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    #[ignore = "Live Youdao smoke test; requires network and is run explicitly"]
    async fn live_youdao_lookup_and_suggestions() {
        let client = Client::builder()
            .user_agent(format!("Popper/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap();
        for query in ["account", "take off"] {
            let (entry, candidates) = tokio::join!(lookup(&client, query), suggest(&client, query));
            let entry = entry.unwrap().expect("known dictionary entry");
            assert!(!entry.definitions.is_empty());
            assert!(candidates
                .unwrap()
                .iter()
                .any(|candidate| candidate.word == query));
        }
        assert!(lookup(&client, "qzxvplmnotaword").await.unwrap().is_none());
    }
    #[test]
    fn routing() {
        for (input, output) in [
            (" account ", "account"),
            ("“take   off.”", "take off"),
            ("don't", "don't"),
            ("well-known", "well-known"),
            ("I am happy", "I am happy"),
        ] {
            assert_eq!(normalize_query(input).as_deref(), Some(output));
        }
        for input in [
            "",
            "hello, world",
            "abc2",
            "中文",
            "hello中文",
            "one two three four five six",
            "-word",
            "a--b",
        ] {
            assert_eq!(normalize_query(input), None, "{input}");
        }
    }
    #[test]
    fn dictionary_hit_requires_definitions() {
        assert!(parse_entry(
            &json!({"input":"x", "simple":{"word":[{"usphone":"x"}]}}),
            "x"
        )
        .unwrap()
        .is_none());
        assert!(parse_entry(&json!({"error":"bad"}), "x").is_err());
        let value = json!({"input":"cat", "ec":{"word":[{"trs":[{"tr":[{"l":{"i":["n. 猫"]}}]}]}]}, "simple":{"word":[{"usphone":"kat"}]}});
        let entry = parse_entry(&value, "cat").unwrap().unwrap();
        assert_eq!(entry.us_phone.as_deref(), Some("kat"));
        assert_eq!(entry.definitions, vec!["n. 猫"]);
    }
    #[test]
    fn suggestion_is_not_a_correction() {
        let suggestions = parse_suggestions(&json!({"result":{"code":200},"data":{"entries":[{"entry":"accoutrement","explain":"equipment"}]}})).unwrap();
        assert_eq!(suggestions[0].word, "accoutrement");
    }

    #[test]
    fn queries_are_encoded_without_changing_words() {
        let url = query_url(
            "https://dict.youdao.com/jsonapi",
            &[("q", "don't take off")],
        );
        assert_eq!(url.query_pairs().next().unwrap().1, "don't take off");
        assert!(!url.as_str().contains(' '));
    }

    #[tokio::test]
    async fn transport_handles_invalid_json_http_errors_and_timeout() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        for (status, body, delay) in [
            (200, "not-json", 0),
            (401, "secret must not appear", 0),
            (200, "{}", 150),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 2048];
                let _ = socket.read(&mut buffer).await;
                tokio::time::sleep(Duration::from_millis(delay)).await;
                let response = format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = socket.write_all(response.as_bytes()).await;
            });
            let error = json_response_with_timeout(
                Client::new().get(format!("http://{address}")),
                Duration::from_millis(100),
            )
            .await
            .unwrap_err();
            assert!(!error.contains("secret"));
            if status == 401 {
                assert!(error.contains("授权"));
            } else if delay == 0 {
                assert!(error.contains("JSON"));
            } else {
                assert!(error.contains("超时"));
            }
            server.abort();
        }
    }
}
