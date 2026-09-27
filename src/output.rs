use crate::http::Problem;
use comfy_table::{ContentArrangement, Table, presets::UTF8_HORIZONTAL_ONLY};
use serde_json::{Value, json};
use std::io::IsTerminal;

const COLUMNS: [&str; 10] = ["id", "name", "title", "filename", "status", "uri", "kind", "at", "by", "updated_at"];

#[derive(Clone, Copy)]
pub struct Output {
    pub json: bool,
}

impl Output {
    pub fn new(json: bool) -> Self {
        Output { json: json || !std::io::stdout().is_terminal() }
    }

    pub fn success(&self, data: &Value, site: Option<&str>, pick: Option<&str>) {
        let data = match pick {
            Some(path) => select(data, path).cloned().unwrap_or(Value::Null),
            None => data.clone(),
        };
        if self.json {
            let mut envelope = json!({ "ok": true, "data": data });
            if let Some(site) = site {
                envelope["site"] = json!(site);
            }
            println!("{envelope}");
        } else if pick.is_some() {
            match &data {
                Value::String(text) => println!("{text}"),
                other => println!("{}", serde_json::to_string_pretty(other).unwrap_or_default()),
            }
        } else {
            human(&data);
        }
    }

    pub fn failure(&self, error: &anyhow::Error) {
        if self.json {
            let body = match error.downcast_ref::<Problem>() {
                Some(problem) => problem.to_json(),
                None => json!({ "code": "error", "message": error.to_string() }),
            };
            println!("{}", json!({ "ok": false, "error": body }));
        } else {
            eprintln!("nibble: {error:#}");
        }
    }
}

pub fn select<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').filter(|part| !part.is_empty()).try_fold(value, |current, part| match part.parse::<usize>() {
        Ok(index) => current.get(index),
        Err(_) => current.get(part),
    })
}

fn human(data: &Value) {
    let Some(object) = data.as_object() else {
        println!("{}", serde_json::to_string_pretty(data).unwrap_or_default());
        return;
    };
    let list = object.iter().find(|(_, value)| value.as_array().is_some_and(|rows| rows.first().is_some_and(Value::is_object)));
    match list {
        Some((name, rows)) => {
            let rows = rows.as_array().unwrap();
            let keys: Vec<&str> = COLUMNS.iter().copied().filter(|key| rows.iter().any(|row| row.get(*key).is_some())).collect();
            let mut table = Table::new();
            table.load_style(UTF8_HORIZONTAL_ONLY).set_header(keys.clone());
            if table.width().is_some_and(|width| width >= 60) {
                table.set_content_arrangement(ContentArrangement::Dynamic);
            }
            for row in rows {
                table.add_row(keys.iter().map(|key| cell(row.get(*key))));
            }
            println!("{table}");
            if let Some(page) = object.get("page") {
                let total = page.get("total").and_then(Value::as_u64).unwrap_or(rows.len() as u64);
                println!("{} of {total} {name}", rows.len());
            }
        }
        None => println!("{}", serde_json::to_string_pretty(data).unwrap_or_default()),
    }
}

fn cell(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_walks_objects_and_arrays() {
        let data = json!({ "entries": [ { "title": "First" } ], "page": { "total": 3 } });
        assert_eq!(select(&data, "entries.0.title"), Some(&json!("First")));
        assert_eq!(select(&data, "page.total"), Some(&json!(3)));
        assert_eq!(select(&data, "entries.5.title"), None);
    }
}
