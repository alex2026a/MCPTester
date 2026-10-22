//! 历史成功调用 → 生成器种子。

use rusqlite::Connection;
use serde_json::Value;

use super::models::{TestMethod, TestSource};

/// 历史样例（参数已按 method 解包）。
#[derive(Debug, Clone)]
pub struct HistorySample {
    pub record_id: String,
    pub method: TestMethod,
    pub target: String,
    pub arguments: Value,
    pub source: TestSource,
}

/// 查询最近成功的手工调用，按 target 去重。
pub fn load_samples(
    conn: &Connection,
    server_id: &str,
    limit: usize,
) -> anyhow::Result<Vec<HistorySample>> {
    let mut stmt = conn.prepare(
        "SELECT id, method, target, params_json
         FROM call_records
         WHERE server_id = ?1 AND status = 'ok' AND origin = 'manual'
           AND method IN ('tools_call','resources_read','prompts_get')
         ORDER BY created_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![server_id, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;

    let mut seen = std::collections::HashSet::new();
    let mut samples = Vec::new();
    for row in rows {
        let (record_id, method, target, params_json) = row?;
        let params: Value = serde_json::from_str(&params_json).unwrap_or(Value::Null);
        let Some((method, arguments)) = unpack(&method, &params) else {
            continue;
        };
        let key = format!("{}|{}", method.as_str(), target);
        if !seen.insert(key) {
            continue;
        }
        samples.push(HistorySample {
            record_id,
            method,
            target,
            arguments,
            source: TestSource::History,
        });
        if samples.len() >= limit {
            break;
        }
    }
    Ok(samples)
}

fn unpack(method: &str, params: &Value) -> Option<(TestMethod, Value)> {
    match method {
        "tools_call" => Some((
            TestMethod::ToolsCall,
            params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default())),
        )),
        "resources_read" => Some((
            TestMethod::ResourcesRead,
            Value::Object(Default::default()),
        )),
        "prompts_get" => Some((
            TestMethod::PromptsGet,
            params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default())),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unpack_wrapped_params() {
        let (method, args) = unpack(
            "tools_call",
            &json!({"name": "echo", "arguments": {"text": "hi"}}),
        )
        .expect("解包失败");
        assert_eq!(method, TestMethod::ToolsCall);
        assert_eq!(args, json!({"text": "hi"}));

        let (method, args) = unpack("resources_read", &json!({"uri": "mock://hello"}))
            .expect("解包失败");
        assert_eq!(method, TestMethod::ResourcesRead);
        assert_eq!(args, json!({}));

        let (method, args) = unpack(
            "prompts_get",
            &json!({"name": "greet", "arguments": {"name": "a"}}),
        )
        .expect("解包失败");
        assert_eq!(method, TestMethod::PromptsGet);
        assert_eq!(args, json!({"name": "a"}));
    }
}
