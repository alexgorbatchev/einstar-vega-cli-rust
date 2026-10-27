use cli_command_kit::{Mode, display_message};
use einstar_vega::{DeviceInfo, Value};
use std::io::{self, Write};
use tabled::{builder::Builder, settings::Style};

/// Escapes terminal controls without changing identities used for requests.
pub fn safe(text: &str) -> String {
    display_message(text)
        .chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

/// Prints one returned error with the selected output mode and safe text.
pub fn report_error(
    output: &mut impl Write,
    mode: Mode,
    error: &impl std::fmt::Display,
) -> io::Result<()> {
    let prefix = if mode == Mode::Agent {
        "ERR:"
    } else {
        "[ERROR]"
    };
    writeln!(output, "{prefix} {}", safe(&error.to_string()))
}

/// Formats arbitrary CBOR values without coercing binary/tagged metadata.
pub fn value(value: &Value) -> String {
    match value {
        Value::Text(text) => text.clone(),
        Value::Integer(number) => number.to_string(),
        Value::Float(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".into(),
        value => format!("{value:?}"),
    }
}

pub fn table(
    output: &mut impl Write,
    mode: Mode,
    headings: &[&str],
    rows: Vec<Vec<String>>,
) -> io::Result<()> {
    let rows = rows
        .into_iter()
        .map(|row| row.into_iter().map(|cell| safe(&cell)).collect::<Vec<_>>());
    if mode == Mode::Agent {
        writeln!(output, "{}", headings.join("\t"))?;
        for row in rows {
            writeln!(output, "{}", row.join("\t"))?;
        }
    } else {
        let mut builder = Builder::default();
        builder.push_record(headings.iter().copied());
        for row in rows {
            builder.push_record(row);
        }
        writeln!(output, "{}", builder.build().with(Style::ascii()))?;
    }
    Ok(())
}

pub fn device(output: &mut impl Write, mode: Mode, info: &DeviceInfo) -> io::Result<()> {
    table(
        output,
        mode,
        &["KEY", "VALUE"],
        info.iter()
            .map(|(key, v)| vec![key.clone(), value(v)])
            .collect(),
    )
}

pub fn size(bytes: Option<u64>) -> String {
    match bytes {
        None => "unknown".into(),
        Some(bytes) if bytes >= 1_048_576 => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
        Some(bytes) if bytes >= 1024 => format!("{:.1} KB", bytes as f64 / 1024.0),
        Some(bytes) => format!("{bytes} B"),
    }
}
