use plist::{Dictionary, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs::OpenOptions;
use std::io::{self, Cursor, Write};
use std::process::{Command, Stdio};

type Snapshot = BTreeMap<String, Dictionary>;

const GLOBAL_DOMAIN: &str = "NSGlobalDomain";
const IGNORED_DOMAINS: &[&str] = &["ContextStoreAgent", "com.apple.xpc.activity2"];
const IGNORED_DOMAIN_PREFIXES: &[&str] = &["com.apple.CloudSubscriptionFeatures.geoCache"];
const IGNORED_KEYS: &[&str] = &[
    "ActivityBaseDates",
    "AppleSelectedInputSources",
    "CloudKitAccountInfoCache",
    "AccountInfoValidationCounter",
    "RTDefaultsSafetyCacheActiveSessionZoneCKSyncEngineMetadata",
    "RTDefaultsSafetyCacheActiveSessionZoneCKSyncEngineMetadataDate",
];
const IGNORED_KEY_SUBSTRINGS: &[&str] = &["_DKThrottledActivityLast_"];

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Defaults(Options),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    append: Option<String>,
    edit: Option<EditorChoice>,
    message: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum EditorChoice {
    Environment,
    Command(String),
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match parse_mode(&std::env::args().skip(1).collect::<Vec<_>>())? {
        None => {
            print_help();
            Ok(())
        }
        Some(Mode::Defaults(options)) => run_defaults_mode(&options),
    }
}

fn parse_mode(args: &[String]) -> Result<Option<Mode>, Box<dyn Error>> {
    let mut options = Options::default();
    let mut index = 0;

    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "-h" | "--help" => return Ok(None),
            "-a" | "--append" | "--out" => {
                index += 1;
                options.append = Some(
                    args.get(index)
                        .ok_or_else(|| format!("{argument} requires a filename"))?
                        .clone(),
                );
            }
            "-e" | "--edit" => {
                options.edit = Some(parse_optional_editor(args, &mut index));
            }
            "-m" | "--message" | "--comment" => {
                index += 1;
                options.message = Some(
                    args.get(index)
                        .ok_or_else(|| format!("{argument} requires a message"))?
                        .clone(),
                );
            }
            other if other.starts_with("--append=") => {
                options.append = Some(option_value(other, "--append")?.to_string());
            }
            other if other.starts_with("--out=") => {
                options.append = Some(option_value(other, "--out")?.to_string());
            }
            other if other.starts_with("--message=") => {
                options.message = Some(option_value(other, "--message")?.to_string());
            }
            other if other.starts_with("--comment=") => {
                options.message = Some(option_value(other, "--comment")?.to_string());
            }
            other if other.starts_with("--edit=") => {
                options.edit = Some(EditorChoice::Command(
                    option_value(other, "--edit")?.to_string(),
                ));
            }
            other if other.starts_with("-a") && other.len() > 2 => {
                options.append = Some(other[2..].to_string());
            }
            other if other.starts_with("-e") && other.len() > 2 => {
                options.edit = Some(EditorChoice::Command(other[2..].to_string()));
            }
            other if other.starts_with("-m") && other.len() > 2 => {
                options.message = Some(other[2..].to_string());
            }
            other => return Err(format!("unrecognized argument: {other}").into()),
        }

        index += 1;
    }

    Ok(Some(Mode::Defaults(options)))
}

fn parse_optional_editor(args: &[String], index: &mut usize) -> EditorChoice {
    let next_index = *index + 1;
    if let Some(editor) = args
        .get(next_index)
        .filter(|argument| !argument.starts_with('-'))
    {
        *index = next_index;
        EditorChoice::Command(editor.clone())
    } else {
        EditorChoice::Environment
    }
}

fn option_value<'a>(argument: &'a str, option: &str) -> Result<&'a str, Box<dyn Error>> {
    let value = argument
        .strip_prefix(option)
        .and_then(|rest| rest.strip_prefix('='))
        .ok_or_else(|| format!("invalid option form: {argument}"))?;
    if value.is_empty() {
        return Err(format!("{option} requires a value").into());
    }
    Ok(value)
}

