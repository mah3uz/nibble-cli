use crate::http::Problem;
use crate::style::{BAD, DIM, HEADING, NAME, STRONG, paint};
use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table, presets::UTF8_HORIZONTAL_ONLY};
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
        // A picked value is for a script to capture, so it is never wrapped in the envelope.
        if let Some(path) = pick {
            match select(data, path) {
                Some(Value::String(text)) => println!("{text}"),
                Some(other) => {
                    println!("{}", if self.json { other.to_string() } else { serde_json::to_string_pretty(other).unwrap_or_default() })
                }
                None => println!("null"),
            }
        } else if self.json {
            let mut envelope = json!({ "ok": true, "data": data });
            if let Some(site) = site {
                envelope["site"] = json!(site);
            }
            println!("{envelope}");
        } else {
            anstream::print!("{}", human(data));
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
            anstream::eprintln!("{}", describe_error(error));
        }
    }
}

pub fn select<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').filter(|part| !part.is_empty()).try_fold(value, |current, part| match part.parse::<usize>() {
        Ok(index) => current.get(index),
        Err(_) => current.get(part),
    })
}

fn describe_error(error: &anyhow::Error) -> String {
    match error.downcast_ref::<Problem>() {
        Some(problem) => {
            let mut text = format!("{} {} {}", paint(BAD, "error"), problem.detail, paint(DIM, format!("({})", problem.code)));
            if let Some(hint) = &problem.hint {
                text.push_str(&format!("\n{} {hint}", paint(NAME, " hint")));
            }
            text
        }
        None => format!("{} {error:#}", paint(BAD, "error")),
    }
}

fn human(data: &Value) -> String {
    let Some(object) = data.as_object() else {
        let mut text = String::new();
        tree(data, 0, &mut text);
        return text;
    };
    if object.contains_key("page")
        && let Some((name, _)) = object.iter().find(|(_, value)| value.as_array().is_some_and(Vec::is_empty))
    {
        return paint(DIM, format!("No {name}.\n"));
    }
    let list = object.iter().find(|(_, value)| value.as_array().is_some_and(|rows| rows.first().is_some_and(Value::is_object)));
    let Some((name, rows)) = list else {
        let mut text = String::new();
        tree(data, 0, &mut text);
        return text;
    };
    let rows = rows.as_array().unwrap();
    let keys: Vec<&str> = COLUMNS.iter().copied().filter(|key| rows.iter().any(|row| row.get(*key).is_some())).collect();
    let mut table = Table::new();
    table.load_style(UTF8_HORIZONTAL_ONLY);
    if !crate::style::colour_on_stdout() {
        table.force_no_tty();
    }
    table.set_header(keys.iter().map(|key| Cell::new(key).add_attribute(Attribute::Bold).fg(Color::DarkMagenta)));
    if table.width().is_some_and(|width| width >= 60) {
        table.set_content_arrangement(ContentArrangement::Dynamic);
    }
    for row in rows {
        table.add_row(keys.iter().map(|key| cell(row.get(*key))));
    }
    let mut text = format!("{table}\n");
    if let Some(page) = object.get("page") {
        let total = page.get("total").and_then(Value::as_u64).unwrap_or(rows.len() as u64);
        text.push_str(&paint(DIM, format!("{} of {total} {name}\n", rows.len())));
    }
    text
}

fn nested(value: &Value) -> bool {
    match value {
        Value::Object(map) => !map.is_empty(),
        Value::Array(items) => items.iter().any(|item| item.is_object() || item.is_array()),
        _ => false,
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => paint(DIM, "—"),
        Value::String(text) => text.clone(),
        Value::Array(items) if items.is_empty() => paint(DIM, "none"),
        Value::Array(items) => items.iter().map(scalar).collect::<Vec<_>>().join(", "),
        Value::Object(_) => paint(DIM, "none"),
        other => other.to_string(),
    }
}

fn tree(value: &Value, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match value {
        Value::Object(map) => {
            let width = map.iter().filter(|(_, value)| !nested(value)).map(|(key, _)| key.chars().count()).max().unwrap_or(0);
            for (key, value) in map {
                if nested(value) {
                    out.push_str(&format!("{pad}{}\n", paint(if indent == 0 { HEADING } else { STRONG }, key)));
                    tree(value, indent + 2, out);
                } else {
                    out.push_str(&format!("{pad}{}  {}\n", paint(DIM, format!("{key:<width$}")), scalar(value)));
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if nested(item) {
                    out.push_str(&format!("{pad}{}\n", paint(DIM, "-")));
                    tree(item, indent + 2, out);
                } else {
                    out.push_str(&format!("{pad}{} {}\n", paint(DIM, "-"), scalar(item)));
                }
            }
        }
        other => out.push_str(&format!("{pad}{}\n", scalar(other))),
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

    fn plain(text: &str) -> String {
        anstream::adapter::strip_str(text).to_string()
    }

    #[test]
    fn a_record_reads_as_aligned_fields_not_json() {
        let data = json!({ "person": { "name": "Ada", "email": "ada@example.com" }, "access": "draft", "can": ["read", "write"] });
        assert_eq!(plain(&human(&data)), "person\n  name   Ada\n  email  ada@example.com\naccess  draft\ncan     read, write\n");
    }

    #[test]
    fn an_error_from_the_site_keeps_its_hint_on_its_own_line_aligned_under_it() {
        let problem = Problem::from_body(403, &json!({ "code": "forbidden", "detail": "You can't publish.", "hint": "Ask an editor." }));
        assert_eq!(plain(&describe_error(&problem.into())), "error You can't publish. (forbidden)\n hint Ask an editor.");
    }

    #[test]
    fn an_empty_page_says_so_rather_than_printing_its_paging() {
        let data = json!({ "entries": [], "page": { "total": 0 } });
        assert_eq!(plain(&human(&data)), "No entries.\n");
    }

    #[test]
    fn pick_walks_objects_and_arrays() {
        let data = json!({ "entries": [ { "title": "First" } ], "page": { "total": 3 } });
        assert_eq!(select(&data, "entries.0.title"), Some(&json!("First")));
        assert_eq!(select(&data, "page.total"), Some(&json!(3)));
        assert_eq!(select(&data, "entries.5.title"), None);
    }
}
