fn main() {
    match parse_args(std::env::args().skip(1).collect()) {
        Ok(Arguments { input, json }) => finish(
            if input.starts_with("https://") {
                skill_manager_lib::validate_source_locator(&input)
            } else {
                skill_manager_lib::validate_source(&input)
            },
            json,
        ),
        Err(()) => usage(),
    }
}

struct Arguments {
    input: String,
    json: bool,
}

fn parse_args(args: Vec<String>) -> Result<Arguments, ()> {
    match args.as_slice() {
        [input] => Ok(Arguments {
            input: input.clone(),
            json: false,
        }),
        [flag, input] | [input, flag] if flag == "--json" => Ok(Arguments {
            input: input.clone(),
            json: true,
        }),
        _ => Err(()),
    }
}

fn finish(result: Result<skill_manager_lib::SourceValidationReport, String>, json: bool) {
    match result {
        Ok(report) if json => {
            println!("{}", report_json(&report));
        }
        Ok(report) => {
            println!(
                "{}: {} valid install(s), {} catalog error(s)",
                report.source_id,
                report.valid_installs,
                report.errors.len()
            );
            for error in report.errors {
                println!("{}: {}", error.path, error.message);
            }
        }
        Err(error) if json => {
            println!("{}", fatal_json(&error));
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

/// The marketplace server parses this document, so its shape is a contract:
/// `sourceId`, `validInstalls`, and `errors[]` of `{ path, message }`.
fn report_json(report: &skill_manager_lib::SourceValidationReport) -> String {
    let errors = report
        .errors
        .iter()
        .map(|error| {
            format!(
                "{{\"path\":{},\"message\":{}}}",
                json_string(&error.path),
                json_string(&error.message)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"sourceId\":{},\"validInstalls\":{},\"errors\":[{}]}}",
        json_string(&report.source_id),
        report.valid_installs,
        errors
    )
}

fn fatal_json(message: &str) -> String {
    format!("{{\"fatal\":{}}}", json_string(message))
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn usage() -> ! {
    eprintln!("usage: validate-source [--json] PATH-OR-HTTPS-URL");
    std::process::exit(2);
}