fn run_defaults_mode(options: &Options) -> Result<(), Box<dyn Error>> {
    eprintln!("Dumping current defaults...");
    let (before, before_warnings) = snapshot_defaults()?;
    print_warnings(&before_warnings);

    prompt_for_change("Make your settings change now, then press Enter to dump defaults again.")?;

    eprintln!("Dumping updated defaults...");
    let (after, after_warnings) = snapshot_defaults()?;
    print_warnings(&after_warnings);

    let commands = diff_defaults_snapshots(&before, &after);
    if commands.is_empty() {
        eprintln!("No defaults changes detected.");
    } else {
        eprintln!("Generated defaults commands.");
    }

    let output = command_output(&commands, options.message.as_deref())?;
    emit_output(&output, options)?;

    Ok(())
}

fn print_help() {
    println!("defaults-differ");
    println!();
    println!("Usage");
    println!("  defaults-differ");
    println!("    [-a FILENAME | --append=FILENAME ]  Append defaults commands to a file");
    println!("    [-m MESSAGE  | --message=MESSAGE ]  Message as comment preceding commands");
    println!("    [-e [EDITOR] | --edit [EDITOR]   ]  Open $EDITOR or specified text editor");
}

fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
}

fn prompt_for_change(message: &str) -> Result<(), Box<dyn Error>> {
    eprintln!("{message}");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(())
}

fn command_output(commands: &[String], message: Option<&str>) -> Result<String, Box<dyn Error>> {
    if commands.is_empty() {
        return Ok(String::new());
    }

    let mut lines = Vec::new();
    if let Some(message) = message {
        lines.push(format!("# {}: {message}", current_date()?));
    }
    lines.extend(commands.iter().cloned());

    Ok(format!("{}\n", lines.join("\n")))
}

