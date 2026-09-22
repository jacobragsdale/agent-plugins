fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|command| command == "stage") {
        match parse_stage_args(&args[1..]) {
            Ok(stage_args) => stage(&stage_args),
            Err(()) => usage(),
        }
    }
    match parse_args(args) {
        Ok(Arguments {
            input,
            json,
            secrets,
        }) => finish(
            if input.starts_with("https://") {
                agent_plugins_lib::validate_source_locator(&input)
            } else {
                agent_plugins_lib::validate_source(&input).and_then(|report| {
                    if secrets {
                        with_secrets(report, &input)
                    } else {
                        Ok(report)
                    }
                })
            },
            json,
        ),
        Err(()) => usage(),
    }
}

struct Arguments {
    input: String,
    json: bool,
    secrets: bool,
}

fn parse_args(args: Vec<String>) -> Result<Arguments, ()> {
    let mut input = None;
    let mut json = false;
    let mut secrets = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "--secrets" => secrets = true,
            flag if flag.starts_with("--") => return Err(()),
            _ if input.is_some() => return Err(()),
            _ => input = Some(arg),
        }
    }
    Ok(Arguments {
        input: input.ok_or(())?,
        json,
        secrets,
    })
}

/// Files that look like credentials fail validation, so the server refuses them on upload.
fn with_secrets(
    mut report: agent_plugins_lib::SourceValidationReport,
    input: &str,
) -> Result<agent_plugins_lib::SourceValidationReport, String> {
    for finding in agent_plugins_lib::staging::scan_for_secrets(std::path::Path::new(input))? {
        report
            .errors
            .push(agent_plugins_lib::SourceValidationError {
                path: finding.path,
                message: format!(
                    "Looks like a credential ({}); remove it before publishing.",
                    finding.reason
                ),
            });
    }
    Ok(report)
}

struct StageArguments {
    namespace: String,
    package_id: String,
    name: Option<String>,
    description: Option<String>,
    input: String,
    output: String,
}

fn parse_stage_args(args: &[String]) -> Result<StageArguments, ()> {
    let mut options = std::collections::BTreeMap::new();
    let mut positional = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--namespace" | "--package-id" | "--name" | "--description" => {
                options.insert(arg.as_str(), iter.next().ok_or(())?.clone());
            }
            flag if flag.starts_with("--") => return Err(()),
            _ => positional.push(arg.clone()),
        }
    }
    let [input, output] = <[String; 2]>::try_from(positional).map_err(|_| ())?;
    Ok(StageArguments {
        namespace: options.remove("--namespace").ok_or(())?,
        package_id: options.remove("--package-id").ok_or(())?,
        name: options.remove("--name"),
        description: options.remove("--description"),
        input,
        output,
    })
}

/// Wraps a skill, a folder of skills, an MCP document, or a source tree into a one-package
/// source zip at `output`, and prints `{ packageId, fileCount, totalBytes }` or `{ fatal }`.
fn stage(args: &StageArguments) -> ! {
    let staging = std::env::temp_dir().join(format!(
        "validate-source-stage-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    ));
    let request = agent_plugins_lib::staging::StageRequest {
        namespace: &args.namespace,
        package_id: Some(&args.package_id),
        name: args.name.as_deref(),
        description: args.description.as_deref(),
    };
    let result = agent_plugins_lib::staging::stage_tree(
        std::path::Path::new(&args.input),
        &request,
        &staging,
    )
    .and_then(|staged| {
        let archive = agent_plugins_lib::staging::zip_tree(&staged.root)?;
        std::fs::write(&args.output, archive)
            .map_err(|error| format!("Could not write {}: {error}", args.output))?;
        Ok(staged)
    });
    let _ = std::fs::remove_dir_all(&staging);
    match result {
        Ok(staged) => {
            println!(
                "{{\"packageId\":{},\"fileCount\":{},\"totalBytes\":{}}}",
                json_string(&staged.package_id),
                staged.file_count,
                staged.total_bytes
            );
            std::process::exit(0);
        }
        Err(error) => {
            println!("{}", fatal_json(&error));
            std::process::exit(1);
        }
    }
}

fn finish(result: Result<agent_plugins_lib::SourceValidationReport, String>, json: bool) {
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
fn report_json(report: &agent_plugins_lib::SourceValidationReport) -> String {
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
    eprintln!("usage: validate-source [--json] [--secrets] PATH-OR-HTTPS-URL");
    eprintln!("       validate-source stage --namespace NS --package-id ID [--name TEXT] [--description TEXT] INPUT OUT.zip");
    std::process::exit(2);
}