fn current_date() -> Result<String, Box<dyn Error>> {
    let output = Command::new("date").arg("+%F").output()?;
    if !output.status.success() {
        return Err(format!(
            "`date +%F` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn emit_output(output: &str, options: &Options) -> Result<(), Box<dyn Error>> {
    if let Some(filename) = &options.append {
        append_output(filename, output)?;
        if let Some(editor) = &options.edit {
            edit_file(filename, editor)?;
        }
    } else if let Some(editor) = &options.edit {
        edit_stdin(output, editor)?;
    } else {
        write_stdout(output)?;
    }

    Ok(())
}

fn write_stdout(output: &str) -> Result<(), Box<dyn Error>> {
    let mut stdout = io::stdout();
    stdout.write_all(output.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

fn append_output(filename: &str, output: &str) -> Result<(), Box<dyn Error>> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(filename)?;
    file.write_all(output.as_bytes())?;
    file.flush()?;
    Ok(())
}

fn editor(choice: &EditorChoice) -> Option<String> {
    match choice {
        EditorChoice::Environment => std::env::var("EDITOR").ok(),
        EditorChoice::Command(editor) => Some(editor.clone()),
    }
    .map(|editor| editor.trim().to_string())
    .filter(|editor| !editor.is_empty())
}

fn edit_file(filename: &str, editor_choice: &EditorChoice) -> Result<(), Box<dyn Error>> {
    let Some(editor) = editor(editor_choice) else {
        eprintln!("warning: --edit requested, but $EDITOR is unset");
        return Ok(());
    };

    let status = Command::new("sh")
        .arg("-c")
        .arg("exec $EDITOR \"$1\"")
        .arg("defaults-differ-editor")
        .arg(filename)
        .env("EDITOR", editor)
        .status()?;
    if !status.success() {
        return Err(format!("editor exited with {status}").into());
    }

    Ok(())
}

fn edit_stdin(output: &str, editor_choice: &EditorChoice) -> Result<(), Box<dyn Error>> {
    let Some(editor) = editor(editor_choice) else {
        eprintln!("warning: --edit requested, but $EDITOR is unset");
        write_stdout(output)?;
        return Ok(());
    };

    let mut child = Command::new("sh")
        .arg("-c")
        .arg("exec $EDITOR -")
        .env("EDITOR", editor)
        .stdin(Stdio::piped())
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "failed to open editor stdin"))?;
    stdin.write_all(output.as_bytes())?;
    drop(stdin);

    let status = child.wait()?;
    if !status.success() {
        return Err(format!("editor exited with {status}").into());
    }

    Ok(())
}

fn snapshot_defaults() -> Result<(Snapshot, Vec<String>), Box<dyn Error>> {
    let mut domains = list_domains()?;
    if !domains.iter().any(|domain| domain == GLOBAL_DOMAIN) {
        domains.push(GLOBAL_DOMAIN.to_string());
    }
    domains.sort();
    domains.dedup();

    let mut snapshot = Snapshot::new();
    let mut warnings = Vec::new();

    for domain in domains {
        match export_domain(&domain) {
            Ok(Some(defaults)) => {
                snapshot.insert(domain, defaults);
            }
            Ok(None) => {}
            Err(error) => {
                warnings.push(format!("skipped {domain}: {error}"));
            }
        }
    }

    Ok((snapshot, warnings))
}

fn list_domains() -> Result<Vec<String>, Box<dyn Error>> {
    let output = Command::new("defaults").arg("domains").output()?;
    if !output.status.success() {
        return Err(format!(
            "`defaults domains` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }

    let stdout = String::from_utf8(output.stdout)?;
    Ok(stdout
        .split(',')
        .map(str::trim)
        .filter(|domain| !domain.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn export_domain(domain: &str) -> Result<Option<Dictionary>, Box<dyn Error>> {
    let output = Command::new("defaults")
        .args(["export", domain, "-"])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("does not exist") {
            return Ok(None);
        }

        return Err(format!("`defaults export` failed: {}", stderr.trim()).into());
    }

    if output.stdout.is_empty() {
        return Ok(None);
    }

    let value = Value::from_reader(Cursor::new(output.stdout))?;
    match value {
        Value::Dictionary(defaults) => Ok(Some(defaults)),
        other => Err(format!(
            "expected defaults export to be a dictionary, got {}",
            value_kind(&other)
        )
        .into()),
    }
}

fn diff_defaults_snapshots(before: &Snapshot, after: &Snapshot) -> Vec<String> {
    let mut commands = Vec::new();
    let domains: BTreeSet<&String> = before.keys().chain(after.keys()).collect();

    for domain in domains {
        if ignore_domain(domain) {
            continue;
        }

        match (before.get(domain), after.get(domain)) {
            (Some(_), None) => {
                commands.push(format!("defaults delete {}", shell_quote(domain)));
            }
            (before_defaults, Some(after_defaults)) => {
                commands.extend(diff_defaults_domain(
                    domain,
                    before_defaults,
                    after_defaults,
                ));
            }
            (None, None) => {}
        }
    }

    commands
}

fn diff_defaults_domain(
    domain: &str,
    before: Option<&Dictionary>,
    after: &Dictionary,
) -> Vec<String> {
    let mut commands = Vec::new();
    let before = before
        .into_iter()
        .flat_map(|dict| dict.iter())
        .collect::<BTreeMap<_, _>>();
    let keys: BTreeSet<&String> = before.keys().copied().chain(after.keys()).collect();

    for key in keys {
        if ignore_key(domain, key, before.get(key).copied(), after.get(key)) {
            continue;
        }

        match (before.get(key), after.get(key)) {
            (Some(old_value), Some(new_value)) if *old_value == new_value => {}
            (_, Some(new_value)) => commands.push(write_defaults_command(domain, key, new_value)),
            (Some(_), None) => commands.push(format!(
                "defaults delete {} {}",
                shell_quote(domain),
                shell_quote(key)
            )),
            (None, None) => {}
        }
    }

    commands
}

fn write_defaults_command(domain: &str, key: &str, value: &Value) -> String {
    let mut parts = vec![
        "defaults".to_string(),
        "write".to_string(),
        shell_quote(domain),
        shell_quote(key),
    ];

    match value {
        Value::Boolean(value) => {
            parts.push("-bool".to_string());
            parts.push(value.to_string());
        }
        Value::Integer(value) => {
            parts.push("-int".to_string());
            parts.push(value.to_string());
        }
        Value::Real(value) => {
            parts.push("-float".to_string());
            parts.push(format_float(*value));
        }
        Value::String(value) => {
            parts.push("-string".to_string());
            parts.push(shell_quote(value));
        }
        Value::Data(bytes) => {
            parts.push("-data".to_string());
            parts.push(hex(bytes));
        }
        other => {
            parts.push(shell_quote(&openstep_value(other)));
        }
    }

    parts.join(" ")
}

fn ignore_domain(domain: &str) -> bool {
    IGNORED_DOMAINS.contains(&domain)
        || IGNORED_DOMAIN_PREFIXES
            .iter()
            .any(|prefix| domain.starts_with(prefix))
}

fn ignore_key(domain: &str, key: &str, before: Option<&Value>, after: Option<&Value>) -> bool {
    if ignore_domain(domain)
        || IGNORED_KEYS.contains(&key)
        || IGNORED_KEY_SUBSTRINGS
            .iter()
            .any(|substring| key.contains(substring))
        || ignore_domain_specific_key(domain, key)
    {
        return true;
    }

    matches!(
        (before, after),
        (Some(Value::String(old)), Some(Value::String(new)))
            if old != new && looks_like_timestamp_noise(key, old, new)
    )
}

fn ignore_domain_specific_key(domain: &str, key: &str) -> bool {
    (domain == "com.apple.controlcenter" && key.starts_with("NSStatusItem Preferred Position "))
        || (domain == "com.apple.DuetExpertCenter.MagicalMoments" && key == "lastPlayed")
        || (domain == "com.apple.DuetExpertCenter.AppPredictionExpert"
            && key.starts_with("ATXUpdatePredictionsLoggerCountsDictionary-"))
        || (domain == "com.apple.spaces" && key == "SpacesDisplayConfiguration")
}

fn looks_like_timestamp_noise(key: &str, before: &str, after: &str) -> bool {
    let lowercase_key = key.to_ascii_lowercase();
    (lowercase_key.ends_with("date") || lowercase_key.contains("timestamp"))
        && looks_like_iso8601_utc(before)
        && looks_like_iso8601_utc(after)
}

fn looks_like_iso8601_utc(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        })
}

fn openstep_value(value: &Value) -> String {
    match value {
        Value::Array(values) => {
            let values = values
                .iter()
                .map(openstep_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({values})")
        }
        Value::Dictionary(values) => {
            let entries = values
                .iter()
                .map(|(key, value)| {
                    format!("{} = {};", openstep_string(key), openstep_value(value))
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!("{{{entries}}}")
        }
        Value::Boolean(true) => "YES".to_string(),
        Value::Boolean(false) => "NO".to_string(),
        Value::Integer(value) => value.to_string(),
        Value::Real(value) => format_float(*value),
        Value::String(value) => openstep_string(value),
        Value::Data(bytes) => format!("<{}>", hex(bytes)),
        Value::Date(value) => openstep_string(&value.to_xml_format()),
        other => openstep_string(&format!("<unsupported {}>", value_kind(other))),
    }
}

fn openstep_string(value: &str) -> String {
    let escaped = value
        .chars()
        .flat_map(|character| match character {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            other => vec![other],
        })
        .collect::<String>();
    format!("\"{escaped}\"")
}

fn format_float(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-_./:@%+=".contains(character))
    {
        return value.to_string();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Array(_) => "array",
        Value::Dictionary(_) => "dictionary",
        Value::Boolean(_) => "boolean",
        Value::Data(_) => "data",
        Value::Date(_) => "date",
        Value::Integer(_) => "integer",
        Value::Real(_) => "real",
        Value::String(_) => "string",
        Value::Uid(_) => "uid",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modes() {
        assert_eq!(
            parse_mode(&[]).unwrap(),
            Some(Mode::Defaults(Options::default()))
        );
        assert!(parse_mode(&["--help".to_string()]).unwrap().is_none());
        assert!(parse_mode(&["plist".to_string()]).is_err());
    }

    #[test]
    fn parses_output_options_and_aliases() {
        assert_eq!(
            parse_mode(&[
                "--out=~/.defaults".to_string(),
                "--edit".to_string(),
                "--comment".to_string(),
                "Trackpad".to_string()
            ])
            .unwrap(),
            Some(Mode::Defaults(Options {
                append: Some("~/.defaults".to_string()),
                edit: Some(EditorChoice::Environment),
                message: Some("Trackpad".to_string()),
            }))
        );

        assert_eq!(
            parse_mode(&[
                "-a".to_string(),
                "defaults.sh".to_string(),
                "-mInitial setup".to_string()
            ])
            .unwrap(),
            Some(Mode::Defaults(Options {
                append: Some("defaults.sh".to_string()),
                edit: None,
                message: Some("Initial setup".to_string()),
            }))
        );

        assert_eq!(
            parse_mode(&["-e".to_string(), "vim".to_string()]).unwrap(),
            Some(Mode::Defaults(Options {
                append: None,
                edit: Some(EditorChoice::Command("vim".to_string())),
                message: None,
            }))
        );

        assert_eq!(
            parse_mode(&[
                "--edit".to_string(),
                "-a".to_string(),
                "defaults.sh".to_string()
            ])
            .unwrap(),
            Some(Mode::Defaults(Options {
                append: Some("defaults.sh".to_string()),
                edit: Some(EditorChoice::Environment),
                message: None,
            }))
        );
    }

    #[test]
    fn renders_no_message_without_commands() {
        assert_eq!(command_output(&[], Some("No diff")).unwrap(), "");
    }

    #[test]
    fn quotes_shell_arguments_only_when_needed() {
        assert_eq!(shell_quote("com.apple.finder"), "com.apple.finder");
        assert_eq!(shell_quote("key with spaces"), "'key with spaces'");
        assert_eq!(shell_quote("can't"), "'can'\\''t'");
    }

    #[test]
    fn renders_scalar_write_commands() {
        assert_eq!(
            write_defaults_command("com.example", "Enabled", &Value::Boolean(true)),
            "defaults write com.example Enabled -bool true"
        );
        assert_eq!(
            write_defaults_command("com.example", "Name", &Value::String("A B".to_string())),
            "defaults write com.example Name -string 'A B'"
        );
    }

    #[test]
    fn renders_nested_values_as_openstep_literals() {
        let mut dictionary = Dictionary::new();
        dictionary.insert(
            "letters".to_string(),
            Value::Array(vec![Value::String("a".to_string())]),
        );

        assert_eq!(
            openstep_value(&Value::Dictionary(dictionary)),
            "{\"letters\" = (\"a\");}"
        );
    }

    #[test]
    fn ignores_known_noise_domains_and_keys() {
        assert!(ignore_domain("ContextStoreAgent"));
        assert!(ignore_domain(
            "com.apple.CloudSubscriptionFeatures.geoCache"
        ));
        assert!(ignore_key(
            "com.apple.anything",
            "CloudKitAccountInfoCache",
            None,
            None
        ));
        assert!(ignore_key(
            "com.apple.controlcenter",
            "NSStatusItem Preferred Position Bluetooth",
            None,
            None
        ));
    }

    #[test]
    fn ignores_timestamp_only_string_updates_for_date_keys() {
        assert!(looks_like_timestamp_noise(
            "LastUpdatedDate",
            "2026-04-25T17:12:44Z",
            "2026-04-25T17:13:40Z"
        ));
        assert!(!looks_like_timestamp_noise(
            "Title",
            "2026-04-25T17:12:44Z",
            "2026-04-25T17:13:40Z"
        ));
    }
}
